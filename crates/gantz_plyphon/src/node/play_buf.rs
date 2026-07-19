//! The `~playbuf` sample-playback node.

use std::hash::{Hash, Hasher};

use gantz_ca::ContentAddr;
use gantz_core::node::{ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use plyphon::Rate;
use plyphon::synthdef::{InputRef, UnitSpec};
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp};

/// Play a content-addressed audio buffer back through plyphon's `PlayBuf`
/// (scsynth's sampler), looping, one output channel per buffer channel.
///
/// The node holds only the asset's *address* plus a cache of its channel count
/// and sample rate - enough to size the output group and set the playback rate
/// without decoding the PCM. The samples themselves live in the content-addressed
/// asset store; the audio driver makes the referenced asset resident, allocates
/// a bufnum, installs the buffer, and sets this node's driver-owned `bufnum` and
/// `rate` control params after spawning (see [`BufferBinding`](crate::BufferBinding)).
///
/// An unassigned node (no asset) reads a guaranteed-missing buffer, so it is
/// silent until an asset is set. Steel-inert like the other dsp nodes.
#[derive(Clone, Debug, Serialize, Deserialize, NodeTag)]
pub struct PlayBuf {
    /// The audio asset to play, or `None` until one is assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    asset: Option<ContentAddr>,
    /// The asset's channel count (cached metadata; sizes the output group).
    #[serde(
        default = "default_channels",
        skip_serializing_if = "is_default_channels"
    )]
    num_channels: usize,
    /// The asset's own sample rate in Hz (cached metadata; the driver divides
    /// it by the engine rate to set the playback `rate`).
    #[serde(default, skip_serializing_if = "crate::node::is_default")]
    sample_rate: f64,
}

impl PlayBuf {
    /// The channel count a fresh, unassigned `~playbuf` reports.
    pub const DEFAULT_CHANNELS: usize = 1;

    /// A node playing `asset`, whose PCM has `num_channels` channels at
    /// `sample_rate` Hz.
    pub fn new(asset: ContentAddr, num_channels: usize, sample_rate: f64) -> Self {
        PlayBuf {
            asset: Some(asset),
            num_channels: num_channels.max(1),
            sample_rate,
        }
    }

    /// The audio asset this node plays, if one is assigned.
    pub fn asset(&self) -> Option<ContentAddr> {
        self.asset
    }

    /// The cached channel count (at least 1).
    pub fn num_channels(&self) -> usize {
        self.num_channels.max(1)
    }

    /// The cached sample rate in Hz.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Assign `asset` and its metadata (content-address affecting; the channel
    /// count is structural - it resizes the output group).
    pub fn set_asset(&mut self, asset: ContentAddr, num_channels: usize, sample_rate: f64) {
        self.asset = Some(asset);
        self.num_channels = num_channels.max(1);
        self.sample_rate = sample_rate;
    }
}

impl Default for PlayBuf {
    fn default() -> Self {
        PlayBuf {
            asset: None,
            num_channels: default_channels(),
            sample_rate: 0.0,
        }
    }
}

impl PartialEq for PlayBuf {
    fn eq(&self, other: &Self) -> bool {
        self.asset == other.asset
            && self.num_channels == other.num_channels
            && self.sample_rate.to_bits() == other.sample_rate.to_bits()
    }
}

impl Eq for PlayBuf {}

impl Hash for PlayBuf {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Hash::hash(&self.asset, state);
        Hash::hash(&self.num_channels, state);
        Hash::hash(&self.sample_rate.to_bits(), state);
    }
}

impl gantz_core::Node for PlayBuf {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        0
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        // Steel-inert: playback happens in the audio engine. A non-numeric
        // placeholder output feeds the inert dsp output edge (see the `NodeDsp`
        // docs).
        gantz_core::node::parse_expr("'()")
    }

    fn required_blobs(&self) -> Vec<(gantz_ca::SectionId, ContentAddr)> {
        self.asset
            .into_iter()
            .map(|addr| (crate::BUFFER_SECTION.to_string(), addr))
            .collect()
    }
}

impl NodeDsp for PlayBuf {
    fn n_dsp_inputs(&self) -> usize {
        0
    }

    fn n_dsp_outputs(&self) -> usize {
        1
    }

    fn ugens(&self, path: &[usize], _inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        let channels = self.num_channels();
        // `bufnum`/`rate` are driver-owned no-lag control params set after spawn
        // (like scope bufnums / bus indices), so an assigned node makes its asset
        // resident via a `BufferBinding`. An unassigned node instead reads a
        // guaranteed-missing buffer (`-1`), which `PlayBuf` renders as silence.
        let (bufnum, rate) = match self.asset {
            Some(asset) => {
                let bufnum = b.push_control_param(path, "bufnum");
                let rate = b.push_control_param(path, "rate");
                b.push_buffer(path, asset, bufnum, rate, self.sample_rate);
                (InputRef::Param(bufnum), InputRef::Param(rate))
            }
            None => (InputRef::Constant(-1.0), InputRef::Constant(1.0)),
        };
        // PlayBuf.ar(bufnum, rate, trig, startPos, loop, doneAction), one output
        // per buffer channel. The slice bakes loop on, no retrigger, no done action.
        let inputs = vec![
            bufnum,
            rate,
            InputRef::Constant(0.0), // trig
            InputRef::Constant(0.0), // startPos
            InputRef::Constant(1.0), // loop
            InputRef::Constant(0.0), // doneAction
        ];
        let unit = b.push_unit(UnitSpec::new("PlayBuf", Rate::Audio, inputs, channels));
        let signal = (0..channels as u32)
            .map(|output| InputRef::Unit { unit, output })
            .collect();
        vec![signal]
    }
}

impl ToNodeDsp for PlayBuf {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

fn default_channels() -> usize {
    PlayBuf::DEFAULT_CHANNELS
}

fn is_default_channels(num_channels: &usize) -> bool {
    *num_channels == default_channels()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::Node as _;

    #[test]
    fn assigned_playbuf_emits_unit_and_binding() {
        let addr = gantz_ca::blob_addr(b"some pcm");
        let node = PlayBuf::new(addr, 1, 44_100.0);
        let mut b = DspBuilder::new(2);
        let outs = node.ugens(&[0], &[], &mut b);
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].width(), 1);
        let finished = b.finish("t");
        assert!(finished.def.units.iter().any(|u| u.name == "PlayBuf"));
        assert_eq!(finished.buffers.len(), 1);
        assert_eq!(finished.buffers[0].asset, addr);
        assert_eq!(finished.buffers[0].sample_rate, 44_100.0);
    }

    #[test]
    fn unassigned_playbuf_is_silent_with_no_binding() {
        let node = PlayBuf::default();
        let mut b = DspBuilder::new(2);
        node.ugens(&[0], &[], &mut b);
        let finished = b.finish("t");
        assert!(finished.buffers.is_empty());
        let playbuf = finished
            .def
            .units
            .iter()
            .find(|u| u.name == "PlayBuf")
            .expect("PlayBuf unit");
        // A missing-buffer bufnum (`-1`), read by `PlayBuf` as silence.
        assert!(matches!(playbuf.inputs[0], InputRef::Constant(v) if v == -1.0));
    }

    #[test]
    fn required_blobs_surfaces_the_asset() {
        let addr = gantz_ca::blob_addr(b"x");
        assert_eq!(
            PlayBuf::new(addr, 1, 1.0).required_blobs(),
            vec![(crate::BUFFER_SECTION.to_string(), addr)],
        );
        assert!(PlayBuf::default().required_blobs().is_empty());
    }
}
