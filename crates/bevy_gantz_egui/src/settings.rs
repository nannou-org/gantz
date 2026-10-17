//! Settings changes from the Settings pane.

use crate::ForApp;
use bevy_ecs::prelude::*;
use bevy_gantz::CompileConfig;

/// Apply a compile config change. The recompile happens next frame in
/// `vm::sync`, which compares each head's compile inputs by value.
pub(crate) fn on_compile_config(
    trigger: On<ForApp<gantz_core::compile::Config>>,
    mut compile_config: ResMut<CompileConfig>,
) {
    let cfg = trigger.event().0;
    if compile_config.0 != cfg {
        compile_config.0 = cfg;
    }
}

/// Turn the change-tracking validation, a debugging aid, on or off.
pub(crate) fn on_validate_change_tracking(
    trigger: On<ForApp<gantz_egui::ValidateChangeTracking>>,
    mut validate: ResMut<bevy_gantz::ValidateCommitted>,
) {
    let gantz_egui::ValidateChangeTracking(enabled) = trigger.event().0;
    validate.0 = enabled;
}
