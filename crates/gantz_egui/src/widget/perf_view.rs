//! Simple performance monitoring widgets for VM execution and GUI frame times.

use std::collections::VecDeque;
use std::time::Duration;

const DEFAULT_MAX_SAMPLES: usize = 500;

/// Format a millisecond value as a Duration debug string (e.g. "231µs", "1.2ms").
fn format_ms(ms: f64) -> String {
    let duration = Duration::from_secs_f64(ms.max(0.0) / 1000.0);
    format!("{duration:?}")
}

/// Capture of timing samples. All on main thread, no sync needed.
pub struct PerfCapture {
    samples: VecDeque<Duration>,
    max_samples: usize,
}

impl Default for PerfCapture {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_SAMPLES)
    }
}

impl PerfCapture {
    /// Create a new capture with the given max sample count.
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(max_samples),
            max_samples,
        }
    }

    /// Record a new timing sample.
    pub fn record(&mut self, duration: Duration) {
        if self.samples.len() >= self.max_samples {
            self.samples.pop_front();
        }
        self.samples.push_back(duration);
    }

    /// Get the samples.
    pub fn samples(&self) -> &VecDeque<Duration> {
        &self.samples
    }

    /// Get the max samples setting.
    pub fn max_samples(&self) -> usize {
        self.max_samples
    }

    /// Set the max samples. Truncates if necessary.
    pub fn set_max_samples(&mut self, max: usize) {
        self.max_samples = max;
        while self.samples.len() > max {
            self.samples.pop_front();
        }
    }
}

/// Minimal widget for displaying performance data as a plot.
pub struct PerfView<'a> {
    id: egui::Id,
    title: &'a str,
    capture: &'a mut PerfCapture,
}

impl<'a> PerfView<'a> {
    pub fn new(title: &'a str, capture: &'a mut PerfCapture) -> Self {
        let id = egui::Id::new(title);
        Self { id, title, capture }
    }

    pub fn with_id(mut self, id: egui::Id) -> Self {
        self.id = id;
        self
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        // Load state for context menu.
        let state_id = self.id.with("state");
        let mut max_samples = ui
            .memory(|m| m.data.get_temp::<usize>(state_id))
            .unwrap_or(self.capture.max_samples());

        // Get text color for the line.
        let text_color = ui.visuals().text_color();

        // Convert samples to plot points (index, duration_ms).
        let points: egui_plot::PlotPoints = self
            .capture
            .samples()
            .iter()
            .enumerate()
            .map(|(i, d)| [i as f64, d.as_secs_f64() * 1000.0])
            .collect();

        let line = egui_plot::Line::new(self.title, points)
            .color(text_color)
            .fill(0.0)
            .fill_alpha(0.3);

        let plot = egui_plot::Plot::new(self.id)
            .height(ui.available_height())
            .width(ui.available_width())
            .allow_boxed_zoom(false)
            .allow_drag(false)
            .allow_scroll(false)
            .show_axes(false)
            .show_grid(false)
            .set_margin_fraction(egui::Vec2::ZERO)
            .include_y(0.0)
            .label_formatter(|_, point| format_ms(point.y));

        ui.scope(|ui| {
            // Remove the 1px border around the plot background.
            ui.style_mut().visuals.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
            // Make the plot background blend-in with the pane background.
            ui.style_mut().visuals.extreme_bg_color = ui.style().visuals.window_fill;

            let plot_response = plot.show(ui, |plot_ui| {
                plot_ui.line(line);
            });

            // Context menu for configuration.
            plot_response.response.context_menu(|ui| {
                ui.label("Max Samples");
                ui.horizontal(|ui| {
                    for &n in &[100, 250, 500, 1000, 2000] {
                        if ui
                            .selectable_label(max_samples == n, format!("{n}"))
                            .clicked()
                        {
                            max_samples = n;
                            self.capture.set_max_samples(n);
                            ui.close();
                        }
                    }
                });
            });
        });

        // Store state.
        ui.memory_mut(|m| m.data.insert_temp(state_id, max_samples));
    }
}
