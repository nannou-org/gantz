//! The `~bus` node: a synthdef boundary on a signal wire.

use gantz_core::node::{ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp, input_or_silent};

/// A synthdef boundary. Drop it on a signal wire to cut the derived synthdef
/// there. The upstream region ends in an `Out` to a driver-allocated private
/// bus and the downstream region begins with an `In` from it, so the two
/// sides become separate synths. An edit then respawns only its own region.
/// The other side's unit state, such as oscillator phase and delay lines,
/// survives untouched.
///
/// The bus carries the input signal's full channel group. Its width is
/// inferred, like `~scopeout`. A `~bus` whose two sides land in the same
/// region anyway costs nothing and lowers to a plain wire. Cutting comes at a
/// price on the wire itself. The write is lifted to audio rate and
/// fade-gained for the crossfade. Cross-region feedback is not supported. A
/// bus cycle fails derivation, see [`DeriveError`](crate::DeriveError).
///
/// A `~bus` fed by several summands keeps only its cut role. Each transitive
/// source writes its own implicit single-writer bus and every reader emits
/// one `In` per source, summing after the reads.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash, NodeTag)]
pub struct Bus {}

impl gantz_core::Node for Bus {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        // Steel-inert. The boundary exists only at synthdef derivation. A
        // non-numeric placeholder output feeds the inert dsp output edge, see
        // the `NodeDsp` docs.
        gantz_core::node::parse_expr("'()")
    }
}

impl NodeDsp for Bus {
    fn n_dsp_inputs(&self) -> usize {
        1
    }

    fn n_dsp_outputs(&self) -> usize {
        1
    }

    fn is_boundary(&self) -> bool {
        true
    }

    fn ugens(
        &self,
        _path: &[usize],
        inputs: &[Option<Signal>],
        _b: &mut DspBuilder,
    ) -> Vec<Signal> {
        // Only reached when both sides share a region, so the boundary was not
        // a cut and lowers to a plain wire. `derive_synthdefs` lowers the cut
        // case itself and emits the bus `Out`/`In` pair.
        vec![input_or_silent(inputs, 0)]
    }
}

impl ToNodeDsp for Bus {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}
