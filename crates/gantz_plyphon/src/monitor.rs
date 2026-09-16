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
/// Steel's list is an unrolled persistent list whose `push_back` is O(n), so
/// appending a whole frame's samples one at a time is O(frame x ring) on the
/// main thread every frame. When the frame alone fills a ring, the prior ring
/// is dropped without being read.
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

    fn test_vm() -> Engine {
        let mut vm = Engine::new_base();
        vm.register_value(gantz_core::ROOT_STATE, SteelVal::empty_hashmap());
        state::init_value_if_absent(&mut vm, &[0], || SteelVal::ListV(Default::default())).unwrap();
        vm
    }

    /// Seed an empty ring at `[0]`, then push mono samples in batches. The
    /// ring keeps only the most recent `size`, oldest dropped first.
    #[test]
    fn push_ring_caps_and_drops_oldest() {
        let mut vm = test_vm();

        // Fill past capacity in two batches. Only the last `size` survive.
        push_ring(&mut vm, &[0], &[1.0, 2.0, 3.0], 4, 1);
        push_ring(&mut vm, &[0], &[4.0, 5.0], 4, 1);

        let got = ring_values(&mut vm, &[0]);
        assert_eq!(
            got,
            vec![vec![2.0, 3.0, 4.0, 5.0]],
            "ring keeps the newest `size`"
        );
    }

    /// A frame at least as long as `size` replaces the ring with its own last
    /// `size` samples. The fast path drops the prior ring rather than
    /// appending to it.
    #[test]
    fn push_ring_full_frame_replaces() {
        let mut vm = test_vm();
        push_ring(&mut vm, &[0], &[1.0, 2.0], 2, 1);
        push_ring(&mut vm, &[0], &[3.0, 4.0, 5.0], 2, 1);
        assert_eq!(ring_values(&mut vm, &[0]), vec![vec![4.0, 5.0]]);
    }

    /// A `size` of 0 is clamped to 1, so each ring keeps the single latest
    /// sample.
    #[test]
    fn push_ring_size_zero_keeps_latest() {
        let mut vm = test_vm();
        push_ring(&mut vm, &[0], &[1.0, 2.0, 3.0], 0, 1);
        assert_eq!(ring_values(&mut vm, &[0]), vec![vec![3.0]]);
    }

    /// Interleaved stereo samples land in one ring per channel.
    #[test]
    fn push_ring_deinterleaves() {
        let mut vm = test_vm();
        push_ring(&mut vm, &[0], &[1.0, -1.0, 2.0, -2.0, 3.0, -3.0], 4, 2);
        assert_eq!(
            ring_values(&mut vm, &[0]),
            vec![vec![1.0, 2.0, 3.0], vec![-1.0, -2.0, -3.0]],
        );
    }

    /// Each channel's ring caps at `size` frames independently, oldest first.
    #[test]
    fn push_ring_caps_per_channel() {
        let mut vm = test_vm();
        push_ring(&mut vm, &[0], &[1.0, -1.0, 2.0, -2.0], 2, 2);
        push_ring(&mut vm, &[0], &[3.0, -3.0], 2, 2);
        assert_eq!(
            ring_values(&mut vm, &[0]),
            vec![vec![2.0, 3.0], vec![-2.0, -3.0]],
        );
    }

    /// A width change, a respawn after a rewire, reshapes the outer list.
    /// Surviving channels keep their ring tails and new channels start fresh.
    #[test]
    fn push_ring_width_change_reuses_surviving_rings() {
        let mut vm = test_vm();
        // Stereo, then the tap narrows to mono. Channel 0's ring survives.
        push_ring(&mut vm, &[0], &[1.0, -1.0, 2.0, -2.0], 4, 2);
        push_ring(&mut vm, &[0], &[3.0], 4, 1);
        assert_eq!(ring_values(&mut vm, &[0]), vec![vec![1.0, 2.0, 3.0]]);

        // Widening back to stereo restarts channel 1 empty, then filled.
        push_ring(&mut vm, &[0], &[4.0, -4.0], 4, 2);
        assert_eq!(
            ring_values(&mut vm, &[0]),
            vec![vec![1.0, 2.0, 3.0, 4.0], vec![-4.0]],
        );
    }

    /// A trailing partial frame is dropped rather than scrambling the deinterleave.
    #[test]
    fn push_ring_truncates_partial_frame() {
        let mut vm = test_vm();
        push_ring(&mut vm, &[0], &[1.0, -1.0, 2.0], 4, 2);
        assert_eq!(ring_values(&mut vm, &[0]), vec![vec![1.0], vec![-1.0]]);
    }

    /// A `channels` of 0 is clamped to 1.
    #[test]
    fn push_ring_channels_zero_clamped() {
        let mut vm = test_vm();
        push_ring(&mut vm, &[0], &[1.0, 2.0], 4, 0);
        assert_eq!(ring_values(&mut vm, &[0]), vec![vec![1.0, 2.0]]);
    }

    /// A flat single-ring list is treated as empty, since its elements are
    /// numbers, not rings.
    #[test]
    fn push_ring_legacy_flat_state_treated_as_empty() {
        let mut vm = test_vm();
        let flat = [1.0, 2.0].iter().map(|&v| SteelVal::NumV(v)).collect();
        state::update_value(&mut vm, &[0], SteelVal::ListV(flat)).unwrap();
        push_ring(&mut vm, &[0], &[3.0], 4, 1);
        assert_eq!(ring_values(&mut vm, &[0]), vec![vec![3.0]]);
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
