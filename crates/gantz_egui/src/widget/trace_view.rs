use egui_extras::{Column, TableBuilder};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tracing::{Level, level_filters::LevelFilter};
use tracing_subscriber::Layer;
use web_time::SystemTime;

/// A table presenting traces captured from tracing
pub struct TraceView {
    capture: TraceCapture,
    id: egui::Id,
    level: LevelFilter,
}

// State that needs to persist between frames.
#[derive(Clone)]
struct TraceViewState {
    target_filter: String,
    auto_scroll: bool,
}

#[derive(Clone)]
pub struct TraceEntry {
    pub level: Level,
    pub message: String,
    pub target: String,
    pub timestamp: SystemTime,
}

#[derive(Clone)]
pub struct TraceCapture {
    entries: Arc<Mutex<VecDeque<TraceEntry>>>,
    max_entries: usize,
}

pub struct TraceCaptureLayer<S> {
    capture: TraceCapture,
    _subscriber: std::marker::PhantomData<S>,
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl TraceEntry {
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

impl TraceCapture {
    pub const DEFAULT_MAX_ENTRIES: usize = 1_000;

    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(Mutex::new(VecDeque::new())),
            max_entries,
        }
    }

    pub fn get_entries(&self) -> Vec<TraceEntry> {
        self.entries.lock().unwrap().iter().cloned().collect()
    }

    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }

    /// Create a tracing layer that captures traces into this TraceCapture
    pub fn layer<S>(self) -> TraceCaptureLayer<S>
    where
        S: tracing::Subscriber,
    {
        TraceCaptureLayer {
            capture: self,
            _subscriber: std::marker::PhantomData,
        }
    }

    fn add_entry(&self, entry: TraceEntry) {
        let mut entries = self.entries.lock().unwrap();
        entries.push_back(entry);

        while entries.len() > self.max_entries {
            entries.pop_front();
        }
    }
}

impl TraceView {
    pub fn new(id: egui::Id, capture: TraceCapture, level: LevelFilter) -> Self {
        Self { capture, id, level }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        // Get or initialize our state from memory
        let state_id = self.id.with("state");
        let mut state = ui
            .memory_mut(|mem| mem.data.get_temp::<TraceViewState>(state_id))
            .unwrap_or_else(|| TraceViewState {
                target_filter: String::new(),
                auto_scroll: true,
            });

        // Controls
        ui.horizontal(|ui| {
            ui.label("Level:");
            let level_label = egui::Label::new(format!("{}", self.level)).selectable(false);
            let level_response = ui.add(level_label);
            #[cfg(not(target_arch = "wasm32"))]
            level_response.on_hover_text("Adjust with RUST_LOG env var at startup");

            ui.separator();
            ui.checkbox(&mut state.auto_scroll, "Auto-scroll");

            ui.separator();
            if ui.button("Clear").clicked() {
                self.capture.clear();
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Target:");
            ui.text_edit_singleline(&mut state.target_filter);
        });

        ui.separator();

        // Get and filter entries
        let mut entries = self.capture.get_entries();

        // Filter by level
        entries.retain(|entry| match self.level {
            LevelFilter::OFF => false,
            LevelFilter::ERROR => entry.level <= Level::ERROR,
            LevelFilter::WARN => entry.level <= Level::WARN,
            LevelFilter::INFO => entry.level <= Level::INFO,
            LevelFilter::DEBUG => entry.level <= Level::DEBUG,
            LevelFilter::TRACE => true,
        });

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
                let row_h = 18.0;
                let n_rows = entries.len();
                let text_color = body.ui_mut().style().visuals.text_color();
                body.rows(row_h, n_rows, |mut row| {
                    let entry = &entries[row.index()];
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
                            Level::ERROR => (egui::Color32::from_rgb(255, 100, 100), "ERROR"),
                            Level::WARN => (egui::Color32::from_rgb(255, 200, 100), "WARN"),
                            Level::INFO => (egui::Color32::from_rgb(100, 200, 255), "INFO"),
                            Level::DEBUG => (egui::Color32::GRAY, "DEBUG"),
                            Level::TRACE => (egui::Color32::DARK_GRAY, "TRACE"),
                        };
                        ui.colored_label(color, text);
                    });

                    row.col(|ui| {
                        ui.colored_label(text_color, &entry.target);
                    });

                    row.col(|ui| {
                        ui.colored_label(text_color, &entry.message);
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
    }
}

impl Default for TraceCapture {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_ENTRIES)
    }
}

impl<S> Clone for TraceCaptureLayer<S> {
    fn clone(&self) -> Self {
        Self {
            capture: self.capture.clone(),
            _subscriber: std::marker::PhantomData,
        }
    }
}

impl<S> Layer<S> for TraceCaptureLayer<S>
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();

        // Extract the message
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let entry = TraceEntry {
            level: *metadata.level(),
            message: visitor.message,
            target: metadata.target().to_string(),
            timestamp: SystemTime::now(),
        };

        self.capture.add_entry(entry);
    }
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            if !self.message.is_empty() {
                self.message.push(' ');
            }
            self.message.push_str(&format!("{:?}", value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            if !self.message.is_empty() {
                self.message.push(' ');
            }
            self.message.push_str(value);
        }
    }
}
