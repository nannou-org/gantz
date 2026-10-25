//! The status line: the focused graph and when it was saved, its session or
//! the vault link, the host's items such as the audio device, the VM's time
//! per update and the count of problems in the log. It sits in a gutter under
//! the panes, or in a pill over the scene's bottom-right corner, or is
//! hidden. See [`StatusStyle`].
//!
//! New warnings and errors in the log also become toasts.

use super::{LogSource, Pane};
use crate::{
    collab::{SessionDisplay, VaultDisplay, VaultState},
    style::{self, Role},
    widget::{PerfCapture, collab_config, status_dot, toast},
};
use std::time::Duration;
use web_time::SystemTime;

/// Where the status line shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum StatusStyle {
    /// A line in a gutter under the panes, on the scene.
    Gutter,
    /// A pill over the bottom-right corner of the scene.
    #[default]
    Pill,
    /// No status line. Toasts still show.
    Hidden,
}

/// An item that the host adds to the status line, such as the audio
/// device's rate and load. See [`super::Gantz::status_items`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusItem {
    /// A small icon before the item.
    pub icon: Option<StatusIcon>,
    /// The item's lead, in the text colour. The pill shows only the lead and
    /// the short text.
    pub lead: Option<String>,
    /// The rest of the item, in the weak colour.
    pub text: String,
    /// A shorter form of `text` for the pill. `None` uses `text`.
    pub short: Option<String>,
    /// Shown while the pointer is over the item.
    pub hover: Option<String>,
    /// What a click on the item does.
    pub action: Option<StatusAction>,
    /// What a click on the item's icon does. `None` leaves the icon to the
    /// item's `action`.
    pub icon_action: Option<StatusAction>,
    /// A level meter after the icon, one bar per channel. Empty shows none.
    pub meter: Vec<MeterLevel>,
}

/// One channel of a [`StatusItem`]'s meter.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterLevel {
    /// How full the bar is, from 0 to 1.
    pub level: f32,
    /// Whether the channel clipped of late. Its bar shows in the error colour.
    pub clipped: bool,
}

/// A small icon of a [`StatusItem`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusIcon {
    /// A speaker. It shows a cross while the focused graph is muted.
    Speaker,
}

/// What a click on a [`StatusItem`] does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusAction {
    /// Mute or unmute the focused graph.
    ToggleMute,
    /// Open the settings section with this title, such as a host's own
    /// section.
    OpenSettings(String),
}

/// What the status line shows.
pub(super) struct Status<'a> {
    /// The focused graph's name.
    pub graph: Option<String>,
    /// How long ago the focused graph was last saved.
    pub saved: Option<Duration>,
    /// The focused graph's session, or else the vault link.
    pub link: Option<Link<'a>>,
    /// The host's items.
    pub items: &'a [StatusItem],
    /// Whether the focused graph is muted.
    pub muted: bool,
    /// The recent VM times per update, oldest first.
    pub vm: Vec<Duration>,
    pub problems: Problems,
}

/// A link to other devices that the status line reports.
#[derive(Clone, Copy)]
pub(super) enum Link<'a> {
    Session(&'a SessionDisplay),
    Vault(&'a VaultDisplay),
}

/// What a click on the status line asks for.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct StatusResponse {
    /// The pane or settings section that a click asks to open.
    pub open: Option<Open>,
    /// An item that mutes the focused graph was clicked.
    pub toggle_mute: bool,
}

/// What a click on the status line opens.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Open {
    Pane(Pane),
    /// The settings section with this title.
    Settings(String),
}

/// The warnings and errors in the log that come from gantz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Problems {
    pub warnings: usize,
    pub errors: usize,
}

/// The height of the gutter that holds the status line.
pub(super) const GUTTER_HEIGHT: f32 = 30.0;

/// The id under which the context keeps the timestamp of the newest log
/// entry already seen, so that only newer entries become toasts.
const SEEN_ID: &str = "gantz-status-seen";

/// The id under which the context keeps this pass's problem counts.
const PROBLEMS_ID: &str = "gantz-status-problems";

/// The number of recent VM samples the status line averages.
const VM_SAMPLES: usize = 30;

/// The number of bars in the VM time sparkline.
const SPARK_BARS: usize = 12;

/// The gap between items of the gutter line and of the pill.
const GUTTER_GAP: f32 = 22.0;
const PILL_GAP: f32 = 14.0;

impl StatusResponse {
    /// Record a click on an item that does `action`.
    fn act(&mut self, action: &StatusAction) {
        match action {
            StatusAction::ToggleMute => self.toggle_mute = true,
            StatusAction::OpenSettings(title) => self.open = Some(Open::Settings(title.clone())),
        }
    }
}

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

/// The most recent VM samples, oldest first, enough for the mean and the
/// sparkline.
pub(super) fn vm_recent(capture: &PerfCapture) -> Vec<Duration> {
    let samples = capture.samples();
    let skip = samples.len().saturating_sub(VM_SAMPLES);
    samples.iter().skip(skip).copied().collect()
}

/// The time since `timestamp`, the time of a commit since the unix epoch.
/// `None` for a commit with no time, such as a base graph's.
pub(super) fn since(timestamp: Duration) -> Option<Duration> {
    if timestamp.is_zero() {
        return None;
    }
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?;
    now.checked_sub(timestamp)
}

/// Draw the status line in the gutter `rect`.
///
/// The line floats over the main scene, which runs on under the gutter. Each
/// end of the line is an area only as big as its items, so the scene between
/// them stays reachable.
pub(super) fn gutter(ctx: &egui::Context, rect: egui::Rect, status: &Status) -> StatusResponse {
    let mut res = StatusResponse::default();
    let side = |id: &str, pivot: egui::Align2, pos: egui::Pos2| {
        egui::Area::new(egui::Id::new(id))
            .order(egui::Order::Middle)
            .pivot(pivot)
            .fixed_pos(pos)
    };
    side(
        "gantz-status-left",
        egui::Align2::LEFT_CENTER,
        rect.left_center(),
    )
    .show(ctx, |ui| {
        ui.horizontal(|ui| {
            quiet(ui);
            ui.spacing_mut().item_spacing.x = GUTTER_GAP;
            if let Some(graph) = &status.graph {
                let saved = status.saved.map(|d| format!("saved {}", ago(d)));
                let item = item(ui, None, Some(graph), saved.as_deref().unwrap_or(""));
                if sensed(ui, &item, "graph", egui::Sense::click())
                    .on_hover_text("open the history")
                    .clicked()
                {
                    res.open = Some(Open::Pane(Pane::History));
                }
            }
            if let Some(link) = status.link {
                link_ui(ui, link, false, &mut res);
            }
            items_ui(ui, status, false, &mut res);
            if let Some(mean) = mean(&status.vm) {
                let ms = mean.as_secs_f64() * 1_000.0;
                let item = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 7.0;
                        ui.weak(format!("vm {ms:.2} ms"));
                        sparkline(ui, &status.vm);
                    })
                    .response;
                vm_ui(ui, &item, &mut res);
            }
        });
    });
    side(
        "gantz-status-right",
        egui::Align2::RIGHT_CENTER,
        rect.right_center(),
    )
    .show(ctx, |ui| {
        ui.horizontal(|ui| {
            quiet(ui);
            ui.spacing_mut().item_spacing.x = GUTTER_GAP;
            problems_ui(ui, status.problems, false, &mut res);
        });
    });
    res
}

/// Draw the status line in a pill whose bottom-right corner is `anchor`.
pub(super) fn pill(ctx: &egui::Context, anchor: egui::Pos2, status: &Status) -> StatusResponse {
    egui::Area::new(egui::Id::new("gantz-status-pill"))
        .order(egui::Order::Middle)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(anchor)
        .show(ctx, |ui| {
            let height = ui.spacing().interact_size.y + 8.0;
            egui::Frame::new()
                .fill(style::color(ui, Role::Panel))
                .corner_radius(style::shape(ctx).widget_corner(height))
                .inner_margin(egui::Margin::symmetric(14, 0))
                .show(ui, |ui| {
                    ui.set_height(height);
                    ui.horizontal_centered(|ui| {
                        quiet(ui);
                        ui.spacing_mut().item_spacing.x = PILL_GAP;
                        let mut res = StatusResponse::default();
                        if let Some(link) = status.link {
                            link_ui(ui, link, true, &mut res);
                        }
                        items_ui(ui, status, true, &mut res);
                        if let Some(mean) = mean(&status.vm) {
                            let ms = mean.as_secs_f64() * 1_000.0;
                            let item = item(ui, None, Some("vm"), &format!("{ms:.2} ms"));
                            vm_ui(ui, &item, &mut res);
                        }
                        problems_ui(ui, status.problems, true, &mut res);
                        res
                    })
                    .inner
                })
                .inner
        })
        .inner
}

/// One item: an optional dot, a lead in the text colour and the rest in the
/// weak colour.
fn item(
    ui: &mut egui::Ui,
    dot: Option<egui::Color32>,
    lead: Option<&str>,
    text: &str,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        if let Some(color) = dot {
            status_dot(ui, color);
        }
        if let Some(lead) = lead {
            ui.label(egui::RichText::new(lead).color(ui.visuals().text_color()));
        }
        if !text.is_empty() {
            ui.weak(text);
        }
    })
    .response
}

/// The focused graph's session, or the vault link. The pill shows a short
/// form. A click opens the collab settings.
fn link_ui(ui: &mut egui::Ui, link: Link, short: bool, res: &mut StatusResponse) {
    let (role, text, hover) = match link {
        Link::Session(session) => {
            let label = session.conn.label();
            let text = match (short, session.peers.len()) {
                (true, _) | (_, 0) => label.to_string(),
                (false, 1) => format!("{label} · 1 peer"),
                (false, n) => format!("{label} · {n} peers"),
            };
            (session.conn.role(), text, session.hover_text())
        }
        Link::Vault(vault) => {
            let state = match &vault.state {
                VaultState::Live => "synced",
                VaultState::Connecting => "connecting",
                VaultState::Offline(_) => "offline",
                _ => "needs action",
            };
            let text = match short {
                true => state.to_string(),
                false => format!("vault {state}"),
            };
            (vault.state.role(), text, vault.state.guidance().to_string())
        }
    };
    let color = style::color(ui, role);
    let item = item(ui, Some(color), None, &text);
    let hover = format!("{hover}\nclick to open the collab settings.");
    if sensed(ui, &item, "link", egui::Sense::click())
        .on_hover_text(hover)
        .clicked()
    {
        res.open = Some(Open::Settings(collab_config::TITLE.to_string()));
    }
}

/// The host's items. A click on an item's icon takes the icon's action, if
/// it has one, and a click on the rest takes the item's action.
fn items_ui(ui: &mut egui::Ui, status: &Status, short: bool, res: &mut StatusResponse) {
    for (i, it) in status.items.iter().enumerate() {
        let text = match short {
            true => it.short.as_deref().unwrap_or(&it.text),
            false => &it.text,
        };
        let mut icon_rect = None;
        let group = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                if let Some(StatusIcon::Speaker) = it.icon {
                    icon_rect = Some(speaker(ui, status.muted));
                }
                if !it.meter.is_empty() {
                    meter(ui, &it.meter);
                }
                if let Some(lead) = &it.lead {
                    ui.label(egui::RichText::new(lead).color(ui.visuals().text_color()));
                }
                ui.weak(text);
            })
            .response;
        let icon = it.icon_action.as_ref().zip(icon_rect);
        let rest = match icon {
            Some((_, rect)) => group.rect.with_min_x(rect.max.x),
            None => group.rect,
        };
        if let Some((action, rect)) = icon {
            let icon = interact(ui, rect, ("icon", i), egui::Sense::click());
            if icon.on_hover_text(hint(action, status.muted)).clicked() {
                res.act(action);
            }
        }
        let sense = match it.action {
            Some(_) => egui::Sense::click(),
            None => egui::Sense::hover(),
        };
        let item = interact(ui, rest, ("item", i), sense);
        let item = match &it.hover {
            Some(hover) => item.on_hover_text(hover),
            None => item,
        };
        if let (true, Some(action)) = (item.clicked(), &it.action) {
            res.act(action);
        }
    }
}

/// The VM time `item`. A click opens the VM perf pane.
fn vm_ui(ui: &egui::Ui, item: &egui::Response, res: &mut StatusResponse) {
    let hover = "the mean time of the recent VM updates. click to open the VM perf pane.";
    if sensed(ui, item, "vm", egui::Sense::click())
        .on_hover_text(hover)
        .clicked()
    {
        res.open = Some(Open::Pane(Pane::VmPerf));
    }
}

/// What a click on an item's icon does, for its hover text.
fn hint(action: &StatusAction, muted: bool) -> String {
    match action {
        StatusAction::ToggleMute if muted => "unmute the focused graph".to_string(),
        StatusAction::ToggleMute => "mute the focused graph".to_string(),
        StatusAction::OpenSettings(title) => {
            format!("open the {} settings", title.to_lowercase())
        }
    }
}

/// The problem counts in their severities' colours. The pill shows only the
/// numbers. A click opens the logs.
fn problems_ui(ui: &mut egui::Ui, problems: Problems, short: bool, res: &mut StatusResponse) {
    let Problems { warnings, errors } = problems;
    let visuals = ui.visuals();
    let counts = [
        (errors, "error", "errors", visuals.error_fg_color),
        (warnings, "warning", "warnings", visuals.warn_fg_color),
    ];
    for (n, one, many, color) in counts.into_iter().filter(|c| c.0 > 0) {
        let text = match (short, n) {
            (true, n) => n.to_string(),
            (false, 1) => format!("1 {one}"),
            (false, n) => format!("{n} {many}"),
        };
        let item = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                status_dot(ui, color);
                ui.label(egui::RichText::new(text).color(color));
            })
            .response;
        if sensed(ui, &item, ("problem", one), egui::Sense::click())
            .on_hover_text("open the logs")
            .clicked()
        {
            res.open = Some(Open::Pane(Pane::Logs));
        }
    }
}

/// The group `res` sensing `sense` under an id from `key`. A group's own id
/// is not stable from pass to pass, so it cannot take clicks or hovers.
fn sensed(
    ui: &egui::Ui,
    res: &egui::Response,
    key: impl std::hash::Hash,
    sense: egui::Sense,
) -> egui::Response {
    interact(ui, res.rect, key, sense)
}

/// `rect` sensing `sense` under an id from `key`. A part that takes clicks
/// shows the pointing hand, as it opens or toggles something.
fn interact(
    ui: &egui::Ui,
    rect: egui::Rect,
    key: impl std::hash::Hash,
    sense: egui::Sense,
) -> egui::Response {
    let res = ui.interact(rect, ui.id().with(key), sense);
    match sense.senses_click() {
        true => res.on_hover_cursor(egui::CursorIcon::PointingHand),
        false => res,
    }
}

/// Make the status line's text plain to the pointer. Its labels do not
/// select, so the pointer shows no text cursor over them.
fn quiet(ui: &mut egui::Ui) {
    ui.style_mut().interaction.selectable_labels = false;
}

/// Keep `problems` as this pass's counts, for parts drawn before the status
/// line, such as the logo's dot.
pub(super) fn set_last(ctx: &egui::Context, problems: Problems) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(PROBLEMS_ID), problems));
}

/// The counts kept by [`set_last`].
pub(super) fn last(ctx: &egui::Context) -> Problems {
    ctx.data(|d| d.get_temp(egui::Id::new(PROBLEMS_ID)))
        .unwrap_or_default()
}

/// The colour of the worst problem in `problems`, if any.
pub(super) fn color(problems: Problems, visuals: &egui::Visuals) -> Option<egui::Color32> {
    match problems {
        p if p.errors > 0 => Some(visuals.error_fg_color),
        p if p.warnings > 0 => Some(visuals.warn_fg_color),
        _ => None,
    }
}

/// The mean of `samples`.
fn mean(samples: &[Duration]) -> Option<Duration> {
    let n = u32::try_from(samples.len()).ok().filter(|&n| n > 0)?;
    Some(samples.iter().sum::<Duration>() / n)
}

/// A small bar chart of the most recent of `samples`, scaled to the largest.
fn sparkline(ui: &mut egui::Ui, samples: &[Duration]) {
    let (bar, gap, height) = (2.0, 2.0, 12.0);
    let width = SPARK_BARS as f32 * (bar + gap) - gap;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let recent = &samples[samples.len().saturating_sub(SPARK_BARS)..];
    let max = recent
        .iter()
        .max()
        .copied()
        .unwrap_or_default()
        .as_secs_f32();
    if max <= 0.0 {
        return;
    }
    let color = ui.visuals().weak_text_color();
    for (i, sample) in recent.iter().enumerate() {
        let h = (sample.as_secs_f32() / max * height).max(1.0);
        let x = rect.left() + i as f32 * (bar + gap);
        let r = egui::Rect::from_min_max(
            egui::pos2(x, rect.bottom() - h),
            egui::pos2(x + bar, rect.bottom()),
        );
        ui.painter().rect_filled(r, 1.0, color);
    }
}

/// A meter of `levels`, one thin bar per channel as tall as the speaker. Each
/// bar fills from its base with its level, in the error colour while its
/// channel clips.
fn meter(ui: &mut egui::Ui, levels: &[MeterLevel]) {
    let (bar, gap) = (2.0, 2.0);
    let height = ui.text_style_height(&egui::TextStyle::Body) * 0.85;
    let width = levels.len() as f32 * (bar + gap) - gap;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let visuals = ui.visuals();
    let track = visuals.widgets.noninteractive.bg_stroke.color;
    for (i, channel) in levels.iter().enumerate() {
        let x = rect.left() + i as f32 * (bar + gap);
        let full = egui::Rect::from_x_y_ranges(x..=x + bar, rect.y_range());
        ui.painter().rect_filled(full, 1.0, track);
        let color = match channel.clipped {
            true => visuals.error_fg_color,
            false => visuals.text_color(),
        };
        let level = full.with_min_y(full.bottom() - channel.level.clamp(0.0, 1.0) * height);
        ui.painter().rect_filled(level, 1.0, color);
    }
}

/// A speaker icon, crossed while `muted`. Returns its rect.
fn speaker(ui: &mut egui::Ui, muted: bool) -> egui::Rect {
    let size = ui.text_style_height(&egui::TextStyle::Body) * 0.85;
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
    let color = ui.visuals().weak_text_color();
    let p = |x: f32, y: f32| rect.min + egui::vec2(x, y) * size;
    let body = vec![
        p(0.05, 0.38),
        p(0.25, 0.38),
        p(0.5, 0.15),
        p(0.5, 0.85),
        p(0.25, 0.62),
        p(0.05, 0.62),
    ];
    ui.painter()
        .add(egui::Shape::convex_polygon(body, color, egui::Stroke::NONE));
    let stroke = egui::Stroke::new(size * 0.09, color);
    match muted {
        true => {
            ui.painter()
                .line_segment([p(0.65, 0.35), p(0.95, 0.65)], stroke);
            ui.painter()
                .line_segment([p(0.65, 0.65), p(0.95, 0.35)], stroke);
        }
        false => {
            let center = p(0.5, 0.5);
            for (radius, sweep) in [(0.22, 0.7), (0.4, 0.8)] {
                let points = (0..=8)
                    .map(|i| {
                        let a = -sweep + 2.0 * sweep * i as f32 / 8.0;
                        center + egui::vec2(a.cos(), a.sin()) * radius * size
                    })
                    .collect();
                ui.painter().add(egui::Shape::line(points, stroke));
            }
        }
    }
    rect
}

/// How long ago `d` was, in the largest whole unit.
fn ago(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..60 => "just now".to_string(),
        60..3_600 => format!("{}m ago", secs / 60),
        3_600..86_400 => format!("{}h ago", secs / 3_600),
        _ => format!("{}d ago", secs / 86_400),
    }
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
        assert_eq!(mean(&vm_recent(&capture)), None);
        for ms in [2, 4] {
            capture.record(Duration::from_millis(ms));
        }
        assert_eq!(mean(&vm_recent(&capture)), Some(Duration::from_millis(3)));
    }

    #[test]
    fn ago_names_the_largest_whole_unit() {
        let ago_secs = |s| ago(Duration::from_secs(s));
        assert_eq!(ago_secs(5), "just now");
        assert_eq!(ago_secs(150), "2m ago");
        assert_eq!(ago_secs(7_300), "2h ago");
        assert_eq!(ago_secs(200_000), "2d ago");
    }
}
