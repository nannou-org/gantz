//! A collection of useful widgets for gantz.

pub use command_palette::CommandPalette;
pub use gantz::{Gantz, GantzState};
pub use graph_scene::{GraphScene, GraphSceneState};
pub use label_button::LabelButton;
pub use label_toggle::LabelToggle;
pub use log_view::LogView;
pub use node_inspector::NodeInspector;

pub mod command_palette;
pub mod gantz;
pub mod graph_scene;
pub mod label_button;
pub mod label_toggle;
pub mod log_view;
pub mod node_inspector;

/// Simple shorthand for viewing steel code.
pub fn steel_view(ui: &mut egui::Ui, code: &str) {
    let language = "scm";
    let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
    egui_extras::syntax_highlighting::code_view_ui(ui, &theme, code, language);
}
