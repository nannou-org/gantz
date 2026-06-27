//! The "Global" sidebar tab: globally relevant configuration.
//!
//! Hosts the global compile config toggles (moved here from the per-head Graph
//! Config pane), the global auto-layout parameters, the drag snapping /
//! snap-align options, and a button to reset all demo graphs to their initial
//! state.

use super::gantz::{AlignConfig, LayoutConfig, SnapConfig, SnapMode};

/// Response from [`global_config`].
#[derive(Default)]
pub struct GlobalConfigResponse {
    /// The global compile config was changed via the compile toggles.
    pub compile_config: Option<gantz_core::compile::Config>,
    /// The change-tracking validation toggle was changed (its new value).
    pub validate_change_tracking: Option<bool>,
    /// The "Reset all demos" button was clicked.
    pub reset_all_demos: bool,
}

/// Render the global configuration controls.
///
/// `compile_config` is the current global config; when `Some` the compile
/// toggles are shown. `validate_change_tracking` is the current state of the
/// change-tracking validation toggle; when `Some` the toggle is shown.
/// `layout_config` holds the global auto-layout parameters, `snap` the drag
/// snapping mode and `align` the drag-time snap-align options (all mutated in
/// place). The config applies to all open heads.
pub fn global_config(
    compile_config: Option<gantz_core::compile::Config>,
    validate_change_tracking: Option<bool>,
    layout_config: &mut LayoutConfig,
    snap: &mut SnapConfig,
    align: &mut AlignConfig,
    ui: &mut egui::Ui,
) -> GlobalConfigResponse {
    let mut changed_config = None;
    let mut changed_validate = None;
    if compile_config.is_some() || validate_change_tracking.is_some() {
        ui.label("Compile:");
    }
    if let Some(mut cfg) = compile_config {
        let mut changed = false;
        changed |= ui
            .checkbox(&mut cfg.validate_ir, "Validate IR")
            .on_hover_text(
                "Check the compiler's own IR invariants on every lowering. A \
                 violation is a bug in gantz, never in your graph. Disable as \
                 an optimisation. Applies to all open graphs.",
            )
            .changed();
        changed |= ui
            .checkbox(&mut cfg.emit_all_node_fns, "Emit all node fns")
            .on_hover_text(
                "Emit a node fn for every node, rather than only those called \
                 by some evaluation, so any node's generated code can be \
                 inspected in the Steel view. The extra definitions are never \
                 called and do not affect evaluation. Applies to all open \
                 graphs.",
            )
            .changed();
        if changed {
            changed_config = Some(cfg);
        }
    }
    if let Some(mut v) = validate_change_tracking {
        if ui
            .checkbox(&mut v, "Validate change tracking")
            .on_hover_text(
                "Re-hash every open graph each frame and warn if one changed \
                 without being reported - a way to catch a missed `changed` \
                 signal. A debugging aid only; leave off in normal use as it \
                 reinstates the per-frame hashing this avoids.",
            )
            .changed()
        {
            changed_validate = Some(v);
        }
    }
    if compile_config.is_some() || validate_change_tracking.is_some() {
        ui.separator();
    }

    // Auto-layout parameters (the non-flow `egui_graph` layout params; flow is
    // per-head, in the Graph Config pane). Applied on the next auto-layout.
    ui.label("Layout:");
    let gap = |ui: &mut egui::Ui, label: &str, value: &mut f32, hover: &str| {
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(value)
                    .speed(0.5)
                    .range(0.0..=500.0)
                    .suffix(" px"),
            )
            .on_hover_text(hover);
            ui.label(label);
        });
    };
    gap(
        ui,
        "Layer gap",
        &mut layout_config.layer_gap,
        "Gap between adjacent layers along the flow direction.",
    );
    gap(
        ui,
        "Node gap",
        &mut layout_config.node_gap,
        "Gap between adjacent nodes within a layer.",
    );
    gap(
        ui,
        "Component gap",
        &mut layout_config.component_gap,
        "Gap between disconnected components of the graph.",
    );
    ui.checkbox(&mut layout_config.socket_aware, "Socket-aware")
        .on_hover_text(
            "Account for the socket each edge connects to when ordering nodes \
             and minimising edge crossings. When off, edges anchor at node \
             centres (classic node-size-only layout).",
        );
    ui.separator();

    // Drag snapping. Point snaps to unit points (effectively free); Grid snaps
    // to a fraction of the dot grid (the grid step is set in Style).
    ui.label("Snap:");
    ui.horizontal(|ui| {
        ui.radio_value(&mut snap.mode, SnapMode::Point, "Point")
            .on_hover_text("Snap to the nearest unit point - effectively free movement.");
        ui.radio_value(&mut snap.mode, SnapMode::Grid, "Grid")
            .on_hover_text("Snap to a fraction of the dot grid (set the grid step in Style).");
    });
    ui.add_enabled_ui(snap.mode == SnapMode::Grid, |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut snap.grid_ratio)
                    .speed(0.01)
                    .range(0.0625..=8.0),
            )
            .on_hover_text(
                "Snap step relative to the grid step: 1.0 full grid, 0.5 half, \
                 0.25 quarter.",
            );
            ui.label("Grid ratio");
        });
    });
    ui.separator();

    // Drag-time snap-align: snap a dragged node to its neighbours' edges /
    // centres and draw guides.
    ui.label("Snap-align:");
    ui.checkbox(&mut align.enabled, "Align to neighbours")
        .on_hover_text(
            "While dragging, align a node to its neighbours' edges or centres \
             and draw guides. Hold Alt to suppress per drag.",
        );
    ui.add_enabled_ui(align.enabled, |ui| {
        ui.checkbox(&mut align.edges, "Edges (sides)")
            .on_hover_text("Align to neighbours' left/right/top/bottom edges.");
        ui.checkbox(&mut align.centers, "Centres")
            .on_hover_text("Align to neighbours' horizontal/vertical centres.");
    });
    ui.separator();

    let reset_all_demos = ui
        .button("Reset all demos")
        .on_hover_text("reset all demo graphs to their initial state")
        .clicked();

    GlobalConfigResponse {
        compile_config: changed_config,
        validate_change_tracking: changed_validate,
        reset_all_demos,
    }
}
