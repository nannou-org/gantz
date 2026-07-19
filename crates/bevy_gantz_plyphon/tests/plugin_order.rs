//! Plugin assembly must be order-insensitive: `PlyphonPlugin` reads the
//! shared `EvalEpoch` in `Plugin::finish`, so adding it BEFORE `GantzPlugin`
//! must work identically.

use bevy::MinimalPlugins;
use bevy::app::App;
use bevy_gantz::GantzPlugin;
use bevy_gantz_plyphon::{DspConfig, DspStatus, PlyphonPlugin};

/// A headless app with the gantz plugins added in REVERSED order builds,
/// finishes and ticks without panicking, with the DSP resources present.
#[test]
fn plugin_order_is_insensitive() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    // Deliberately reversed: the DSP runtime before the core plugin.
    app.add_plugins(PlyphonPlugin::new());
    app.add_plugins(GantzPlugin);
    // The typed side (cache + builtin instances) is otherwise owned by
    // `GantzEguiPlugin`, which this headless test does not add.
    app.init_resource::<bevy_gantz_egui::GraphCache>();
    app.init_resource::<bevy_gantz_egui::BuiltinNodes>();
    app.finish();
    app.cleanup();
    for _ in 0..3 {
        app.update();
    }
    assert!(app.world().contains_resource::<DspConfig>());
    assert!(
        app.world().contains_resource::<DspStatus>(),
        "DspStatus is inserted at finish regardless of device presence",
    );
}
