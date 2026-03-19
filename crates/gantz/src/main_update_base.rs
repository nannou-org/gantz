//! Developer tool for authoring base nodes.
//!
//! Starts with the registry populated from `base/base.gantz`. GUI state
//! (open heads, views, egui memory) is persisted under a separate
//! `PkvStore` so it never collides with the main gantz binary's storage.
//! On every debounced input event, named graphs are exported back to
//! `base/base.gantz` and GUI state is saved.
//!
//! Usage: `cargo run -p gantz --bin update-base`

use bevy::{prelude::*, window::Window};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_gantz::{
    BuiltinNodes, CompiledModule, FocusedHead, GantzPlugin, HeadRef, HeadTabOrder, OpenHead,
    OpenHeadDataReadOnly, WorkingGraph,
    debounced_input::{DebouncedInputEvent, DebouncedInputPlugin},
    timestamp, vm,
};
use bevy_gantz_egui::{GantzEguiPlugin, GuiState, HeadGuiState, TraceCapture, Views};
use bevy_pkv::PkvStore;
use builtin::Builtins;
use storage::Pkv;

mod builtin;
mod node;
mod storage;

fn main() {
    App::new()
        .add_plugins(GantzPlugin::<Box<dyn node::Node>>::default())
        .add_plugins(GantzEguiPlugin::<Box<dyn node::Node>>::default().base_immutable(false))
        .insert_resource(BuiltinNodes::<Box<dyn node::Node>>(Box::new(
            Builtins::new(),
        )))
        .add_plugins(DefaultPlugins.set(log_plugin()).set(window_plugin()))
        .add_plugins(EguiPlugin::default())
        .add_plugins(DebouncedInputPlugin::new(0.25))
        .insert_resource(Pkv(PkvStore::new("nannou-org", "gantz-update-base")))
        .insert_resource(bevy_gantz_egui::base::ExportPath(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../base/base.gantz"
        )))
        .add_systems(
            Startup,
            (
                setup_camera,
                setup_gui_state,
                bevy_gantz_egui::base::load::<Box<dyn node::Node>>.after(setup_gui_state),
                setup_open.after(bevy_gantz_egui::base::load::<Box<dyn node::Node>>),
                vm::setup::<Box<dyn node::Node>>.after(setup_open),
            ),
        )
        .add_systems(EguiPrimaryContextPass, load_egui_memory)
        .add_systems(
            Update,
            (
                bevy_gantz_egui::base::export_to_file::<Box<dyn node::Node>>,
                persist_state,
            )
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

fn setup_gui_state(storage: Res<Pkv>, mut cmds: Commands) {
    let views = bevy_gantz_egui::storage::load_views(&*storage);
    let gui_state = bevy_gantz_egui::storage::load_gui_state(&*storage);
    cmds.insert_resource(views);
    cmds.insert_resource(gui_state);
}

fn setup_open(
    storage: Res<Pkv>,
    mut registry: ResMut<bevy_gantz::Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
    mut cmds: Commands,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
) {
    let loaded =
        bevy_gantz_egui::storage::load_open(&*storage, &mut *registry, &*views, timestamp());
    let focused_head = bevy_gantz::storage::load_focused_head(&*storage);

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

        if is_focused || (**focused).is_none() {
            **focused = Some(entity);
        }
    }
}

fn load_egui_memory(mut ctxs: EguiContexts, mut storage: ResMut<Pkv>, mut loaded: Local<bool>) {
    if !*loaded {
        if let Ok(ctx) = ctxs.ctx_mut() {
            bevy_gantz_egui::storage::load_egui_memory(&mut *storage, ctx);
            *loaded = true;
        }
    }
}

fn persist_state(
    views: Res<Views>,
    gui_state: Res<GuiState>,
    mut storage: ResMut<Pkv>,
    mut ctxs: EguiContexts,
    tab_order: Res<HeadTabOrder>,
    focused: Res<FocusedHead>,
    heads_query: Query<OpenHeadDataReadOnly<Box<dyn node::Node>>, With<OpenHead>>,
) {
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
    // Save views.
    bevy_gantz_egui::storage::save_views(&mut *storage, &*views);
    // Save GUI state.
    bevy_gantz_egui::storage::save_gui_state(&mut *storage, &gui_state);
    // Save egui memory (widget states, tile layouts).
    if let Ok(ctx) = ctxs.ctx_mut() {
        bevy_gantz_egui::storage::save_egui_memory(&mut *storage, ctx);
    }
}
