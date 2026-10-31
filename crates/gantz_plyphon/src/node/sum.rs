//! The `~sum` node: sum signals into their unity-gain mix.

use gantz_ca::Datum;
use gantz_nodedef::{ExprArgs, Meta, View};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

use crate::dsp::{DspBuilder, NodeDsp, Signal, ToNodeDsp, sum_signals};
use crate::view::{COUNT, MAX_COUNT, count_row, doc, edit_count};

/// Sum `count` input signals into one channel group. It is the same
/// unity-gain mix as wiring several edges into one input, see
/// [`sum_signals`], as an explicit node. The output is as wide as the widest
/// input, a mono input broadcasts across every channel and a narrower one
/// contributes silence past its own width. Channel concatenation is `~pack`.
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

    /// Set the input count. It is structural and affects the content address,
    /// since it changes the node's input sockets.
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

impl gantz_nodedef::NodeDef for Sum {
    const TAG: &'static str = <Self as NodeTag>::TAG;

    fn meta(&self) -> Meta {
        // Every input is a dsp signal of any channel width.
        Meta {
            name: "~sum".into(),
            description: "Sum signals into their unity-gain mix (mixed, not packed)".into(),
            inputs: self.count,
            outputs: 1,
            ..Meta::default()
        }
    }

    fn expr(&self, _ctx: ExprArgs<'_>) -> Result<String, String> {
        // Steel-inert. The summing happens at synthdef derivation. A
        // non-numeric placeholder output feeds the inert dsp output edge, see
        // the `NodeDsp` docs.
        Ok("'()".into())
    }

    fn view(&self) -> View {
        let input = |ix| {
            let description =
                format!("signal {ix} to sum, of any channel width. Silence when unconnected");
            doc("signal", description)
        };
        let output = "the channel-wise sum. Its width is the widest input. Mono inputs broadcast";
        View {
            inputs: (0..self.count).map(input).collect(),
            outputs: vec![doc("signal", output)],
            rows: vec![count_row(self.count, "the number of signals to sum")],
            ..View::default()
        }
    }

    fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
        let count = edit_count(value, [1, MAX_COUNT]).filter(|_| edit == COUNT)?;
        Some(Sum { count })
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
        // Unconnected inputs contribute nothing, since silence would fold away.
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
