//! A collection of useful widgets for gantz.

pub use command_palette::CommandPalette;
pub use gantz::{Gantz, GantzState, update_graph_pane_head};
pub use graph_config::{GraphConfig, GraphConfigResponse};
pub use graph_scene::{GraphScene, GraphSceneState};
pub use graph_select::GraphSelect;
pub use graph_tab::{GraphTab, GraphTabResponse};
pub use head_name_edit::{HeadNameEditResponse, head_name, head_name_edit};
pub use head_row::{HeadRowResponse, HeadRowType, fmt_commit_timestamp, head_row};
pub use history_view::{HistoryMode, HistoryView, HistoryViewState};
pub use label_button::LabelButton;
pub use label_toggle::LabelToggle;
pub use log_view::LogView;
pub use node_inspector::NodeInspector;
pub use pane_menu::PaneMenu;
pub use perf_view::{PerfCapture, PerfView};

pub mod command_palette;
pub mod gantz;
pub mod graph_config;
pub mod graph_scene;
pub mod graph_select;
pub mod graph_tab;
pub mod head_name_edit;
pub mod head_row;
pub mod history_view;
pub mod label_button;
pub mod label_toggle;
pub mod log_view;
pub mod node_inspector;
pub mod pane_menu;
pub mod perf_view;
#[cfg(feature = "tracing")]
pub mod trace_view;

/// Simple shorthand for viewing steel code.
pub fn steel_view(ui: &mut egui::Ui, code: &str) {
    let language = "scm";
    let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
    egui_extras::syntax_highlighting::code_view_ui(ui, &theme, code, language);
}
