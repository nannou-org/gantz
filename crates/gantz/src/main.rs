use bevy::{
    prelude::*,
    window::{Window, WindowPlugin},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(window_plugin()))
        .add_plugins(EguiPlugin::default())
        .add_systems(Startup, setup_camera)
        .add_systems(EguiPrimaryContextPass, gui)
        .run();
}

fn window_plugin() -> WindowPlugin {
    WindowPlugin {
        primary_window: Some(Window {
            title: "gantz".into(),
            name: Some("gantz".into()),
            fit_canvas_to_parent: true,
            ..default()
        }),
        ..default()
    }
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

fn gui(mut ctxs: EguiContexts) -> Result {
    egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctxs.ctx_mut()?, |_ui| {
            // TODO
        });
    Ok(())
}
