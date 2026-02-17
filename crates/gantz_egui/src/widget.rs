//! A collection of useful widgets for gantz.
#[cfg(target_arch = "wasm32")]
use js_sys::Date;
use time::{OffsetDateTime, UtcOffset, format_description};

pub use command_palette::CommandPalette;
pub use gantz::{Gantz, GantzState, update_graph_pane_head};
pub use graph_config::{GraphConfig, GraphConfigResponse};
pub use graph_scene::{GraphScene, GraphSceneState};
pub use graph_select::GraphSelect;
pub use graph_tab::{GraphTab, GraphTabResponse};
<<<<<<< HEAD
pub use head_name_edit::{HeadNameEditResponse, head_name, head_name_edit};
pub use head_row::{HeadRowResponse, HeadRowType, fmt_commit_timestamp, head_row};
=======
pub use head_row::{HeadRowResponse, HeadRowType, head_row};
>>>>>>> 98b4c97 (Refactor: abstract timestamp formatting into shared helper function)
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

/// Convert a UTC datetime to local timezone, with fallback to UTC if unavailable.
/// Convert a UTC datetime to local timezone, with fallback to UTC if unavailable.
pub(crate) fn to_local_datetime(datetime: OffsetDateTime) -> OffsetDateTime {
    UtcOffset::current_local_offset()
        .map(|offset| datetime.to_offset(offset))
        .unwrap_or(datetime)
}

/// Format a SystemTime as a local datetime string.
pub(crate) fn format_local_datetime(system_time: std::time::SystemTime) -> String {
    let datetime = OffsetDateTime::from(system_time);
    let local_datetime = to_local_datetime(datetime);

    let format = format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]")
        .expect("invalid format");

    local_datetime
        .format(&format)
        .unwrap_or_else(|_| "<invalid-timestamp>".to_string())
}

/// Simple shorthand for viewing steel code.
pub fn steel_view(ui: &mut egui::Ui, code: &str) {
    let language = "scm";
    let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
    egui_extras::syntax_highlighting::code_view_ui(ui, &theme, code, language);
}
