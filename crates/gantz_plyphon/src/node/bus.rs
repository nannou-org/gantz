//! The `~bus` node: a synthdef boundary on a signal wire.

use gantz_nodedef::{ExprArgs, Meta, View};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp, input_or_silent};
use crate::view::doc;

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

impl gantz_nodedef::NodeDef for Bus {
    const TAG: &'static str = <Self as NodeTag>::TAG;

    fn meta(&self) -> Meta {
        Meta {
            name: "~bus".into(),
            description: "Synthdef boundary: edits beyond it leave this side's synth running"
                .into(),
            inputs: 1,
            outputs: 1,
            ..Meta::default()
        }
    }

    fn expr(&self, _ctx: ExprArgs<'_>) -> Result<String, String> {
        // Steel-inert. The boundary exists only at synthdef derivation. A
        // non-numeric placeholder output feeds the inert dsp output edge, see
        // the `NodeDsp` docs.
        Ok("'()".into())
    }

    fn view(&self) -> View {
        View {
            inputs: vec![doc(
                "signal",
                "signal to carry across the synthdef boundary (any channel width)",
            )]
            .into(),
            outputs: vec![doc(
                "signal",
                "the same signal, entering the downstream synthdef",
            )]
            .into(),
            ..View::default()
        }
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
