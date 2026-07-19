//! The `~sum` node: sum signals into their unity-gain mix.

use gantz_core::node::{ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp, sum_signals};

/// Sum `count` input signals into one channel group - the same unity-gain mix
/// ([`sum_signals`]) as wiring several edges into one input, as an explicit
/// node: the output is as wide as the widest input, a mono input broadcasts
/// across every channel and a narrower one contributes silence past its own
/// width. Per-input gain is a different (future `~mix`) node; channel
/// concatenation is `~pack`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, NodeTag)]
pub struct Sum {
    #[serde(default = "default_count", skip_serializing_if = "is_default_count")]
    count: usize,
}

impl Sum {
    /// The number of inputs a fresh `~sum` starts with.
    pub const DEFAULT_COUNT: usize = 2;

    /// The number of dsp inputs to sum.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Set the input count (content-address affecting; structural - it changes
    /// the node's input sockets).
    pub fn set_count(&mut self, count: usize) {
        self.count = count.max(1);
    }
}

impl Default for Sum {
    fn default() -> Self {
        Sum {
            count: default_count(),
        }
    }
}

impl gantz_core::Node for Sum {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        // Every input is a dsp signal (any channel width each).
        self.count
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        // Steel-inert: the summing happens at synthdef derivation. A non-numeric
        // placeholder output feeds the inert dsp output edge (see the `NodeDsp`
        // docs).
        gantz_core::node::parse_expr("'()")
    }
}

impl NodeDsp for Sum {
    fn n_dsp_inputs(&self) -> usize {
        self.count
    }

    fn n_dsp_outputs(&self) -> usize {
        1
    }

    fn ugens(&self, _path: &[usize], inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        // Unconnected inputs contribute nothing (silence would fold away).
        let sigs: Vec<Signal> = inputs.iter().flatten().cloned().collect();
        vec![sum_signals(b, &sigs)]
    }
}

impl ToNodeDsp for Sum {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

fn default_count() -> usize {
    Sum::DEFAULT_COUNT
}

fn is_default_count(count: &usize) -> bool {
    *count == default_count()
}
