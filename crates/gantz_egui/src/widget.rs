//! A collection of useful widgets for gantz.
use std::collections::VecDeque;
use time::{OffsetDateTime, UtcOffset, format_description};

pub use buffered_edit::{BufferedEditResponse, buffered_edit};
pub use checkbox_enabled::CheckboxEnabled;
pub use collab_config::{CollabSettings, CollabSettingsTab, collab_config};
pub use edge_style::{EdgeStyle, EdgeStyleCtx, EdgeStyling};
pub use ext_pane::{ExtPane, ExtPaneCtx, ExtPaneEntry};
pub use gantz::{
    AlignConfig, BaseSourcesCtx, EdgeConfig, FollowSelectionConfig, Gantz, GantzState,
    GraphPlacement, GridConfig, LayoutConfig, NodeViewPane, Pane, PaneWindowGeometry,
    PaneWindowMode, SceneConfig, SnapConfig, SnapMode, StatusStyle, WindowedPane,
    close_windowed_pane, pane_key, redock_windowed_pane,
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

pub mod buffered_edit;
pub mod checkbox_enabled;
pub mod collab_config;
pub mod edge_style;
pub mod ext_pane;
pub mod gantz;
pub mod global_config;
pub mod graph_config;
pub mod graph_scene;
pub mod graph_select;
pub mod head_name_edit;
pub mod head_row;
pub mod history_view;
pub mod keybinds_config;
pub mod kit;
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
pub mod toast;
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
            debug: visuals.text_color(),
            trace: visuals.weak_text_color(),
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

/// The glyph that marks a locked node. See [`locked_text`].
pub(crate) const LOCK_GLYPH: &str = "🔒";

/// The button that discards a locked node's unrecognised settings. Returns
/// whether it was clicked. See [`crate::DropUnknownData`].
pub(crate) fn discard_button(ui: &mut egui::Ui) -> bool {
    ui.button("discard unrecognised settings")
        .on_hover_text("rewrite this node as this gantz writes it. Undo restores the settings")
        .clicked()
}

/// The warning line that tells why a node is locked. See [`locked_text`].
pub(crate) fn locked_label(ui: &mut egui::Ui, lost: &[String]) {
    let text = format!("{LOCK_GLYPH} {}", locked_text(lost));
    ui.colored_label(ui.visuals().warn_fg_color, text);
}

/// Why a node is locked, given the data it holds that this build drops. See
/// [`crate::node::InstanceEntry::lost`].
pub(crate) fn locked_text(lost: &[String]) -> String {
    format!(
        "This node holds settings this gantz does not recognise: {}. \
         Update gantz to edit it.",
        lost.join(", "),
    )
}

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

/// A log row's message galley, wrapped at `width`. A count above one is
/// prefixed as a dim `×count`.
pub(crate) fn message_galley(
    painter: &egui::Painter,
    font: &egui::FontId,
    width: f32,
    color: egui::Color32,
    count: usize,
    message: &str,
) -> std::sync::Arc<egui::Galley> {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    if count > 1 {
        let format = TextFormat::simple(font.clone(), color.gamma_multiply(0.7));
        job.append(&format!("×{count} "), 0.0, format);
    }
    job.append(message, 0.0, TextFormat::simple(font.clone(), color));
    painter.layout_job(job)
}

/// A captured log or trace entry that can collapse consecutive duplicates
/// into a single item.
pub(crate) trait RunEntry: Sized {
    /// Whether `other` continues this entry's run of duplicates.
    fn continues(&self, other: &Self) -> bool;

    /// Merge `other`, which continues this entry's run.
    fn merge(&mut self, other: Self);
}

/// Push `entry` onto the back of `entries`, merging it into the consecutive
/// run of duplicates at the back. The oldest entries are evicted while the
/// queue exceeds `max_entries`.
pub(crate) fn push_entry<T: RunEntry>(entries: &mut VecDeque<T>, max_entries: usize, entry: T) {
    if let Some(last) = entries.back_mut()
        && last.continues(&entry)
    {
        last.merge(entry);
        return;
    }
    entries.push_back(entry);
    while entries.len() > max_entries {
        entries.pop_front();
    }
}

/// A titled, full-width group card. Settings tabs use it to separate
/// sections. See [`kit::card`].
pub fn section<R>(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    kit::card(ui, Some(title), None, add_contents)
}

#[cfg(test)]
mod tests {
    use super::{RunEntry, push_entry};
    use std::collections::VecDeque;

    #[derive(Clone, Debug, PartialEq)]
    struct Entry {
        message: &'static str,
        count: usize,
    }

    impl RunEntry for Entry {
        fn continues(&self, other: &Self) -> bool {
            self.message == other.message
        }

        fn merge(&mut self, other: Self) {
            self.count += other.count;
        }
    }

    fn entry(message: &'static str) -> Entry {
        Entry { message, count: 1 }
    }

    #[test]
    fn push_entry_merges_only_consecutive_duplicates() {
        let mut entries = VecDeque::new();
        push_entry(&mut entries, 1000, entry("a"));
        push_entry(&mut entries, 1000, entry("a"));
        push_entry(&mut entries, 1000, entry("b"));
        push_entry(&mut entries, 1000, entry("a"));
        let entries: Vec<_> = entries.into_iter().collect();
        assert_eq!(
            entries,
            vec![
                Entry {
                    message: "a",
                    count: 2,
                },
                Entry {
                    message: "b",
                    count: 1,
                },
                Entry {
                    message: "a",
                    count: 1,
                },
            ]
        );
    }

    #[test]
    fn push_entry_merges_duplicates_without_evicting() {
        let mut entries = VecDeque::new();
        push_entry(&mut entries, 2, entry("first"));
        for _ in 0..100 {
            push_entry(&mut entries, 2, entry("repeated"));
        }
        let entries: Vec<_> = entries.into_iter().collect();
        assert_eq!(
            entries,
            vec![
                Entry {
                    message: "first",
                    count: 1,
                },
                Entry {
                    message: "repeated",
                    count: 100,
                },
            ]
        );
    }
}
