//! The "Panes" controls. Per-pane visibility checkboxes plus a reset-layout
//! button. Shared between the Settings pane's Panes subtab and the graph-area
//! context menu's "panes" submenu.

use super::gantz::ViewToggles;

/// Render the per-pane visibility checkboxes.
///
/// `ext` lists the supplied extension panes. See [`ExtPane`][super::ExtPane].
/// Each entry gets one checkbox after the built-in tray panes.
pub fn panes_config(view: &mut ViewToggles, ext: &[super::ExtPaneEntry], ui: &mut egui::Ui) {
    // Enabling a sidebar pane also opens the sidebar if it is closed, so the
    // toggle has a visible effect from the graph-area context menu.
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
        &mut view.settings,
        "Settings",
        "Global / Style / Panes settings (also reachable from this menu).",
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
    // Tray panes are independent of the sidebar.
    ui.checkbox(&mut view.logs, "Logs")
        .on_hover_text("Log output from the running graphs.");
    ui.checkbox(&mut view.steel, "Steel")
        .on_hover_text("The compiled Steel code for the focused graph.");
    ui.checkbox(&mut view.gui_debug, "GUI Debug").on_hover_text(
        "An editable GUI tree literal rendered live against the focused graph's VM.",
    );
    for entry in ext {
        let on = view.ext.entry(entry.key.clone()).or_insert(false);
        ui.checkbox(on, entry.title.as_str())
            .on_hover_text(entry.description.as_str());
    }
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
