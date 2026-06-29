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
    /// Labels for entries whose target identifies an emitting node (see
    /// `gantz_std::log::log_target`), keyed by node path.
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
    target_filter: String,
    auto_scroll: bool,
}

#[derive(Clone)]
pub struct LogEntry {
    pub level: Level,
    pub message: String,
    pub target: String,
    pub timestamp: SystemTime,
}

impl LogEntry {
    fn format_timestamp(&self) -> String {
        let system_time = crate::system_time_from_web(self.timestamp).expect("failed to convert");
        crate::widget::format_local_datetime(system_time)
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
        };

        let mut entries = self.entries.lock().unwrap();
        entries.push_back(entry);

        while entries.len() > self.max_entries {
            entries.pop_front();
        }
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

    /// Provide node labels for gantz-target entries; their target cells
    /// then show `label (path)` and become clickable for navigation.
    pub fn node_labels(mut self, labels: &'a HashMap<Vec<node::Id>, String>) -> Self {
        self.node_labels = Some(labels);
        self
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> LogViewResponse {
        let mut response = LogViewResponse::default();
        // Get or initialize our state from memory
        let state_id = self.id.with("state");
        let mut state = ui
            .memory_mut(|mem| mem.data.get_temp::<LogViewState>(state_id))
            .unwrap_or_else(|| LogViewState {
                level_filter: log::max_level(),
                target_filter: String::new(),
                auto_scroll: true,
            });

        // Controls
        ui.horizontal(|ui| {
            ui.label("Level:");
            let init_filter = state.level_filter.clone();
            egui::ComboBox::from_id_salt(self.id.with("level_filter"))
                .selected_text(format!("{}", state.level_filter))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut state.level_filter, log::LevelFilter::Off, "Off");
                    ui.selectable_value(&mut state.level_filter, log::LevelFilter::Error, "Error");
                    ui.selectable_value(&mut state.level_filter, log::LevelFilter::Warn, "Warn");
                    ui.selectable_value(&mut state.level_filter, log::LevelFilter::Info, "Info");
                    ui.selectable_value(&mut state.level_filter, log::LevelFilter::Debug, "Debug");
                    ui.selectable_value(&mut state.level_filter, log::LevelFilter::Trace, "Trace");
                });
            if init_filter != state.level_filter {
                log::set_max_level(state.level_filter);
            }

            ui.separator();
            ui.checkbox(&mut state.auto_scroll, "Auto-scroll");

            ui.separator();
            if ui.button("Clear").clicked() {
                self.logger.clear();
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Target:");
            ui.text_edit_singleline(&mut state.target_filter);
        });

        ui.separator();

        // Get and filter entries
        let mut entries = self.logger.get_entries();

        entries.retain(|entry| entry.level <= state.level_filter);

        if !state.target_filter.is_empty() {
            let filter = state.target_filter.to_lowercase();
            entries.retain(|entry| entry.target.to_lowercase().contains(&filter));
        }

        entries.reverse();

        // Collapse runs of consecutive entries that differ only by their
        // timestamp into a single row, carrying an occurrence count.
        let runs = crate::widget::group_runs(&entries, |a, b| {
            a.level == b.level && a.message == b.message && a.target == b.target
        });

        // Create table
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .column(Column::auto().at_least(80.0)) // Timestamp
            .column(Column::auto().at_least(50.0)) // Level
            .column(Column::auto().at_least(80.0)) // Target
            .column(Column::remainder().at_least(100.0)) // Message
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Time");
                });
                header.col(|ui| {
                    ui.strong("Level");
                });
                header.col(|ui| {
                    ui.strong("Target");
                });
                header.col(|ui| {
                    ui.strong("Message");
                });
            })
            .body(|mut body| {
                let row_h = 18.0;
                let n_rows = runs.len();
                let text_color = body.ui_mut().style().visuals.text_color();
                body.rows(row_h, n_rows, |mut row| {
                    let (idx, count) = runs[row.index()];
                    let entry = &entries[idx];
                    let freshness = entry.freshness();
                    let fresh = freshness > 0.0;
                    let text_color = if fresh {
                        let hl_col = egui::Color32::WHITE;
                        text_color.lerp_to_gamma(hl_col, freshness)
                    } else {
                        text_color
                    };

                    row.col(|ui| {
                        let text = entry.format_timestamp();
                        ui.colored_label(text_color, text);
                    });

                    row.col(|ui| {
                        let (color, text) = match entry.level {
                            Level::Error => (egui::Color32::from_rgb(255, 100, 100), "ERROR"),
                            Level::Warn => (egui::Color32::from_rgb(255, 200, 100), "WARN"),
                            Level::Info => (egui::Color32::from_rgb(100, 200, 255), "INFO"),
                            Level::Debug => (egui::Color32::GRAY, "DEBUG"),
                            Level::Trace => (egui::Color32::DARK_GRAY, "TRACE"),
                        };
                        ui.colored_label(color, text);
                    });

                    row.col(|ui| {
                        let node_path = gantz_std::log::parse_log_target(&entry.target);
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

                    row.col(|ui| {
                        ui.horizontal(|ui| {
                            if count > 1 {
                                ui.colored_label(
                                    text_color.gamma_multiply(0.7),
                                    format!("×{count}"),
                                )
                                .on_hover_text("occurrences collapsed (repeated log)");
                            }
                            ui.colored_label(text_color, &entry.message);
                        });
                    });

                    if fresh {
                        row.response().ctx.request_repaint();
                    }
                });
            });

        if state.auto_scroll {
            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
        }

        // Store the modified state back in memory
        ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));

        response
    }
}
