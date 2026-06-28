use bevy::{
    prelude::*,
    window::{PrimaryWindow, Window},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_gantz::{
    BuiltinNodes, FocusedHead, GantzPlugin, HeadRef, HeadTabOrder, OpenHead, Registry,
    WorkingGraph,
    debounced_input::{DebouncedInputEvent, DebouncedInputPlugin},
    reg, timestamp,
};
use bevy_gantz_egui::{GantzEguiPlugin, HeadGuiState, TraceCapture, Views};
use bevy_pkv::PkvStore;
use builtin::Builtins;
use storage::Pkv;

mod builtin;
mod node;
mod persist;
mod storage;
mod window;

fn main() {
    App::new()
        // Core gantz plugin (provides FocusedHead, HeadTabOrder, HeadVms, Registry, Views)
        .add_plugins(GantzPlugin::<Box<dyn node::Node>>::default())
        // Egui plugin (provides GuiState, TraceCapture, PerfVm, PerfGui, GUI systems)
        .add_plugins(GantzEguiPlugin::<Box<dyn node::Node>>::default())
        // App-specific builtins
        .insert_resource(BuiltinNodes::<Box<dyn node::Node>>(Box::new(
            Builtins::new(),
        )))
        .add_plugins(DefaultPlugins.set(log_plugin()).set(window::plugin()))
        .add_plugins(EguiPlugin::default())
        // Drives both layout settling and the registry/views persist.
        .add_plugins(DebouncedInputPlugin::<DebouncedInputEvent>::new(0.25))
        // Off-thread, debounced persistence of registry/views/gui + egui memory.
        .add_plugins(persist::PersistPlugin)
        .insert_resource(Pkv::new(PkvStore::new("nannou-org", "gantz")))
        .add_systems(
            Startup,
            (
                setup_camera,
                setup_window,
                setup_resources,
                bevy_gantz_egui::base::load::<Box<dyn node::Node>>
                    .after(setup_resources)
                    .before(setup_open),
                setup_open.after(setup_resources),
                reg::prune_unused::<Box<dyn node::Node>>
                    .after(setup_resources)
                    .after(setup_open),
            ),
        )
        .add_systems(EguiPrimaryContextPass, load_egui_memory)
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

fn setup_camera(mut cmds: Commands) {
    cmds.spawn(Camera2d);
}

/// Restore the persisted window size (native only; no-op on web).
fn setup_window(storage: Res<Pkv>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    if let Ok(mut window) = windows.single_mut() {
        window::apply_saved_size(&*storage, &mut window);
    }
}

fn setup_resources(storage: Res<Pkv>, mut cmds: Commands) {
    let registry: Registry<Box<dyn node::Node>> = bevy_gantz::storage::load_registry(&*storage);
    // Seed the persist tracker from the disk-loaded registry: everything loaded
    // is, by definition, already on disk. Done before `base::load` merges base
    // graphs (so they're written on first persist) and before `prune_unused`
    // (so prunes are detected on the first incremental save).
    let persisted = bevy_gantz::storage::PersistedRegistry::from_registry(&registry);
    let views = bevy_gantz_egui::storage::load_views(&*storage);
    let gui_state = bevy_gantz_egui::storage::load_gui_state(&*storage);
    cmds.insert_resource(registry);
    cmds.insert_resource(persisted);
    cmds.insert_resource(views);
    cmds.insert_resource(gui_state);
}

fn setup_open(
    storage: Res<Pkv>,
    mut registry: ResMut<Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
    mut cmds: Commands,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
) {
    let loaded =
        bevy_gantz_egui::storage::load_open(&*storage, &mut *registry, &*views, timestamp());
    let focused_head = bevy_gantz::storage::load_focused_head(&*storage);

    // Spawn entities for each open head. `OpenHead`'s required components
    // cover the compile outcome; `vm::sync` initializes the VMs on the first
    // `Update`.
    for (head, graph, head_views) in loaded {
        let is_focused = focused_head.as_ref() == Some(&head);
        let entity = cmds
            .spawn((
                OpenHead,
                HeadRef(head),
                WorkingGraph(graph),
                head_views,
                HeadGuiState::default(),
            ))
            .id();

        tab_order.push(entity);

        // Set focused to the persisted focused head, or first head as fallback.
        if is_focused || (**focused).is_none() {
            **focused = Some(entity);
        }
    }
}

/// Load egui memory from storage once on first frame.
fn load_egui_memory(mut ctxs: EguiContexts, mut storage: ResMut<Pkv>, mut loaded: Local<bool>) {
    if !*loaded {
        if let Ok(ctx) = ctxs.ctx_mut() {
            bevy_gantz_egui::storage::load_egui_memory(&mut *storage, ctx);
            *loaded = true;
        }
    }
}

#[cfg(test)]
mod tests {
    const BASE_GANTZ: &[u8] = gantz_base::BYTES;

    #[test]
    fn base_gantz_deserializes() {
        let _export: gantz_egui::export::Export<
            gantz_core::node::graph::Graph<Box<dyn super::node::Node>>,
        > = gantz_egui::export::parse_export(BASE_GANTZ).expect("valid .gantz");
    }
}
