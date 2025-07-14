use egui_extras::{Column, TableBuilder};
use log::{Level, Metadata, Record};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::SystemTime,
};

/// A table presenting the
pub struct LogView {
    logger: Logger,
    id: egui::Id,
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
        humantime::format_rfc3339_seconds(self.timestamp).to_string()
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

impl LogView {
    pub fn new(id: egui::Id, logger: Logger) -> Self {
        Self { logger, id }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
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
                for entry in entries {
                    let freshness = entry.freshness();
                    let text_color = if freshness > 0.0 {
                        let col = body.ui_mut().style().visuals.text_color();
                        let hl_col = egui::Color32::WHITE;
                        body.ui_mut().ctx().request_repaint();
                        col.lerp_to_gamma(hl_col, freshness)
                    } else {
                        body.ui_mut().style().visuals.text_color()
                    };
                    body.row(18.0, |mut row| {
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
                            ui.colored_label(text_color, &entry.target);
                        });

                        row.col(|ui| {
                            ui.colored_label(text_color, &entry.message);
                        });
                    });
                }
            });

        if state.auto_scroll {
            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
        }

        // Store the modified state back in memory
        ui.memory_mut(|mem| mem.data.insert_temp(state_id, state));
    }
}
