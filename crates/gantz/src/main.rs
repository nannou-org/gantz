use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .add_systems(Startup, setup_camera)
        .add_systems(EguiPrimaryContextPass, gui)
        .run();
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

fn gui(mut ctxs: EguiContexts) -> Result {
    egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctxs.ctx_mut()?, |_ui| {
            // TODO: Gantz widget.
        });

    Ok(())
}
