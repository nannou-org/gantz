use bevy::{
    prelude::*,
    window::{Window, WindowPlugin},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_gantz::{
    BuiltinNodes, CompiledModule, FocusedHead, GantzPlugin, HeadRef, HeadTabOrder, OpenHead,
    OpenHeadDataReadOnly, Registry, WorkingGraph,
    debounced_input::{DebouncedInputEvent, DebouncedInputPlugin},
    reg, timestamp, vm,
};
use bevy_gantz_egui::{GantzEguiPlugin, GuiState, HeadGuiState, TraceCapture, Views};
use bevy_pkv::PkvStore;
use builtin::Builtins;

mod builtin;
mod node;
mod storage;

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
        .add_plugins(DefaultPlugins.set(log_plugin()).set(window_plugin()))
        .add_plugins(EguiPlugin::default())
        .add_plugins(DebouncedInputPlugin::new(0.25))
        .insert_resource(PkvStore::new("nannou-org", "gantz"))
        .add_systems(
            Startup,
            (
                setup_camera,
                setup_resources,
                setup_open.after(setup_resources),
                reg::prune_unused::<Box<dyn node::Node>>
                    .after(setup_resources)
                    .after(setup_open),
                vm::setup::<Box<dyn node::Node>>.after(reg::prune_unused::<Box<dyn node::Node>>),
            ),
        )
        .add_systems(EguiPrimaryContextPass, load_egui_memory)
        .add_systems(
            Update,
            persist_resources.run_if(on_message::<DebouncedInputEvent>),
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

fn window_plugin() -> WindowPlugin {
    WindowPlugin {
        primary_window: Some(Window {
            title: "gantz".into(),
            name: Some("gantz".into()),
            fit_canvas_to_parent: true,
            // NOTE: This vastly improves input-latency on wayland. If you
            // notice tearing or simialr issues, open an issue so we can try and
            // select the right `PresentMode` for each system!
            present_mode: bevy::window::PresentMode::AutoNoVsync,
            ..default()
        }),
        ..default()
    }
}

fn setup_camera(mut cmds: Commands) {
    cmds.spawn(Camera2d);
}

fn setup_resources(storage: Res<PkvStore>, mut cmds: Commands) {
    let registry: Registry<Box<dyn node::Node>> = storage::load_registry(&*storage);
    let views = storage::load_views(&*storage);
    let gui_state = storage::load_gui_state(&*storage);
    cmds.insert_resource(registry);
    cmds.insert_resource(views);
    cmds.insert_resource(gui_state);
}

fn setup_open(
    storage: Res<PkvStore>,
    mut registry: ResMut<Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
    mut cmds: Commands,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
) {
    let loaded = storage::load_open(&*storage, &mut *registry, &*views, timestamp());
    let focused_head = storage::load_focused_head(&*storage);

    // Spawn entities for each open head.
    for (head, graph, head_views) in loaded {
        let is_focused = focused_head.as_ref() == Some(&head);
        let entity = cmds
            .spawn((
                OpenHead,
                HeadRef(head),
                WorkingGraph(graph),
                head_views,
                CompiledModule::default(),
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
fn load_egui_memory(
    mut ctxs: EguiContexts,
    mut storage: ResMut<PkvStore>,
    mut loaded: Local<bool>,
) {
    if !*loaded {
        if let Ok(ctx) = ctxs.ctx_mut() {
            storage::load_egui_memory(&mut *storage, ctx);
            *loaded = true;
        }
    }
}

fn persist_resources(
    registry: Res<Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
    gui_state: Res<GuiState>,
    mut storage: ResMut<PkvStore>,
    mut ctxs: EguiContexts,
    tab_order: Res<HeadTabOrder>,
    focused: Res<FocusedHead>,
    heads_query: Query<OpenHeadDataReadOnly<Box<dyn node::Node>>, With<OpenHead>>,
) {
    // Save graphs.
    let mut addrs: Vec<_> = registry.graphs().keys().copied().collect();
    addrs.sort();
    storage::save_graph_addrs(&mut *storage, &addrs);
    storage::save_graphs(&mut *storage, &registry.graphs());

    // Save commits.
    let mut addrs: Vec<_> = registry.commits().keys().copied().collect();
    addrs.sort();
    storage::save_commit_addrs(&mut *storage, &addrs);
    storage::save_commits(&mut *storage, registry.commits());

    // Save names.
    storage::save_names(&mut *storage, registry.names());

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
    storage::save_open_heads(&mut *storage, &heads);

    // Save the focused head.
    if let Some(focused_entity) = **focused {
        if let Ok(data) = heads_query.get(focused_entity) {
            storage::save_focused_head(&mut *storage, &**data.head_ref);
        }
    }

    // Save all views (already updated in update_vm).
    storage::save_views(&mut *storage, &*views);

    // Save the gantz GUI state.
    storage::save_gui_state(&mut *storage, &gui_state);

    // Save egui memory (widget states).
    if let Ok(ctx) = ctxs.ctx_mut() {
        storage::save_egui_memory(&mut *storage, ctx);
    }
}
