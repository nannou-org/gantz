//! The status line: the focused graph, its session, the VM's time per update
//! and the count of problems in the log. It sits in a gutter under the panes,
//! or in a pill over the scene's bottom-right corner, or is hidden. See
//! [`StatusStyle`].
//!
//! New warnings and errors in the log also become toasts.

use super::LogSource;
use crate::{
    collab::SessionDisplay,
    style::{self, Role},
    widget::{PerfCapture, status_dot, toast},
};
use web_time::SystemTime;

/// Where the status line shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum StatusStyle {
    /// A line in a gutter under the panes, on the scene.
    #[default]
    Gutter,
    /// A pill over the bottom-right corner of the scene.
    Pill,
    /// No status line. Toasts still show.
    Hidden,
}

/// What the status line shows.
pub(super) struct Status<'a> {
    /// The focused graph's name.
    pub graph: Option<String>,
    /// The focused graph's collaborative session, if it has one.
    pub session: Option<&'a SessionDisplay>,
    /// The mean VM time per update over the recent samples.
    pub vm: Option<std::time::Duration>,
    pub problems: Problems,
}

/// The warnings and errors in the log that come from gantz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Problems {
    pub warnings: usize,
    pub errors: usize,
}

/// One measure on the right of the status line.
enum Measure {
    Vm(std::time::Duration),
    /// A problem count, as text, in its severity's colour.
    Count(String, egui::Color32),
}

/// The height of the gutter that holds the status line.
pub(super) const GUTTER_HEIGHT: f32 = 24.0;

/// The id under which the context keeps the timestamp of the newest log
/// entry already seen, so that only newer entries become toasts.
const SEEN_ID: &str = "gantz-status-seen";

/// The number of recent VM samples the status line averages.
const VM_SAMPLES: usize = 30;

/// Count the problems in the log and push a toast for each warning or error
/// that is newer than the last call.
pub(super) fn scan_log(ctx: &egui::Context, source: Option<&LogSource>) -> Problems {
    let Some(source) = source else {
        return Problems::default();
    };
    let id = egui::Id::new(SEEN_ID);
    let seen: Option<SystemTime> = ctx.data(|d| d.get_temp(id));
    let mut problems = Problems::default();
    let mut newest = seen;
    let mut fresh = vec![];
    let mut visit = |severity: Option<toast::Severity>,
                     target: &str,
                     message: &str,
                     timestamp: SystemTime,
                     count: usize| {
        let Some(severity) = severity.filter(|_| target.contains("gantz")) else {
            return;
        };
        match severity {
            toast::Severity::Error => problems.errors += count,
            _ => problems.warnings += count,
        }
        if seen.is_none_or(|seen| timestamp > seen) {
            fresh.push((severity, message.to_string()));
        }
        newest = newest.max(Some(timestamp));
    };
    match source {
        LogSource::Logger(logger) => logger.with_entries(|entries| {
            for e in entries {
                let severity = match e.level {
                    log::Level::Error => Some(toast::Severity::Error),
                    log::Level::Warn => Some(toast::Severity::Warning),
                    _ => None,
                };
                visit(severity, &e.target, &e.message, e.timestamp, e.count);
            }
        }),
        #[cfg(feature = "tracing")]
        LogSource::TraceCapture(capture, _) => capture.with_entries(|entries| {
            for e in entries {
                let severity = match e.level {
                    tracing::Level::ERROR => Some(toast::Severity::Error),
                    tracing::Level::WARN => Some(toast::Severity::Warning),
                    _ => None,
                };
                visit(severity, &e.target, &e.message, e.timestamp, e.count);
            }
        }),
    }
    for (severity, message) in fresh {
        toast::push(ctx, severity, message);
    }
    if let Some(newest) = newest {
        ctx.data_mut(|d| d.insert_temp(id, newest));
    }
    problems
}

/// The mean of the most recent VM samples.
pub(super) fn vm_mean(capture: &PerfCapture) -> Option<std::time::Duration> {
    let samples = capture.samples();
    let n = samples.len().min(VM_SAMPLES);
    let sum: std::time::Duration = samples.iter().rev().take(n).sum();
    u32::try_from(n).ok().filter(|&n| n > 0).map(|n| sum / n)
}

/// Draw the status line in the gutter `rect`. Returns whether the problem
/// counts were clicked.
pub(super) fn gutter(ui: &mut egui::Ui, rect: egui::Rect, status: &Status) -> bool {
    let builder = egui::UiBuilder::new()
        .max_rect(rect)
        .layout(egui::Layout::left_to_right(egui::Align::Center));
    ui.scope_builder(builder, |ui| {
        spaced(ui);
        graph_ui(ui, status);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            measures_ui(ui, status, true)
        })
        .inner
    })
    .inner
}

/// Draw the status line in a pill whose bottom-right corner is `anchor`.
/// Returns whether the problem counts were clicked.
pub(super) fn pill(ctx: &egui::Context, anchor: egui::Pos2, status: &Status) -> bool {
    egui::Area::new(egui::Id::new("gantz-status-pill"))
        .order(egui::Order::Middle)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(anchor)
        .show(ctx, |ui| {
            let height = ui.spacing().interact_size.y;
            egui::Frame::new()
                .fill(style::color(ui, Role::Panel))
                .corner_radius(style::shape(ctx).widget_corner(height))
                .inner_margin(egui::Margin::symmetric(10, 0))
                .show(ui, |ui| {
                    ui.set_height(height);
                    ui.horizontal_centered(|ui| {
                        spaced(ui);
                        graph_ui(ui, status);
                        measures_ui(ui, status, false)
                    })
                    .inner
                })
                .inner
        })
        .inner
}

/// Space the items of the status line a little wider than usual.
fn spaced(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.x = 8.0;
}

/// The focused graph and its session.
fn graph_ui(ui: &mut egui::Ui, status: &Status) {
    if let Some(graph) = &status.graph {
        ui.label(egui::RichText::new(graph).color(ui.visuals().text_color()));
    }
    if let Some(session) = status.session {
        let color = style::color(ui, session.conn.role());
        status_dot(ui, color).on_hover_text(session.hover_text());
        let peers = session.peers.len();
        let text = match peers {
            0 => session.conn.label().to_string(),
            1 => format!("{} · 1 peer", session.conn.label()),
            n => format!("{} · {n} peers", session.conn.label()),
        };
        ui.weak(text);
    }
}

/// The VM time and the problem counts. In a right-to-left layout `rtl` must
/// be set, so that they still read left to right. Returns whether the
/// problem counts were clicked.
fn measures_ui(ui: &mut egui::Ui, status: &Status, rtl: bool) -> bool {
    let Problems { warnings, errors } = status.problems;
    let visuals = ui.visuals();
    let counts = [
        (warnings, "warning", "warnings", visuals.warn_fg_color),
        (errors, "error", "errors", visuals.error_fg_color),
    ];
    let mut items: Vec<Measure> = status.vm.map(Measure::Vm).into_iter().collect();
    for (n, one, many, color) in counts.into_iter().filter(|c| c.0 > 0) {
        let noun = if n == 1 { one } else { many };
        items.push(Measure::Count(format!("{n} {noun}"), color));
    }
    if rtl {
        items.reverse();
    }
    let mut clicked = false;
    for item in items {
        match item {
            Measure::Vm(vm) => {
                let ms = vm.as_secs_f64() * 1_000.0;
                ui.weak(format!("vm {ms:.2} ms"))
                    .on_hover_text("the mean time of the recent VM updates");
            }
            Measure::Count(text, color) => {
                let count = |ui: &mut egui::Ui| {
                    let text = egui::RichText::new(&text).color(color);
                    ui.add(egui::Label::new(text).sense(egui::Sense::click()))
                        .on_hover_text("open the logs")
                        .clicked()
                };
                if rtl {
                    clicked |= count(ui);
                    status_dot(ui, color);
                } else {
                    status_dot(ui, color);
                    clicked |= count(ui);
                }
            }
        }
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::log_view::Logger;
    use log::Log;

    fn log(logger: &Logger, level: log::Level, target: &str, message: &str) {
        let args = format_args!("{message}");
        let record = log::Record::builder()
            .level(level)
            .target(target)
            .args(args)
            .build();
        logger.log(&record);
    }

    #[test]
    fn scan_log_counts_only_gantz_problems() {
        let logger = Logger::new(100);
        log(&logger, log::Level::Warn, "gantz:1", "a");
        log(&logger, log::Level::Warn, "gantz:1", "a");
        log(&logger, log::Level::Error, "bevy_gantz::vm", "b");
        log(&logger, log::Level::Warn, "wgpu_core", "c");
        log(&logger, log::Level::Info, "gantz", "d");
        let source = LogSource::Logger(logger);
        let ctx = egui::Context::default();
        let problems = scan_log(&ctx, Some(&source));
        let expected = Problems {
            warnings: 2,
            errors: 1,
        };
        assert_eq!(problems, expected);
        // A second scan finds the same problems.
        assert_eq!(scan_log(&ctx, Some(&source)), expected);
    }

    #[test]
    fn vm_mean_averages_the_recent_samples() {
        let mut capture = PerfCapture::default();
        assert_eq!(vm_mean(&capture), None);
        for ms in [2, 4] {
            capture.record(std::time::Duration::from_millis(ms));
        }
        assert_eq!(vm_mean(&capture), Some(std::time::Duration::from_millis(3)));
    }
}
