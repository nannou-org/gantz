//! Toasts: short messages that float over the scene for a moment.
//!
//! Anything with the context can [`push`] a toast. [`show`] draws the
//! current toasts in a column. A warning fades after a few seconds. An error
//! stays until dismissed, since it usually asks for an action.

use crate::{style, widget::kit};

/// How serious a toast is. It sets the toast's mark and how long it stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// One toast.
#[derive(Clone, Debug)]
struct Toast {
    id: u64,
    severity: Severity,
    text: String,
    /// The context time at which the toast was pushed, in seconds.
    born: f64,
}

/// The toasts of a context, newest last.
#[derive(Clone, Debug, Default)]
struct Toasts {
    toasts: Vec<Toast>,
    next_id: u64,
}

/// The id under which a context keeps its toasts.
const TOASTS_ID: &str = "gantz-toasts";

/// How long an info or warning toast stays, in seconds.
const LIFETIME: f64 = 6.0;

/// The most toasts shown at once. The oldest go first.
const MAX_TOASTS: usize = 4;

/// Add a toast to `ctx`.
pub fn push(ctx: &egui::Context, severity: Severity, text: impl Into<String>) {
    let born = ctx.input(|i| i.time);
    let text = text.into();
    ctx.data_mut(|d| {
        let toasts = d.get_temp_mut_or_default::<Toasts>(egui::Id::new(TOASTS_ID));
        // A repeat of the newest toast refreshes it rather than stacking.
        if let Some(last) = toasts.toasts.last_mut()
            && last.text == text
            && last.severity == severity
        {
            last.born = born;
            return;
        }
        let id = toasts.next_id;
        toasts.next_id += 1;
        toasts.toasts.push(Toast {
            id,
            severity,
            text,
            born,
        });
        let excess = toasts.toasts.len().saturating_sub(MAX_TOASTS);
        toasts.toasts.drain(..excess);
    });
    ctx.request_repaint();
}

/// Draw the toasts of `ctx` in a column whose bottom centre is `anchor`.
/// Expired toasts are dropped and a click on a toast's cross dismisses it.
pub fn show(ctx: &egui::Context, anchor: egui::Pos2) {
    let now = ctx.input(|i| i.time);
    let id = egui::Id::new(TOASTS_ID);
    let mut toasts = ctx.data(|d| d.get_temp::<Toasts>(id)).unwrap_or_default();
    toasts
        .toasts
        .retain(|t| t.severity == Severity::Error || now - t.born < LIFETIME);
    if toasts.toasts.is_empty() {
        ctx.data_mut(|d| d.insert_temp(id, toasts));
        return;
    }
    let mut dismissed = vec![];
    egui::Area::new(egui::Id::new("gantz-toasts-area"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_BOTTOM)
        .fixed_pos(anchor)
        .interactable(true)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.vertical_centered(|ui| {
                for toast in &toasts.toasts {
                    if toast_ui(ui, toast) {
                        dismissed.push(toast.id);
                    }
                }
            });
        });
    toasts.toasts.retain(|t| !dismissed.contains(&t.id));
    // Fading toasts need a frame when they expire.
    if toasts.toasts.iter().any(|t| t.severity != Severity::Error) {
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
    ctx.data_mut(|d| d.insert_temp(id, toasts));
}

/// One toast: a mark in its severity's colour, the text and a cross.
/// Returns whether the cross was clicked.
fn toast_ui(ui: &mut egui::Ui, toast: &Toast) -> bool {
    let visuals = ui.visuals();
    let color = match toast.severity {
        Severity::Info => visuals.text_color(),
        Severity::Warning => visuals.warn_fg_color,
        Severity::Error => visuals.error_fg_color,
    };
    let shape = style::shape(ui.ctx());
    let frame = egui::Frame::popup(ui.style())
        .stroke(egui::Stroke::NONE)
        .corner_radius(shape.window_corner())
        .inner_margin(egui::Margin::symmetric(12, 6));
    frame
        .show(ui, |ui| {
            ui.set_max_width(480.0);
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::Vec2::splat(8.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, color);
                ui.add(egui::Label::new(&toast.text).wrap());
                let cross = kit::pill(ui, false, "×").on_hover_text("dismiss");
                cross.clicked()
            })
            .inner
        })
        .inner
}
