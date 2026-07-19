//! The `~unpack` node: split a channel group into mono outputs.

use gantz_core::node::{ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp, input_or_silent};

/// Split a channel group into `count` mono outputs (like Max's `mc.unpack~` or
/// a VCV split): output `i` carries the input's channel `i`, or silence past
/// the input's width.
///
/// A routing node: it emits no UGens, it only re-groups wires at
/// synthdef-derivation time (and is Steel-inert like the other dsp nodes).
///
/// Shrinking `count` while edges hang off the removed outputs leaves those
/// edges dangling: the Steel compile surfaces an error diagnostic until they
/// are deleted (the same contract as `Expr`'s `#:out`), while synthdef
/// derivation silently ignores them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, NodeTag)]
pub struct Unpack {
    #[serde(default = "default_count", skip_serializing_if = "is_default_count")]
    count: usize,
}

impl Unpack {
    /// The number of outputs a fresh `~unpack` starts with.
    pub const DEFAULT_COUNT: usize = 2;

    /// The number of mono outputs the input signal splits into.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Set the output count (content-address affecting; structural - it changes
    /// the node's output sockets).
    pub fn set_count(&mut self, count: usize) {
        self.count = count.max(1);
    }
}

impl Default for Unpack {
    fn default() -> Self {
        Unpack {
            count: default_count(),
        }
    }
}

impl gantz_core::Node for Unpack {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        // A single dsp signal input carrying the whole channel group.
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        self.count
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        // Steel-inert: the splitting happens at synthdef derivation. Non-numeric
        // placeholder outputs feed the inert dsp output edges (see the `NodeDsp`
        // docs) - a single value for one output, a list of values otherwise (the
        // multi-output expr contract).
        let src = match self.count {
            1 => "'()".to_string(),
            n => format!("(list {})", vec!["'()"; n].join(" ")),
        };
        gantz_core::node::parse_expr(&src)
    }
}

impl NodeDsp for Unpack {
    fn n_dsp_inputs(&self) -> usize {
        1
    }

    fn n_dsp_outputs(&self) -> usize {
        self.count
    }

    fn ugens(
        &self,
        _path: &[usize],
        inputs: &[Option<Signal>],
        _b: &mut DspBuilder,
    ) -> Vec<Signal> {
        // Pure re-grouping: no units, output `i` = the input's channel `i` (or
        // mono silence past the input's width).
        let signal = input_or_silent(inputs, 0);
        (0..self.count)
            .map(|i| {
                signal
                    .channel(i)
                    .map(Signal::mono)
                    .unwrap_or_else(|| Signal::silent(1))
            })
            .collect()
    }
}

impl ToNodeDsp for Unpack {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

fn default_count() -> usize {
    Unpack::DEFAULT_COUNT
}

fn is_default_count(count: &usize) -> bool {
    *count == default_count()
}
