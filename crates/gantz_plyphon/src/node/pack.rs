//! The `~pack` node: concatenate signals into one channel group.

use gantz_core::node::{ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp, input_or_silent};

/// Concatenate `count` input signals into one channel group (like Max's
/// `mc.pack~` or a VCV merge): the output's width is the sum of the input
/// widths, an unconnected input contributing one channel of silence. Channels
/// are *packed*, never summed - summing is `~sum` (and per-input-gain mixing
/// a future `~mix`).
///
/// A routing node: it emits no UGens, it only re-groups wires at
/// synthdef-derivation time (and is Steel-inert like the other dsp nodes).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, NodeTag)]
pub struct Pack {
    #[serde(default = "default_count", skip_serializing_if = "is_default_count")]
    count: usize,
}

impl Pack {
    /// The number of inputs a fresh `~pack` starts with.
    pub const DEFAULT_COUNT: usize = 2;

    /// The number of dsp inputs to concatenate.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Set the input count (content-address affecting; structural - it changes
    /// the node's input sockets).
    pub fn set_count(&mut self, count: usize) {
        self.count = count.max(1);
    }
}

impl Default for Pack {
    fn default() -> Self {
        Pack {
            count: default_count(),
        }
    }
}

impl gantz_core::Node for Pack {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        // Every input is a dsp signal (any channel width each).
        self.count
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        // Steel-inert: the packing happens at synthdef derivation. A non-numeric
        // placeholder output feeds the inert dsp output edge (see the `NodeDsp`
        // docs).
        gantz_core::node::parse_expr("'()")
    }
}

impl NodeDsp for Pack {
    fn n_dsp_inputs(&self) -> usize {
        self.count
    }

    fn n_dsp_outputs(&self) -> usize {
        1
    }

    fn ugens(
        &self,
        _path: &[usize],
        inputs: &[Option<Signal>],
        _b: &mut DspBuilder,
    ) -> Vec<Signal> {
        // Pure re-grouping: no units, just the concatenation of every input's
        // channels (an unconnected input contributes mono silence).
        vec![Signal::concat(
            (0..inputs.len()).map(|i| input_or_silent(inputs, i)),
        )]
    }
}

impl ToNodeDsp for Pack {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

fn default_count() -> usize {
    Pack::DEFAULT_COUNT
}

fn is_default_count(count: &usize) -> bool {
    *count == default_count()
}
