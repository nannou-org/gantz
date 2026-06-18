//! The "Global" sidebar tab: globally relevant configuration.
//!
//! Hosts the global compile config toggles (moved here from the per-head Graph
//! Config pane) and a button to reset all demo graphs to their initial state.

/// Response from [`global_config`].
#[derive(Default)]
pub struct GlobalConfigResponse {
    /// The global compile config was changed via the compile toggles.
    pub compile_config: Option<gantz_core::compile::Config>,
    /// The "Reset all demos" button was clicked.
    pub reset_all_demos: bool,
}

/// Render the global configuration controls.
///
/// `compile_config` is the current global config; when `Some` the compile
/// toggles are shown. The config applies to all open heads.
pub fn global_config(
    compile_config: Option<gantz_core::compile::Config>,
    ui: &mut egui::Ui,
) -> GlobalConfigResponse {
    let mut changed_config = None;
    if let Some(mut cfg) = compile_config {
        ui.label("Compile:");
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
        ui.separator();
    }

    let reset_all_demos = ui
        .button("Reset all demos")
        .on_hover_text("reset all demo graphs to their initial state")
        .clicked();

    GlobalConfigResponse {
        compile_config: changed_config,
        reset_all_demos,
    }
}
