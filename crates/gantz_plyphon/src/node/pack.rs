//! The `~pack` node: concatenate signals into one channel group.

use gantz_ca::Datum;
use gantz_nodedef::{ExprArgs, Meta, View};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp, input_or_silent};
use crate::view::{COUNT, MAX_COUNT, count_row, doc, edit_count};

/// Concatenate `count` input signals into one channel group, like Max's
/// `mc.pack~` or a VCV merge. The output's width is the sum of the input
/// widths. An unconnected input contributes one channel of silence. Channels
/// are packed, never summed. Summing is `~sum`.
///
/// A routing node. It emits no UGens and only re-groups wires at
/// synthdef-derivation time. It is Steel-inert like the other dsp nodes.
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

    /// Set the input count. It is structural and affects the content address,
    /// since it changes the node's input sockets.
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

impl gantz_nodedef::NodeDef for Pack {
    const TAG: &'static str = <Self as NodeTag>::TAG;

    fn meta(&self) -> Meta {
        // Every input is a dsp signal of any channel width.
        Meta {
            name: "~pack".into(),
            description: "Concatenate signals into one multichannel signal (packed, not mixed)"
                .into(),
            inputs: self.count,
            outputs: 1,
            ..Meta::default()
        }
    }

    fn expr(&self, _ctx: ExprArgs<'_>) -> Result<String, String> {
        // Steel-inert. The packing happens at synthdef derivation. A
        // non-numeric placeholder output feeds the inert dsp output edge, see
        // the `NodeDsp` docs.
        Ok("'()".into())
    }

    fn view(&self) -> View {
        let input = |ix| {
            let description =
                format!("signal {ix} to pack, of any channel width. Silence when unconnected");
            doc("signal", description)
        };
        let output = "every input's channels concatenated (width = the sum of input widths)";
        View {
            inputs: (0..self.count).map(input).collect(),
            outputs: vec![doc("signal", output)],
            rows: vec![count_row(self.count, "the number of signals to pack")],
            ..View::default()
        }
    }

    fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
        let count = edit_count(value, [1, MAX_COUNT]).filter(|_| edit == COUNT)?;
        Some(Pack { count })
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
        // Pure re-grouping with no units, the concatenation of every input's
        // channels.
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
