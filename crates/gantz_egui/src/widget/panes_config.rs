//! The "Panes" sidebar tab: per-pane visibility toggles.
//!
//! This replaces the floating eye that used to live in the bottom-right of the
//! graph scene. The toggles mutate [`ViewToggles`] in place; the tree picks up
//! the change on the next frame via `set_tile_visibility`.

use super::gantz::ViewToggles;

/// Render the per-pane visibility toggle list as checkboxes.
pub fn panes_config(view: &mut ViewToggles, ui: &mut egui::Ui) {
    ui.checkbox(&mut view.graphs, "Graphs");
    ui.checkbox(&mut view.history, "History");
    ui.checkbox(&mut view.graph_config, "Graph Config");
    ui.checkbox(&mut view.node_inspector, "Node Inspector");
    ui.checkbox(&mut view.perf_vm, "VM Perf");
    ui.checkbox(&mut view.perf_gui, "GUI Perf");
    ui.checkbox(&mut view.logs, "Logs");
    ui.checkbox(&mut view.steel, "Steel");
}
