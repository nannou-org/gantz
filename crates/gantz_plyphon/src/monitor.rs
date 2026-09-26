//! Writing sampled dsp values back into a monitor node's ring-buffer state.
//!
//! A `~scopeout` monitor node holds its recent samples as one plain Steel
//! list per channel inside an outer [`SteelVal::ListV`]. That is the
//! list-of-lists shape `plot` renders as stacked per-channel sub-plots. Each
//! frame the audio driver drains its `ScopeOut` scope stream and calls
//! [`push_ring`] with the interleaved samples. It deinterleaves them and caps
//! each channel's ring at the node's configured length. The node's control
//! `expr` surfaces this state on a trigger push and derives its channel-count
//! output from the outer list's length.

use gantz_core::node::state;
use gantz_core::steel::SteelVal;
use gantz_core::steel::steel_vm::engine::Engine;

/// Deinterleave the `channels`-wide `values` into the per-channel ring-buffer
/// lists at the node `path` in VM state, dropping the oldest samples so each
/// ring holds at most `size`. A `size` of 0 is treated as 1, so a ring always
/// keeps at least the latest sample. A `channels` of 0 is treated as 1.
///
/// The state is an outer [`SteelVal::ListV`] holding one flat numeric ring
/// list per channel, seeded empty in the node's `register`. The outer list
/// takes the width of the incoming stream, so a width change after a respawn
/// reshapes it. Prior rings are reused where their channel still exists. A
/// non-list or absent value is treated as empty. So is a flat single-ring
/// list of numbers.
///
/// Each ring is rebuilt in a single `collect` rather than element by element.
/// Steel's list is an unrolled persistent list whose `push_back` is O(n).
/// Appending a whole frame's samples one at a time is therefore
/// O(frame x ring) on the main thread every frame. When the frame alone
/// fills a ring, the prior ring is dropped without being read.
///
/// Any trailing partial frame in `values` is dropped. plyphon streams whole
/// frames, so this is normally a no-op, but a misbehaving producer must not
/// permanently scramble the deinterleave.
pub fn push_ring(vm: &mut Engine, path: &[usize], values: &[f32], size: usize, channels: usize) {
    let size = size.max(1);
    let channels = channels.max(1);
    let values = &values[..values.len() - (values.len() % channels)];
    let frames = values.len() / channels;
    let sample = |&v: &f32| SteelVal::NumV(v as f64);

    // The prior per-channel rings, reused where the frame does not fill a ring
    // on its own. Non-list elements, such as the numbers of a flat ring, and
    // rings past `channels` contribute nothing.
    let old: Vec<SteelVal> = match state::extract_value(vm, path) {
        Ok(Some(SteelVal::ListV(rings))) => rings.iter().cloned().collect(),
        _ => Vec::new(),
    };

    let rings = (0..channels)
        .map(|c| {
            // Channel `c`'s samples within the interleaved stream.
            let ch = values.iter().skip(c).step_by(channels);
            if frames >= size {
                // Fast path. This frame alone fills the ring, so keep its last
                // `size` samples and drop the prior ring unread.
                SteelVal::ListV(ch.skip(frames - size).map(sample).collect())
            } else {
                // Otherwise keep the tail of the old ring so it plus the frame
                // totals `size`.
                match old.get(c) {
                    Some(SteelVal::ListV(old_ring)) => {
                        let keep = size - frames;
                        let skip = old_ring.len().saturating_sub(keep);
                        SteelVal::ListV(
                            old_ring
                                .iter()
                                .cloned()
                                .skip(skip)
                                .chain(ch.map(sample))
                                .collect(),
                        )
                    }
                    _ => SteelVal::ListV(ch.map(sample).collect()),
                }
            }
        })
        .collect();
    let _ = state::update_value(vm, path, SteelVal::ListV(rings));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::steel::steel_vm::engine::Engine;

    /// A `(values, size, channels)` batch for [`push_ring`].
    type Push = (&'static [f32], usize, usize);

    /// Each case seeds its own node path, pushes its batches in order, then
    /// reads back the per-channel rings. An empty seed is the empty outer list
    /// that the node's `register` seeds.
    #[test]
    fn push_ring_keeps_capped_per_channel_rings() {
        let cases: &[(&str, &[f64], &[Push], &[&[f64]])] = &[
            // Only the newest `size` samples survive, oldest dropped first.
            (
                "fill past capacity",
                &[],
                &[(&[1.0, 2.0, 3.0], 4, 1), (&[4.0, 5.0], 4, 1)],
                &[&[2.0, 3.0, 4.0, 5.0]],
            ),
            // The fast path drops the prior ring rather than appending to it.
            (
                "a full frame replaces the ring",
                &[],
                &[(&[1.0, 2.0], 2, 1), (&[3.0, 4.0, 5.0], 2, 1)],
                &[&[4.0, 5.0]],
            ),
            (
                "a size of 0 keeps the latest sample",
                &[],
                &[(&[1.0, 2.0, 3.0], 0, 1)],
                &[&[3.0]],
            ),
            (
                "stereo lands in one ring per channel",
                &[],
                &[(&[1.0, -1.0, 2.0, -2.0, 3.0, -3.0], 4, 2)],
                &[&[1.0, 2.0, 3.0], &[-1.0, -2.0, -3.0]],
            ),
            (
                "each channel caps independently",
                &[],
                &[(&[1.0, -1.0, 2.0, -2.0], 2, 2), (&[3.0, -3.0], 2, 2)],
                &[&[2.0, 3.0], &[-2.0, -3.0]],
            ),
            // A width change, a respawn after a rewire, reshapes the outer
            // list. Surviving channels keep their ring tails and new channels
            // start fresh.
            (
                "narrowing to mono keeps channel 0",
                &[],
                &[(&[1.0, -1.0, 2.0, -2.0], 4, 2), (&[3.0], 4, 1)],
                &[&[1.0, 2.0, 3.0]],
            ),
            (
                "widening back to stereo restarts channel 1",
                &[],
                &[
                    (&[1.0, -1.0, 2.0, -2.0], 4, 2),
                    (&[3.0], 4, 1),
                    (&[4.0, -4.0], 4, 2),
                ],
                &[&[1.0, 2.0, 3.0, 4.0], &[-4.0]],
            ),
            // A partial frame must not scramble the deinterleave.
            (
                "a trailing partial frame is dropped",
                &[],
                &[(&[1.0, -1.0, 2.0], 4, 2)],
                &[&[1.0], &[-1.0]],
            ),
            (
                "a channels of 0 clamps to 1",
                &[],
                &[(&[1.0, 2.0], 4, 0)],
                &[&[1.0, 2.0]],
            ),
            // Its elements are numbers, not rings.
            (
                "a flat single-ring list reads as empty",
                &[1.0, 2.0],
                &[(&[3.0], 4, 1)],
                &[&[3.0]],
            ),
        ];
        let mut vm = Engine::new_base();
        vm.register_value(gantz_core::ROOT_STATE, SteelVal::empty_hashmap());
        for (ix, (label, seed, pushes, expected)) in cases.iter().enumerate() {
            let path = [ix];
            state::init_value_if_absent(&mut vm, &path, || {
                SteelVal::ListV(seed.iter().map(|&v| SteelVal::NumV(v)).collect())
            })
            .unwrap();
            for &(values, size, channels) in pushes.iter() {
                push_ring(&mut vm, &path, values, size, channels);
            }
            assert_eq!(ring_values(&mut vm, &path), *expected, "{label}");
        }
    }

    /// Read the per-channel rings at `path` back as `f64`s.
    fn ring_values(vm: &mut Engine, path: &[usize]) -> Vec<Vec<f64>> {
        match state::extract_value(vm, path) {
            Ok(Some(SteelVal::ListV(rings))) => rings
                .iter()
                .map(|ring| match ring {
                    SteelVal::ListV(ring) => ring
                        .iter()
                        .filter_map(|v| match v {
                            SteelVal::NumV(f) => Some(*f),
                            SteelVal::IntV(i) => Some(*i as f64),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}
