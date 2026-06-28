use bevy::{
    prelude::*,
    window::{PrimaryWindow, Window},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_gantz::{
    BuiltinNodes, FocusedHead, GantzPlugin, HeadRef, HeadTabOrder, OpenHead, OpenHeadDataReadOnly,
    Registry, WorkingGraph,
    debounced_input::{DebouncedInputEvent, DebouncedInputPlugin},
    reg, timestamp,
};
use bevy_gantz_egui::{GantzEguiPlugin, GuiState, HeadGuiState, TraceCapture, Views};
use bevy_pkv::PkvStore;
use builtin::Builtins;
use storage::Pkv;

mod builtin;
mod node;
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
        .add_plugins(DebouncedInputPlugin::new(0.25))
        .insert_resource(Pkv(PkvStore::new("nannou-org", "gantz")))
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
        .add_systems(
            Update,
            persist_resources
                // After `settle_layout` so a layout commit settled this frame
                // (and its seeded view) is saved in the same pass.
                .after(bevy_gantz_egui::settle_layout::<Box<dyn node::Node>>)
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

fn persist_resources(
    registry: Res<Registry<Box<dyn node::Node>>>,
    mut persisted: ResMut<bevy_gantz::storage::PersistedRegistry>,
    views: Res<Views>,
    gui_state: Res<GuiState>,
    mut storage: ResMut<Pkv>,
    mut ctxs: EguiContexts,
    tab_order: Res<HeadTabOrder>,
    focused: Res<FocusedHead>,
    heads_query: Query<OpenHeadDataReadOnly<Box<dyn node::Node>>, With<OpenHead>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
) {
    // Incrementally save the registry (only newly-seen graphs/commits and any
    // changed name maps). `persist_resources` runs only on debounced events, so
    // `is_changed()` here means "changed since the last persist" - a cheap guard
    // for the idle case; the per-blob dedup is what removes the spike.
    if registry.is_changed() {
        bevy_gantz::storage::save_registry_incremental(&mut *storage, &registry, &mut persisted);
    }
    // Save all open heads in tab order.
    let heads: Vec<_> = tab_order
        .iter()
        .filter_map(|&entity| {
            heads_query
                .get(entity)
                .ok()
                .map(|data| (**data.head_ref).clone())
        })
        .collect();
    bevy_gantz::storage::save_open_heads(&mut *storage, &heads);
    // Save the focused head.
    if let Some(focused_entity) = **focused {
        if let Ok(data) = heads_query.get(focused_entity) {
            bevy_gantz::storage::save_focused_head(&mut *storage, &**data.head_ref);
        }
    }

    // Save all views (kept up to date by `persist_camera_and_seed` and
    // `settle_layout`).
    bevy_gantz_egui::storage::save_views(&mut *storage, &*views);
    // Save the gantz GUI state.
    bevy_gantz_egui::storage::save_gui_state(&mut *storage, &gui_state);
    // Save egui memory (widget states).
    if let Ok(ctx) = ctxs.ctx_mut() {
        bevy_gantz_egui::storage::save_egui_memory(&mut *storage, ctx);
    }
    // Save the native window size (no-op on web).
    if let Ok(window) = primary_window.single() {
        window::save(&mut *storage, window);
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
