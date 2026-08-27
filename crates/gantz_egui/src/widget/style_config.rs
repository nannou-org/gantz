//! The "Style" settings subtab: the egui theme and style, plus the visual
//! configuration of the graph scene.
//!
//! The theme preference and the per-theme [`egui::Style`] live in a
//! [`StyleConfig`] on [`GantzState`][super::GantzState] (see [`crate::style`]);
//! this only edits it, and [`crate::style::apply`] does the applying. The
//! editor edits the selected theme's style.
//!
//! Also hosts the dot-grid controls (show/hide and base step). The grid step
//! feeds snap-to-grid (see `Global > Snap`), so it stays editable even when the
//! grid is hidden.

use super::gantz::GridConfig;
use crate::{
    StyleConfig,
    style::{eq_style, reset_theme, set_style_of, style_of},
};

/// Response from [`style_config`].
#[derive(Default)]
pub struct StyleConfigResponse {
    /// The "Export" button was clicked.
    pub export: bool,
    /// The "Import" button was clicked.
    pub import: bool,
}

/// Render the style configuration controls. `style` and `grid` are mutated in
/// place; both apply to the whole UI, including all open heads.
pub fn style_config(
    style: &mut StyleConfig,
    grid: &mut GridConfig,
    ui: &mut egui::Ui,
) -> StyleConfigResponse {
    ui.strong("Theme");
    style.theme.radio_buttons(ui);
    ui.separator();

    // The editor below edits the selected theme's style. `ctx.theme()` only
    // reflects a preference change on the next frame, so resolve directly.
    let edit = match style.theme {
        egui::ThemePreference::Dark => egui::Theme::Dark,
        egui::ThemePreference::Light => egui::Theme::Light,
        egui::ThemePreference::System => ui.ctx().theme(),
    };

    ui.strong("Style");
    let mut res = StyleConfigResponse::default();
    let mut reset = false;
    ui.horizontal(|ui| {
        reset = ui
            .button("Reset")
            .on_hover_text("Discard this theme's edits, restoring egui's default style.")
            .clicked();
        res.export = ui
            .button("Export…")
            .on_hover_text("Save both themes' styles to a file.")
            .clicked();
        res.import = ui
            .button("Import…")
            .on_hover_text("Load both themes' styles from an exported file.")
            .clicked();
    });
    if reset {
        reset_theme(style, edit);
    }

    // egui's own style editor, as seen in its demo. It edits a copy of the
    // effective style; storing that back drops the override again if the user
    // has hand-reverted every value.
    let mut edited = style_of(style, edit);
    edited.ui(ui);
    // egui's "Reset style" button at the bottom of the tree resets to
    // `Style::default()`, whose visuals are dark whichever theme is edited.
    // Treat it as a reset to the edited theme's own default.
    if eq_style(&edited, &egui::Style::default()) {
        edited = edit.default_style();
    }
    set_style_of(style, edit, edited);
    ui.separator();

    ui.strong("Grid");
    ui.checkbox(&mut grid.show, "Show grid")
        .on_hover_text("Draw the dot grid behind the graph.");
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut grid.step)
                .speed(0.5)
                .range(1.0..=500.0)
                .suffix(" px"),
        )
        .on_hover_text(
            "Base spacing of the dot grid, in graph-space units. Snap-to-grid \
             uses a fraction of this (see Global > Snap), so it applies even \
             when the grid is hidden.",
        );
        ui.label("Grid step");
    });

    res
}
