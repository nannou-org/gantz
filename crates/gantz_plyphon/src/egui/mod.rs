//! The DSP domain's egui implementations. Everything behind the `egui`
//! feature lives here. That is the settings subtab, the DSP pane, the edge
//! styles and the reference inspector's `inline` toggle. The views of the DSP
//! nodes are `gantz_ui` data, see [`crate::node`].
//!
//! Keeping every egui item here means the feature holds with a single cfg
//! gate on this module's declaration in the crate root. The rest of the crate
//! is headless by construction, so new UI code cannot leak egui into a
//! `--no-default-features` build.

pub use edge_style::DspEdgeStyle;
pub use pane::{DSP_PANE_KEY, DspPane, DspPaneHead};
pub use ref_ext::DspRefExtUi;
pub use settings::DspSettingsTab;

pub mod edge_style;
pub mod pane;
pub mod ref_ext;
pub mod settings;

// Lets the synthdef compiler and dsp driver find DSP nodes within the erased
// UI node. A DSP node is a def node whose decoded node is a `DspNode`. See
// `crate::node::node_def`.
impl crate::ToNodeDsp for gantz_egui::node::DynNode {
    fn to_node_dsp(&self) -> Option<&dyn crate::NodeDsp> {
        let node: &dyn gantz_core::Node = &**self;
        let any: &dyn std::any::Any = node;
        let node = any.downcast_ref::<gantz_egui::node::DefNode>()?;
        node.node::<crate::DspNode>().map(crate::DspNode::dsp)
    }
}
