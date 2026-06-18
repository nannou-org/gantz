//! The "Panes" controls: per-pane visibility checkboxes plus a reset-layout
//! button. Shared between the Settings pane's Panes subtab and the graph-area
//! context menu's "panes" submenu.

use super::gantz::ViewToggles;

/// Render the per-pane visibility checkboxes.
pub fn panes_config(view: &mut ViewToggles, ui: &mut egui::Ui) {
    // Sidebar panes: enabling one also opens the sidebar if it is closed, so
    // the toggle has a visible effect (e.g. from the graph-area context menu).
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.graphs,
        "Graphs",
        "Browse, create and import graphs.",
    );
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.history,
        "History",
        "Browse the commit history.",
    );
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.graph_config,
        "Graph Config",
        "Per-graph layout settings and rename.",
    );
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.node_inspector,
        "Node Inspector",
        "Inspect and edit the selected node(s).",
    );
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.perf_vm,
        "VM Perf",
        "Virtual machine evaluation timing.",
    );
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.perf_gui,
        "GUI Perf",
        "GUI rendering timing.",
    );
    // Tray panes: independent of the sidebar.
    ui.checkbox(&mut view.logs, "Logs")
        .on_hover_text("Log output from the running graphs.");
    ui.checkbox(&mut view.steel, "Steel")
        .on_hover_text("The compiled Steel code for the focused graph.");
}

/// Render the "reset all" layout button. Returns `true` when clicked.
pub fn reset_layout_button(ui: &mut egui::Ui) -> bool {
    ui.button("reset all")
        .on_hover_text("Reset all top-level panes to their default arrangement and size.")
        .clicked()
}

/// A sidebar-pane checkbox. Enabling it opens the sidebar so the change shows.
fn sidebar_pane(
    ui: &mut egui::Ui,
    sidebar_open: &mut bool,
    on: &mut bool,
    label: &str,
    hover: &str,
) {
    if ui.checkbox(on, label).on_hover_text(hover).changed() && *on {
        *sidebar_open = true;
    }
}
