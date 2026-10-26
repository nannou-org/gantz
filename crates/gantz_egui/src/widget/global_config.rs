//! The "Global" settings subtab. Globally relevant configuration.
//!
//! Hosts the global compile config toggles, the global auto-layout
//! parameters, the drag snapping and snap-align options, the follow-selection
//! toggles, and a button to reset all demo graphs to their initial state.

use super::gantz::{
    AlignConfig, FollowSelectionConfig, LayoutConfig, SnapConfig, SnapMode,
    request_clear_egui_memory,
};
use crate::widget::{kit, section};

/// Response from [`global_config`].
#[derive(Default)]
pub struct GlobalConfigResponse {
    /// The global compile config was changed via the compile toggles.
    pub compile_config: Option<gantz_core::compile::Config>,
    /// The change-tracking validation toggle was changed. Holds the new value.
    pub validate_change_tracking: Option<bool>,
    /// The "Reset all demos" button was clicked.
    pub reset_all_demos: bool,
}

/// Render the global configuration controls.
///
/// `compile_config` is the current global config. The compile toggles show
/// when it is `Some`. `validate_change_tracking` is the current state of the
/// change-tracking validation toggle. The toggle shows when it is `Some`.
/// `layout_config` holds the global auto-layout parameters, `snap` the drag
/// snapping mode, `align` the drag-time snap-align options and
/// `follow_selection` which panes scroll to the selected node. All four are
/// mutated in place. The config applies to all open heads.
pub fn global_config(
    compile_config: Option<gantz_core::compile::Config>,
    validate_change_tracking: Option<bool>,
    layout_config: &mut LayoutConfig,
    snap: &mut SnapConfig,
    align: &mut AlignConfig,
    follow_selection: &mut FollowSelectionConfig,
    ui: &mut egui::Ui,
) -> GlobalConfigResponse {
    let mut changed_config = None;
    let mut changed_validate = None;
    if compile_config.is_some() || validate_change_tracking.is_some() {
        section(ui, "compile", |ui| {
            if let Some(mut cfg) = compile_config {
                let mut changed = false;
                changed |= kit::switch_row(ui, &mut cfg.validate_ir, "validate ir")
                    .on_hover_text(
                        "Check the compiler's own IR invariants on every lowering. A \
                         violation is a bug in gantz, never in your graph. Disable as \
                         an optimisation. Applies to all open graphs.",
                    )
                    .changed();
                changed |= kit::switch_row(ui, &mut cfg.emit_all_node_fns, "emit all node fns")
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
                if kit::switch_row(ui, &mut v, "validate change tracking")
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
        });
    }

    // The non-flow `egui_graph` layout params, applied on the next
    // auto-layout. Flow is per-head, in the Graph Config pane.
    section(ui, "layout", |ui| {
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
            "layer gap",
            &mut layout_config.layer_gap,
            "Gap between adjacent layers along the flow direction.",
        );
        gap(
            ui,
            "node gap",
            &mut layout_config.node_gap,
            "Gap between adjacent nodes within a layer.",
        );
        gap(
            ui,
            "component gap",
            &mut layout_config.component_gap,
            "Gap between disconnected components of the graph.",
        );
        kit::switch_row(ui, &mut layout_config.socket_aware, "socket-aware").on_hover_text(
            "Account for the socket each edge connects to when ordering nodes \
                 and minimising edge crossings. When off, edges anchor at node \
                 centres (classic node-size-only layout).",
        );
    });

    // Point snaps to unit points, which is effectively free. Grid snaps to a
    // fraction of the dot grid. The grid step is set in Style.
    section(ui, "snap", |ui| {
        let options = [(SnapMode::Grid, "grid"), (SnapMode::Point, "point")];
        kit::pill_row(ui, "snap to", &mut snap.mode, &options).on_hover_text(
            "Point snaps to the nearest unit point, which is effectively free movement. \
             Grid snaps to a fraction of the dot grid, whose step is set in Style.",
        );
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
                ui.label("grid ratio");
            });
        });
    });

    // Snap a dragged node to its neighbours' edges or centres and draw guides.
    section(ui, "snap-align", |ui| {
        kit::switch_row(ui, &mut align.enabled, "align to neighbours").on_hover_text(
            "While dragging, align a node to its neighbours' edges or centres \
                 and draw guides. Hold Alt to suppress per drag.",
        );
        ui.add_enabled_ui(align.enabled, |ui| {
            kit::switch_row(ui, &mut align.edges, "edges")
                .on_hover_text("Align to neighbours' left/right/top/bottom edges.");
            kit::switch_row(ui, &mut align.centers, "centres")
                .on_hover_text("Align to neighbours' horizontal/vertical centres.");
        });
    });

    section(ui, "follow selection", |ui| {
        kit::switch_row(ui, &mut follow_selection.inspector, "inspector").on_hover_text(
            "When the selection changes, scroll the Node Inspector to the selected node.",
        );
        kit::switch_row(ui, &mut follow_selection.steel, "steel").on_hover_text(
            "When the selection changes, scroll the Steel pane to the selected node's code.",
        );
    });

    // A recovery tool that drops egui's persisted UI memory when it has
    // accumulated stale state. It never touches the graph registry.
    let reset_all_demos = section(ui, "maintenance", |ui| {
        if ui
            .button("clear egui memory")
            .on_hover_text(
                "Discard egui's persisted UI memory (panel layout, widget state) on \
                 the next frame. Recovers from accumulated or stale egui state \
                 without touching your graphs. The UI layout resets to default.",
            )
            .clicked()
        {
            request_clear_egui_memory(ui.ctx());
        }
        let text = egui::RichText::new("reset all demos").color(ui.visuals().error_fg_color);
        ui.button(text)
            .on_hover_text("reset all demo graphs to their initial state")
            .clicked()
    });

    GlobalConfigResponse {
        compile_config: changed_config,
        validate_change_tracking: changed_validate,
        reset_all_demos,
    }
}
