//! The DSP domain's egui implementations. Everything behind the `egui`
//! feature lives here. That is the node set's `NodeUi` impls, their shared
//! inspector-row helpers, the settings subtab and the reference inspector's
//! `inline` toggle.
//!
//! Keeping every egui item here means the feature holds with a single cfg
//! gate on this module's declaration in the crate root. The rest of the crate
//! is headless by construction, so new UI code cannot leak egui into a
//! `--no-default-features` build.

pub use edge_style::DspEdgeStyle;
pub use node::sample::LoadSample;
pub use pane::{DSP_PANE_KEY, DspPane, DspPaneHead};
pub use ref_ext::DspRefExtUi;
pub use settings::DspSettingsTab;

pub mod edge_style;
pub mod envelope;
pub mod node;
pub mod pane;
pub mod param;
pub mod ref_ext;
pub mod settings;

// Lets the synthdef compiler and dsp driver find DSP nodes within the erased
// UI node by delegating to this crate's downcast probe.
impl crate::ToNodeDsp for gantz_egui::node::DynNode {
    fn to_node_dsp(&self) -> Option<&dyn crate::NodeDsp> {
        let node: &dyn gantz_core::Node = &**self;
        crate::node_dsp_of(node as &dyn std::any::Any)
    }
}
