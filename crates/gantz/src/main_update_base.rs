//! Developer tool for authoring base nodes.
//!
//! Starts with a clean registry populated only from `base/base.gantz`.
//! No user storage is loaded or saved. On every debounced input event,
//! all named graphs are exported back to `base/base.gantz`.
//!
//! Usage: `cargo run -p gantz --bin update-base`

use bevy::{prelude::*, window::Window};
use bevy_egui::EguiPlugin;
use bevy_gantz::{
    BuiltinNodes, GantzPlugin,
    debounced_input::{DebouncedInputEvent, DebouncedInputPlugin},
    vm,
};
use bevy_gantz_egui::{GantzEguiPlugin, TraceCapture};
use builtin::Builtins;

mod builtin;
mod node;

fn main() {
    App::new()
        .add_plugins(GantzPlugin::<Box<dyn node::Node>>::default())
        .add_plugins(GantzEguiPlugin::<Box<dyn node::Node>>::default())
        .insert_resource(BuiltinNodes::<Box<dyn node::Node>>(Box::new(
            Builtins::new(),
        )))
        .add_plugins(DefaultPlugins.set(log_plugin()).set(window_plugin()))
        .add_plugins(EguiPlugin::default())
        .add_plugins(DebouncedInputPlugin::new(0.25))
        .insert_resource(bevy_gantz_egui::base::ExportPath(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../base/base.gantz"
        )))
        .add_systems(
            Startup,
            (
                setup_camera,
                bevy_gantz_egui::base::load::<Box<dyn node::Node>>,
                vm::setup::<Box<dyn node::Node>>
                    .after(bevy_gantz_egui::base::load::<Box<dyn node::Node>>),
            ),
        )
        .add_systems(
            Update,
            bevy_gantz_egui::base::export_to_file::<Box<dyn node::Node>>
                .run_if(on_message::<DebouncedInputEvent>),
        )
        .run();
}

fn log_plugin() -> bevy::log::LogPlugin {
    bevy::log::LogPlugin {
        custom_layer: move |app| {
            let capture = app.world().resource_ref::<TraceCapture>();
            Some(Box::new(capture.0.clone().layer()))
        },
        ..Default::default()
    }
}

fn window_plugin() -> bevy::window::WindowPlugin {
    bevy::window::WindowPlugin {
        primary_window: Some(Window {
            title: "gantz - update base".into(),
            name: Some("gantz-update-base".into()),
            fit_canvas_to_parent: true,
            present_mode: bevy::window::PresentMode::AutoNoVsync,
            ..default()
        }),
        ..default()
    }
}

fn setup_camera(mut cmds: Commands) {
    cmds.spawn(Camera2d);
}
