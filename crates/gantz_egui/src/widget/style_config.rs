//! The "Style" settings subtab: visual configuration of the graph scene.
//!
//! Currently hosts the dot-grid controls (show/hide and base step). The grid
//! step also feeds snap-to-grid (see `Global > Snap`), so it stays editable even
//! when the grid is hidden.

use super::gantz::GridConfig;

/// Render the style configuration controls. `grid` is mutated in place; the
/// config applies to all open heads.
pub fn style_config(grid: &mut GridConfig, ui: &mut egui::Ui) {
    ui.label("Grid:");
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
}
