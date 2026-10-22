use egui_extras::{Column, TableBuilder};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tracing::{Level, level_filters::LevelFilter};
use tracing_subscriber::Layer;
use web_time::SystemTime;

/// A table presenting traces captured from tracing.
pub struct TraceView {
    capture: TraceCapture,
    id: egui::Id,
    level: LevelFilter,
}

// State that needs to persist between frames.
#[derive(Clone)]
struct TraceViewState {
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
pub struct TraceEntry {
    pub level: Level,
    pub message: String,
    pub target: String,
    pub timestamp: SystemTime,
    /// The number of consecutive occurrences of this entry.
    pub count: usize,
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
    /// The target of an event bridged from the `log` crate. The bridge gives
    /// every such event the target "log" and records the real one here.
    log_target: Option<String>,
}

impl TraceEntry {
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

    /// Call `f` with the entries, oldest first, without cloning them.
    pub fn with_entries<R>(&self, f: impl FnOnce(&VecDeque<TraceEntry>) -> R) -> R {
        f(&self.entries.lock().unwrap())
    }

    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }

    /// Create a tracing layer that captures traces into this TraceCapture.
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
        crate::widget::push_entry(&mut entries, self.max_entries, entry);
    }
}

impl crate::widget::RunEntry for TraceEntry {
    fn continues(&self, other: &Self) -> bool {
        self.level == other.level && self.message == other.message && self.target == other.target
    }

    fn merge(&mut self, other: Self) {
        self.count += other.count;
        self.timestamp = other.timestamp;
    }
}

impl TraceView {
    pub fn new(id: egui::Id, capture: TraceCapture, level: LevelFilter) -> Self {
        Self { capture, id, level }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let state_id = self.id.with("state");
        let mut state = ui
            .memory_mut(|mem| mem.data.get_temp::<TraceViewState>(state_id))
            .unwrap_or_else(|| TraceViewState {
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
                    .on_hover_text("trace options");
                egui::Popup::menu(&btn)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        // The level is fixed at startup by the `RUST_LOG` filter.
                        #[allow(unused_variables)]
                        let level_response = ui.label(format!("level {}", self.level));
                        #[cfg(not(target_arch = "wasm32"))]
                        level_response.on_hover_text("adjust with RUST_LOG env var at startup");
                        ui.checkbox(&mut state.auto_scroll, "auto-scroll");
                        ui.separator();
                        ui.label("columns");
                        ui.checkbox(&mut state.show_time, "time");
                        ui.checkbox(&mut state.show_level, "level");
                        ui.checkbox(&mut state.show_target, "target");
                        ui.separator();
                        if ui.button("clear").clicked() {
                            self.capture.clear();
                        }
                    });
                // The filter fills the remaining width.
                super::kit::field_visuals(ui);
                egui::TextEdit::singleline(&mut state.text_filter)
                    .desired_width(ui.available_width())
                    .hint_text("filter logs")
                    .margin(super::kit::FIELD_MARGIN)
                    .show(ui);
            });
        });

        let mut entries = self.capture.get_entries();

        entries.retain(|entry| match self.level {
            LevelFilter::OFF => false,
            LevelFilter::ERROR => entry.level <= Level::ERROR,
            LevelFilter::WARN => entry.level <= Level::WARN,
            LevelFilter::INFO => entry.level <= Level::INFO,
            LevelFilter::DEBUG => entry.level <= Level::DEBUG,
            LevelFilter::TRACE => true,
        });

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
            table = table.column(Column::auto()); // Level
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
                                Level::ERROR => (level_colors.error, "error"),
                                Level::WARN => (level_colors.warn, "warn"),
                                Level::INFO => (level_colors.info, "info"),
                                Level::DEBUG => (level_colors.debug, "debug"),
                                Level::TRACE => (level_colors.trace, "trace"),
                            };
                            ui.colored_label(color, text);
                        });
                    }

                    if show_target {
                        row.col(|ui| {
                            ui.colored_label(text_color, &entry.target);
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

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let entry = TraceEntry {
            level: *metadata.level(),
            message: visitor.message,
            target: visitor
                .log_target
                .unwrap_or_else(|| metadata.target().to_string()),
            timestamp: SystemTime::now(),
            count: 1,
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
        match field.name() {
            "message" => {
                if !self.message.is_empty() {
                    self.message.push(' ');
                }
                self.message.push_str(value);
            }
            "log.target" => self.log_target = Some(value.to_string()),
            _ => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(message: &str) -> TraceEntry {
        TraceEntry {
            level: Level::INFO,
            message: message.to_string(),
            target: "test".to_string(),
            timestamp: SystemTime::now(),
            count: 1,
        }
    }

    #[test]
    fn merges_consecutive_duplicates_into_one_entry() {
        let capture = TraceCapture::new(1000);
        capture.add_entry(entry("a"));
        capture.add_entry(entry("a"));
        capture.add_entry(entry("b"));
        capture.add_entry(entry("a"));
        let entries: Vec<_> = capture
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
        let capture = TraceCapture::new(2);
        capture.add_entry(entry("first"));
        for _ in 0..100 {
            capture.add_entry(entry("repeated"));
        }
        let entries = capture.get_entries();
        let messages: Vec<_> = entries.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(messages, ["first", "repeated"]);
        assert_eq!(entries[1].count, 100);
    }
}
