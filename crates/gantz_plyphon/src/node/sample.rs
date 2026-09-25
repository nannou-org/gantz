//! The `~sample` buffer source node.

use std::hash::{Hash, Hasher};

use gantz_ca::ContentAddr;
use gantz_core::node::{ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use plyphon::synthdef::InputRef;
use serde::{Deserialize, Serialize};

use crate::dsp::{BufferSource, DspBuilder, NodeDsp, Signal, ToNodeDsp};

/// A content-addressed audio asset as a buffer source. It emits the bufnum
/// wire of the asset's buffer for buffer sockets such as `~playbuf`'s.
///
/// The node holds the asset's address plus a cache of its channel count,
/// frame count and sample rate, so a reader can size its outputs without
/// decoding the PCM. The samples live in the content-addressed asset store.
/// The audio driver installs the asset once, shares it read-only across every
/// synth that reads it, and sets this node's bufnum param after spawning.
/// Writers cannot write to it, see [`BufferAccess`](crate::BufferAccess).
///
/// An unassigned node emits the bufnum `-1`, which reads an empty buffer.
/// Steel-inert like the other dsp nodes.
#[derive(Clone, Debug, Default, Serialize, Deserialize, NodeTag)]
pub struct Sample {
    /// The audio asset, or `None` until one is assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    asset: Option<ContentAddr>,
    /// The asset's cached channel count. It sizes a reader's output group.
    #[serde(default, skip_serializing_if = "crate::node::is_default")]
    channels: usize,
    /// The asset's cached frame count, for display.
    #[serde(default, skip_serializing_if = "crate::node::is_default")]
    frames: usize,
    /// The asset's cached sample rate in Hz, for display.
    #[serde(default, skip_serializing_if = "crate::node::is_default")]
    sample_rate: f64,
}

impl Sample {
    /// A node for `asset`, whose PCM has `channels` channels of `frames`
    /// frames at `sample_rate` Hz.
    pub fn new(asset: ContentAddr, channels: usize, frames: usize, sample_rate: f64) -> Self {
        Sample {
            asset: Some(asset),
            channels,
            frames,
            sample_rate,
        }
    }

    /// A node for the given asset, with its metadata cached.
    pub fn from_asset(asset: &crate::AudioAsset) -> Self {
        Sample::new(
            asset.addr(),
            asset.num_channels(),
            asset.num_frames(),
            asset.sample_rate(),
        )
    }

    /// The assigned asset, if any.
    pub fn asset(&self) -> Option<ContentAddr> {
        self.asset
    }

    /// The cached channel count, at least 1.
    pub fn channels(&self) -> usize {
        self.channels.max(1)
    }

    /// The cached frame count.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// The cached sample rate in Hz.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
}

impl PartialEq for Sample {
    fn eq(&self, other: &Self) -> bool {
        self.asset == other.asset
            && self.channels == other.channels
            && self.frames == other.frames
            && self.sample_rate.to_bits() == other.sample_rate.to_bits()
    }
}

impl Eq for Sample {}

impl Hash for Sample {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.asset.hash(state);
        self.channels.hash(state);
        self.frames.hash(state);
        self.sample_rate.to_bits().hash(state);
    }
}

impl gantz_core::Node for Sample {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        0
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        // Steel-inert. A non-numeric placeholder output feeds the inert dsp
        // output edge, see the `NodeDsp` docs.
        gantz_core::node::parse_expr("'()")
    }

    fn required_blobs(&self) -> Vec<(gantz_ca::SectionId, ContentAddr)> {
        self.asset
            .into_iter()
            .map(|addr| (crate::BUFFER_SECTION.to_string(), addr))
            .collect()
    }
}

impl NodeDsp for Sample {
    fn n_dsp_inputs(&self) -> usize {
        0
    }

    fn n_dsp_outputs(&self) -> usize {
        1
    }

    fn is_buffer_source(&self) -> bool {
        true
    }

    fn ugens(&self, path: &[usize], _inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        let bufnum = match self.asset {
            Some(asset) => b.push_buffer(path, BufferSource::Asset(asset), self.channels()),
            None => Signal::mono(InputRef::Constant(-1.0)),
        };
        vec![bufnum]
    }
}

impl ToNodeDsp for Sample {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::Node as _;

    #[test]
    fn assigned_sample_pushes_an_asset_binding() {
        let addr = gantz_ca::blob_addr(b"pcm");
        let mut b = DspBuilder::new(1);
        let outs = Sample::new(addr, 2, 64, 44_100.0).ugens(&[0], &[], &mut b);
        assert!(matches!(outs[0].channel(0), Some(InputRef::Param(_))));
        let finished = b.finish("t");
        assert_eq!(finished.buffers.len(), 1);
        assert_eq!(finished.buffers[0].source, BufferSource::Asset(addr));
        assert_eq!(finished.buffers[0].channels, 2);
        assert!(finished.def.units.is_empty(), "a source emits no units");
    }

    #[test]
    fn unassigned_sample_emits_minus_one() {
        let mut b = DspBuilder::new(1);
        let outs = Sample::default().ugens(&[0], &[], &mut b);
        assert!(matches!(outs[0].channel(0), Some(InputRef::Constant(v)) if v == -1.0));
        assert!(b.finish("t").buffers.is_empty());
    }

    #[test]
    fn required_blobs_surfaces_the_asset() {
        let addr = gantz_ca::blob_addr(b"x");
        assert_eq!(
            Sample::new(addr, 1, 1, 1.0).required_blobs(),
            vec![(crate::BUFFER_SECTION.to_string(), addr)],
        );
        assert!(Sample::default().required_blobs().is_empty());
    }
}
