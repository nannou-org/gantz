//! The `~buffer` scratch buffer source node.

use gantz_core::node::{ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

use crate::dsp::{BufferSource, DspBuilder, NodeDsp, Signal, ToNodeDsp};

/// A zeroed scratch buffer as a buffer source. It emits the bufnum wire of
/// the buffer for buffer sockets, for example a `~recordbuf` that writes it
/// and a `~playbuf` that reads it.
///
/// The audio driver allocates the buffer per open head and node path, so
/// each instance of a nested graph gets its own buffer. The buffer and its
/// contents survive a respawn of the synths that use it, as long as its
/// shape does not change. A change of `frames` or `channels` gives a new
/// zeroed buffer. Steel-inert like the other dsp nodes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, NodeTag)]
pub struct Buffer {
    #[serde(default = "default_frames", skip_serializing_if = "is_default_frames")]
    frames: usize,
    #[serde(
        default = "default_channels",
        skip_serializing_if = "is_default_channels"
    )]
    channels: usize,
}

impl Buffer {
    /// The frame count a fresh `~buffer` starts with, a power of two.
    pub const DEFAULT_FRAMES: usize = 65_536;

    /// The channel count a fresh `~buffer` starts with.
    pub const DEFAULT_CHANNELS: usize = 1;

    /// The largest frame count.
    pub const MAX_FRAMES: usize = 1 << 24;

    /// The largest channel count.
    pub const MAX_CHANNELS: usize = 32;

    /// A buffer of `frames` frames and `channels` channels, each clamped to
    /// its allowed range.
    pub fn new(frames: usize, channels: usize) -> Self {
        Buffer {
            frames: frames.clamp(1, Self::MAX_FRAMES),
            channels: channels.clamp(1, Self::MAX_CHANNELS),
        }
    }

    /// The frame count, within `1..=MAX_FRAMES`.
    pub fn frames(&self) -> usize {
        self.frames.clamp(1, Self::MAX_FRAMES)
    }

    /// The channel count, within `1..=MAX_CHANNELS`.
    pub fn channels(&self) -> usize {
        self.channels.clamp(1, Self::MAX_CHANNELS)
    }

    /// Set the frame count, clamped to `1..=MAX_FRAMES`. It is structural.
    pub fn set_frames(&mut self, frames: usize) {
        self.frames = frames.clamp(1, Self::MAX_FRAMES);
    }

    /// Set the channel count, clamped to `1..=MAX_CHANNELS`. It is structural.
    pub fn set_channels(&mut self, channels: usize) {
        self.channels = channels.clamp(1, Self::MAX_CHANNELS);
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer {
            frames: default_frames(),
            channels: default_channels(),
        }
    }
}

impl gantz_core::Node for Buffer {
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
}

impl NodeDsp for Buffer {
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
        let source = BufferSource::Scratch {
            frames: self.frames(),
        };
        vec![b.push_buffer(path, source, self.channels())]
    }
}

impl ToNodeDsp for Buffer {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

fn default_frames() -> usize {
    Buffer::DEFAULT_FRAMES
}

fn is_default_frames(frames: &usize) -> bool {
    *frames == default_frames()
}

fn default_channels() -> usize {
    Buffer::DEFAULT_CHANNELS
}

fn is_default_channels(channels: &usize) -> bool {
    *channels == default_channels()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_pushes_a_scratch_binding() {
        let mut b = DspBuilder::new(1);
        Buffer::new(128, 2).ugens(&[3], &[], &mut b);
        let finished = b.finish("t");
        assert_eq!(finished.buffers.len(), 1);
        let binding = &finished.buffers[0];
        assert_eq!(binding.source, BufferSource::Scratch { frames: 128 });
        assert_eq!(binding.channels, 2);
        assert_eq!(binding.node_path, vec![3]);
    }

    #[test]
    fn shape_is_clamped() {
        let buffer = Buffer::new(0, 1_000);
        assert_eq!(buffer.frames(), 1);
        assert_eq!(buffer.channels(), Buffer::MAX_CHANNELS);
    }

    #[test]
    fn default_serializes_bare() {
        assert_eq!(ron::to_string(&Buffer::default()).unwrap(), "()");
    }
}
