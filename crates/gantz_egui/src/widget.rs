//! A collection of useful widgets for gantz.
use time::{OffsetDateTime, UtcOffset, format_description};

pub use checkbox_enabled::CheckboxEnabled;
pub use collab_config::{CollabSettings, CollabSettingsTab, collab_config};
pub use edge_style::{EdgeStyle, EdgeStyleCtx, EdgeStyling};
pub use ext_pane::{ExtPane, ExtPaneCtx, ExtPaneEntry};
pub use gantz::{
    AlignConfig, BaseSourcesCtx, Gantz, GantzState, GridConfig, LayoutConfig, NodeViewPane, Pane,
    PaneWindowGeometry, PaneWindowMode, SceneConfig, SnapConfig, SnapMode, WindowedPane,
    close_windowed_pane, pane_key, redock_windowed_pane, update_graph_pane_head,
};
pub use global_config::{GlobalConfigResponse, global_config};
pub use graph_config::{GraphConfig, GraphConfigResponse};
pub use graph_scene::{GraphScene, GraphSceneState};
pub use graph_select::GraphSelect;
pub use head_name_edit::{HeadNameEditResponse, head_name, head_name_edit};
pub use head_row::{HeadRowResponse, HeadRowType, fmt_commit_timestamp, head_row};
pub use history_view::{HistoryMode, HistoryView, HistoryViewState};
pub use keybinds_config::keybinds_config;
pub use label_button::LabelButton;
pub use label_toggle::LabelToggle;
pub use log_view::LogView;
pub use node_inspector::NodeInspector;
pub use node_palette::NodePalette;
pub use panes_config::{panes_config, reset_layout_button};
pub use perf_view::{PerfCapture, PerfView};
pub use settings::{SettingsResponse, SettingsTab, settings};
pub use status_dot::status_dot;
pub use steel_view::SteelView;
pub use style_config::{StyleConfigResponse, style_config};
pub use tab::{Tab, TabResponse};

pub mod checkbox_enabled;
pub mod collab_config;
pub mod edge_style;
pub mod ext_pane;
pub mod gantz;
pub mod global_config;
pub mod graph_config;
pub mod graph_scene;
pub mod graph_select;
pub mod gui_debug;
pub mod head_name_edit;
pub mod head_row;
pub mod history_view;
pub mod keybinds_config;
pub mod label_button;
pub mod label_toggle;
pub mod log_view;
pub mod node_inspector;
pub mod node_palette;
pub mod panes_config;
pub mod perf_view;
pub mod settings;
pub mod status_dot;
pub mod steel_view;
pub mod style_config;
pub mod tab;
#[cfg(feature = "tracing")]
pub mod trace_view;

/// Level label colours for the log and trace views.
pub(crate) struct LevelColors {
    pub error: egui::Color32,
    pub warn: egui::Color32,
    pub info: egui::Color32,
    pub debug: egui::Color32,
    pub trace: egui::Color32,
}

impl LevelColors {
    pub(crate) fn from_visuals(visuals: &egui::Visuals) -> Self {
        Self {
            error: visuals.error_fg_color,
            warn: visuals.warn_fg_color,
            info: visuals.widgets.hovered.fg_stroke.color,
            debug: egui::Color32::GRAY,
            trace: egui::Color32::DARK_GRAY,
        }
    }
}

/// Convert a UTC datetime to the local timezone. Falls back to UTC when the
/// local offset is unavailable.
pub(crate) fn to_local_datetime(datetime: OffsetDateTime) -> OffsetDateTime {
    UtcOffset::current_local_offset()
        .map(|offset| datetime.to_offset(offset))
        .unwrap_or(datetime)
}

/// The glyph for a widget's options button. Swap it if it does not render.
pub(crate) const OPTIONS_GLYPH: &str = "⛭";

/// Format a SystemTime as a local string using the given `time` format
/// description.
fn format_local(system_time: std::time::SystemTime, desc: &str) -> String {
    let datetime = OffsetDateTime::from(system_time);
    let local_datetime = to_local_datetime(datetime);
    let format = format_description::parse_borrowed::<2>(desc).expect("invalid format");
    local_datetime
        .format(&format)
        .unwrap_or_else(|_| "<invalid-timestamp>".to_string())
}

/// Format a SystemTime as a local `YYYY-MM-DD HH:MM:SS` datetime string.
pub(crate) fn format_local_datetime(system_time: std::time::SystemTime) -> String {
    format_local(system_time, "[year]-[month]-[day] [hour]:[minute]:[second]")
}

/// Format a SystemTime as a local `HH:MM:SS` time-of-day string without a date.
pub(crate) fn format_local_time(system_time: std::time::SystemTime) -> String {
    format_local(system_time, "[hour]:[minute]:[second]")
}

/// Group consecutive slice elements that `eq` considers equal into runs.
/// Returns `(index_of_first, count)` pairs in order.
///
/// The log and trace views use this to collapse repeated entries into a
/// single row with an occurrence count.
pub(crate) fn group_runs<T>(items: &[T], eq: impl Fn(&T, &T) -> bool) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if let Some((first, count)) = runs.last_mut() {
            if eq(&items[*first], item) {
                *count += 1;
                continue;
            }
        }
        runs.push((i, 1));
    }
    runs
}

/// Simple shorthand for viewing steel code without highlights.
pub fn steel_view(ui: &mut egui::Ui, code: &str) {
    SteelView::new(code).show(ui);
}

/// A titled, full-width group. Settings tabs use it to separate sections.
pub fn section<R>(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.group(|ui| {
        ui.set_min_width(ui.available_width());
        ui.strong(title);
        add_contents(ui)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::group_runs;

    #[test]
    fn group_runs_collapses_consecutive_equal_items() {
        // Only consecutive equal items collapse. The trailing `a` items form
        // a separate run from the leading ones.
        let items = ['a', 'a', 'a', 'b', 'a', 'a'];
        let runs = group_runs(&items, |x, y| x == y);
        assert_eq!(runs, vec![(0, 3), (3, 1), (4, 2)]);
    }

    #[test]
    fn group_runs_empty_is_empty() {
        let items: [char; 0] = [];
        assert!(group_runs(&items, |x, y| x == y).is_empty());
    }

    #[test]
    fn group_runs_all_distinct() {
        let items = [1, 2, 3];
        let runs = group_runs(&items, |x, y| x == y);
        assert_eq!(runs, vec![(0, 1), (1, 1), (2, 1)]);
    }
}
