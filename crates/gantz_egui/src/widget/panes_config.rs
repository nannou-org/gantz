//! The "Panes" controls. Per-pane visibility checkboxes plus a reset-layout
//! button. Shared between the Settings pane's Panes subtab and the graph-area
//! context menu's "panes" submenu.

use super::gantz::{StatusStyle, ViewToggles};

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
        "graphs",
        "Browse, create and import graphs.",
    );
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.history,
        "history",
        "Browse the commit history.",
    );
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.settings,
        "settings",
        "Global, style, keybind and pane settings.",
    );
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.graph_config,
        "graph",
        "The focused graph's name, layout and sharing.",
    );
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.node_inspector,
        "inspector",
        "Inspect and edit the selected node(s).",
    );
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.perf_vm,
        "vm perf",
        "Virtual machine evaluation timing.",
    );
    sidebar_pane(
        ui,
        &mut view.sidebar_open,
        &mut view.perf_gui,
        "gui perf",
        "GUI rendering timing.",
    );
    // Tray panes are independent of the sidebar.
    ui.checkbox(&mut view.logs, "logs")
        .on_hover_text("Log output from the running graphs.");
    ui.checkbox(&mut view.steel, "steel")
        .on_hover_text("The compiled Steel code for the focused graph.");
    ui.checkbox(&mut view.gui_preview, "gui preview")
        .on_hover_text("The focused graph's GUI, rendered live from its gui marker.");
    ui.checkbox(&mut view.gui_tree, "gui tree")
        .on_hover_text("The stored gui marker tree as scheme text, with decode warnings.");
    for entry in ext {
        let on = view.ext.entry(entry.key.clone()).or_insert(false);
        ui.checkbox(on, entry.title.to_lowercase())
            .on_hover_text(entry.description.as_str());
    }
    let options = [
        (StatusStyle::Gutter, "line"),
        (StatusStyle::Pill, "pill"),
        (StatusStyle::Hidden, "off"),
    ];
    super::kit::pill_row(ui, "status", &mut view.status, &options).on_hover_text(
        "Show the focused graph, its session, the VM time and the log's problems in a line \
         under the panes, or in a pill over the scene.",
    );
}

/// Render the "reset all" layout button. Returns `true` when clicked.
pub fn reset_layout_button(ui: &mut egui::Ui) -> bool {
    ui.button("reset layout")
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
