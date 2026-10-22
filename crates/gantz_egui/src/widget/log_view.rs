use egui_extras::{Column, TableBuilder};
use gantz_core::node;
use log::{Level, Metadata, Record};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
use web_time::SystemTime;

/// A table presenting the log entries.
pub struct LogView<'a> {
    logger: Logger,
    id: egui::Id,
    /// Labels for entries whose target identifies an emitting node, keyed by
    /// node path. See `gantz_io::log::log_target`.
    node_labels: Option<&'a HashMap<Vec<node::Id>, String>>,
}

/// The response returned by [`LogView::show`].
#[derive(Default)]
pub struct LogViewResponse {
    /// The node path of a clicked gantz-target entry, for navigation.
    pub clicked_path: Option<Vec<node::Id>>,
}

// State that needs to persist between frames.
#[derive(Clone)]
struct LogViewState {
    level_filter: log::LevelFilter,
    /// Substring filter over each entry's message and target. All
    /// space-separated terms must match.
    text_filter: String,
    auto_scroll: bool,
    /// Whether the Time, Level and Target columns are shown. Target defaults
    /// off since it is the least useful column for most entries.
    show_time: bool,
    show_level: bool,
    show_target: bool,
}

#[derive(Clone)]
pub struct LogEntry {
    pub level: Level,
    pub message: String,
    pub target: String,
    pub timestamp: SystemTime,
    /// The number of consecutive occurrences of this entry.
    pub count: usize,
}

impl LogEntry {
    fn system_time(&self) -> std::time::SystemTime {
        crate::system_time_from_web(self.timestamp).expect("failed to convert")
    }

    fn freshness(&self) -> f32 {
        let now = SystemTime::now();
        if let Ok(duration) = now.duration_since(self.timestamp) {
            if duration.as_secs_f32() >= 1.0 {
                return 0.0;
            }
            return (1.0 - duration.as_secs_f32()).powf(3.0);
        }
        0.0
    }
}

impl crate::widget::RunEntry for LogEntry {
    fn continues(&self, other: &Self) -> bool {
        self.level == other.level && self.message == other.message && self.target == other.target
    }

    fn merge(&mut self, other: Self) {
        self.count += other.count;
        self.timestamp = other.timestamp;
    }
}

#[derive(Clone)]
pub struct Logger {
    entries: Arc<Mutex<VecDeque<LogEntry>>>,
    max_entries: usize,
}

impl Logger {
    pub const DEFAULT_MAX_ENTRIES: usize = 1_000;

    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(Mutex::new(VecDeque::new())),
            max_entries,
        }
    }

    pub fn get_entries(&self) -> Vec<LogEntry> {
        self.entries.lock().unwrap().iter().cloned().collect()
    }

    /// Call `f` with the entries, oldest first, without cloning them.
    pub fn with_entries<R>(&self, f: impl FnOnce(&VecDeque<LogEntry>) -> R) -> R {
        f(&self.entries.lock().unwrap())
    }

    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }
}

impl Default for Logger {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_ENTRIES)
    }
}

impl log::Log for Logger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let entry = LogEntry {
            level: record.level(),
            message: record.args().to_string(),
            target: record.target().to_string(),
            timestamp: SystemTime::now(),
            count: 1,
        };

        let mut entries = self.entries.lock().unwrap();
        crate::widget::push_entry(&mut entries, self.max_entries, entry);
    }

    fn flush(&self) {}
}

impl<'a> LogView<'a> {
    pub fn new(id: egui::Id, logger: Logger) -> Self {
        Self {
            logger,
            id,
            node_labels: None,
        }
    }

    /// Provide node labels for gantz-target entries. Their target cells then
    /// show `label (path)` and become clickable for navigation.
    pub fn node_labels(mut self, labels: &'a HashMap<Vec<node::Id>, String>) -> Self {
        self.node_labels = Some(labels);
        self
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> LogViewResponse {
        let mut response = LogViewResponse::default();
        let state_id = self.id.with("state");
        let mut state = ui
            .memory_mut(|mem| mem.data.get_temp::<LogViewState>(state_id))
            .unwrap_or_else(|| LogViewState {
                level_filter: log::max_level(),
                text_filter: String::new(),
                auto_scroll: true,
                show_time: true,
                show_level: true,
                show_target: false,
            });

        // A text filter, with an options button on the right that opens a popup
        // menu of view settings.
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let h = ui.spacing().interact_size.y;
                let btn = ui
                    .add_sized([h, h], egui::Button::new(crate::widget::OPTIONS_GLYPH))
                    .on_hover_text("log options");
                egui::Popup::menu(&btn)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.horizontal(|ui| {
                            ui.label("level");
                            let init = state.level_filter;
                            egui::ComboBox::from_id_salt(self.id.with("level_filter"))
                                .selected_text(format!("{}", state.level_filter))
                                .show_ui(ui, |ui| {
                                    use log::LevelFilter as L;
                                    ui.selectable_value(&mut state.level_filter, L::Off, "Off");
                                    ui.selectable_value(&mut state.level_filter, L::Error, "Error");
                                    ui.selectable_value(&mut state.level_filter, L::Warn, "Warn");
                                    ui.selectable_value(&mut state.level_filter, L::Info, "Info");
                                    ui.selectable_value(&mut state.level_filter, L::Debug, "Debug");
                                    ui.selectable_value(&mut state.level_filter, L::Trace, "Trace");
                                });
                            if init != state.level_filter {
                                log::set_max_level(state.level_filter);
                            }
                        });
                        ui.checkbox(&mut state.auto_scroll, "auto-scroll");
                        ui.separator();
                        ui.label("columns");
                        ui.checkbox(&mut state.show_time, "time");
                        ui.checkbox(&mut state.show_level, "level");
                        ui.checkbox(&mut state.show_target, "target");
                        ui.separator();
                        if ui.button("clear").clicked() {
                            self.logger.clear();
                        }
                    });
                // The filter fills the remaining width.
                egui::TextEdit::singleline(&mut state.text_filter)
                    .desired_width(ui.available_width())
                    .hint_text("filter logs")
                    .show(ui);
            });
        });

        let mut entries = self.logger.get_entries();

        entries.retain(|entry| entry.level <= state.level_filter);

        if !state.text_filter.is_empty() {
            let filter = state.text_filter.to_lowercase();
            entries.retain(|entry| {
                let hay = format!("{} {}", entry.message, entry.target).to_lowercase();
                filter.split_whitespace().all(|term| hay.contains(term))
            });
        }

        // Show the newest entries first.
        entries.reverse();

        // The Time, Level and Target columns are optional. Message is always
        // the trailing remainder column.
        let (show_time, show_level, show_target) =
            (state.show_time, state.show_level, state.show_target);
        ui.spacing_mut().item_spacing.x = 14.0;
        let mut table = TableBuilder::new(ui)
            .resizable(false)
            .cell_layout(egui::Layout::left_to_right(egui::Align::TOP));
        if show_time {
            table = table.column(Column::auto()); // Timestamp
        }
        if show_level {
            table = table.column(Column::auto().at_least(50.0)); // Level
        }
        if show_target {
            table = table.column(Column::auto().at_least(80.0)); // Target
        }
        table
            .column(Column::remainder().at_least(100.0).clip(true)) // Message
            .body(|mut body| {
                let row_h = 18.0;
                let text_color = body.ui_mut().style().visuals.text_color();
                let strong = body.ui_mut().style().visuals.strong_text_color();
                let level_colors =
                    crate::widget::LevelColors::from_visuals(&body.ui_mut().style().visuals);
                // Each message takes one line, cut at its column's width. Its
                // galley is rendered as-is.
                let msg_w = body.widths().last().copied().unwrap_or(0.0);
                let font = egui::TextStyle::Body.resolve(body.ui_mut().style());
                let rows: Vec<_> = entries
                    .iter()
                    .map(|entry| {
                        let color = text_color.lerp_to_gamma(strong, entry.freshness());
                        let painter = body.ui_mut().painter();
                        let galley = crate::widget::message_galley(
                            painter,
                            &font,
                            msg_w,
                            color,
                            entry.count,
                            &entry.message,
                        );
                        (color, galley)
                    })
                    .collect();
                let heights = rows.iter().map(|(_, galley)| galley.size().y.max(row_h));
                body.heterogeneous_rows(heights, |mut row| {
                    let (text_color, galley) = &rows[row.index()];
                    let text_color = *text_color;
                    let entry = &entries[row.index()];
                    let count = entry.count;

                    if show_time {
                        row.col(|ui| {
                            let t = entry.system_time();
                            ui.weak(crate::widget::format_local_time(t))
                                .on_hover_text(crate::widget::format_local_datetime(t));
                        });
                    }

                    if show_level {
                        row.col(|ui| {
                            let (color, text) = match entry.level {
                                Level::Error => (level_colors.error, "error"),
                                Level::Warn => (level_colors.warn, "warn"),
                                Level::Info => (level_colors.info, "info"),
                                Level::Debug => (level_colors.debug, "debug"),
                                Level::Trace => (level_colors.trace, "trace"),
                            };
                            ui.colored_label(color, text);
                        });
                    }

                    if show_target {
                        row.col(|ui| {
                            let node_path = gantz_io::log::parse_log_target(&entry.target);
                            match node_path {
                                Some(path) => {
                                    let label = self
                                        .node_labels
                                        .and_then(|labels| labels.get(&path))
                                        .map(String::as_str);
                                    let ids: Vec<String> =
                                        path.iter().map(ToString::to_string).collect();
                                    let text = match label {
                                        Some(label) => format!("{label} ({})", ids.join(":")),
                                        None => ids.join(":"),
                                    };
                                    if ui.link(text).on_hover_text("select node").clicked() {
                                        response.clicked_path = Some(path);
                                    }
                                }
                                None => {
                                    ui.colored_label(text_color, &entry.target);
                                }
                            }
                        });
                    }

                    row.col(|ui| {
                        let res = ui.add(egui::Label::new(galley.clone()));
                        res.on_hover_text(match count > 1 {
                            true => format!("{}\n\n{count} in a row", entry.message),
                            false => entry.message.clone(),
                        });
                    });

                    if entry.freshness() > 0.0 {
                        row.response().ctx.request_repaint();
                    }
                });
            });

        if state.auto_scroll {
            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
        }

        ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_message(logger: &Logger, message: &str) {
        log::Log::log(
            logger,
            &log::Record::builder()
                .args(format_args!("{message}"))
                .level(Level::Info)
                .target("test")
                .build(),
        );
    }

    #[test]
    fn merges_consecutive_duplicates_into_one_entry() {
        let logger = Logger::new(1000);
        log_message(&logger, "a");
        log_message(&logger, "a");
        log_message(&logger, "b");
        log_message(&logger, "a");
        let entries: Vec<_> = logger
            .get_entries()
            .iter()
            .map(|e| (e.message.clone(), e.count))
            .collect();
        assert_eq!(
            entries,
            [
                ("a".to_string(), 2),
                ("b".to_string(), 1),
                ("a".to_string(), 1),
            ]
        );
    }

    #[test]
    fn duplicates_do_not_consume_the_message_window() {
        let logger = Logger::new(2);
        log_message(&logger, "first");
        for _ in 0..100 {
            log_message(&logger, "repeated");
        }
        let entries = logger.get_entries();
        let messages: Vec<_> = entries.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(messages, ["first", "repeated"]);
        assert_eq!(entries[1].count, 100);
    }
}
