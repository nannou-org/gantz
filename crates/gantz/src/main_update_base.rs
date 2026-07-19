//! Developer tool for authoring base nodes.
//!
//! Starts with the registry populated from `base/base.gantz`. GUI state
//! (open heads, egui memory) is persisted under a separate
//! `PkvStore` so it never collides with the main gantz binary's storage.
//! On every debounced input event, named graphs are exported back to
//! `base/base.gantz` and GUI state is saved.
//!
//! Usage: `cargo run -p gantz --bin update-base`

use bevy::{prelude::*, window::Window};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_gantz::{
    FocusedHead, GantzPlugin, HeadRef, HeadTabOrder, OpenHead, OpenHeadDataReadOnly, WorkingGraph,
    debounced_input::{DebouncedInputEvent, DebouncedInputPlugin},
    timestamp,
};
use bevy_gantz_egui::{BuiltinNodes, GantzEguiPlugin, GuiState, HeadGuiState, TraceCapture};
use bevy_pkv::PkvStore;
use storage::Pkv;

mod node;
mod storage;

fn main() {
    App::new()
        .add_plugins(GantzPlugin)
        .add_plugins(GantzEguiPlugin::default().base_immutable(false))
        // The DSP plugin contributes the plyphon base source, and lets DSP
        // demos be heard while they are edited.
        .add_plugins(bevy_gantz_plyphon::PlyphonPlugin::default())
        .insert_resource({
            let (builtins, errs) = BuiltinNodes::reify(node::builtins(), &node::codec());
            assert!(errs.is_empty(), "builtins failed to reify: {errs:?}");
            builtins
        })
        // The app's value-level node codec, for the `.gantz` parse/export
        // paths (base load, import/export, clipboard, write-back).
        .insert_resource(bevy_gantz_egui::NodeCodecRes(node::codec()))
        .add_plugins(DefaultPlugins.set(log_plugin()).set(window_plugin()))
        .add_plugins(EguiPlugin::default())
        .add_plugins(DebouncedInputPlugin::<DebouncedInputEvent>::new(0.25))
        .insert_resource(Pkv::new(PkvStore::new("nannou-org", "gantz-update-base")))
        // Each base source writes back to its own crate's file. Graphs
        // created in this session (no recorded source) land in the core file.
        .insert_resource(bevy_gantz_egui::base::ExportPaths {
            paths: [
                (
                    "gantz",
                    concat!(env!("CARGO_MANIFEST_DIR"), "/../gantz_base/base.gantz"),
                ),
                (
                    "plyphon",
                    concat!(env!("CARGO_MANIFEST_DIR"), "/../gantz_plyphon/base.gantz"),
                ),
            ]
            .into_iter()
            .collect(),
            default_source: "gantz",
        })
        .add_systems(
            Startup,
            (
                setup_camera,
                setup_gui_state,
                bevy_gantz_egui::base::load.after(setup_gui_state),
                setup_open.after(bevy_gantz_egui::base::load),
            ),
        )
        .add_systems(EguiPrimaryContextPass, load_egui_memory)
        .add_systems(
            Update,
            (bevy_gantz_egui::base::export_to_file, persist_state)
                // After `settle_layout` so a layout commit settled this frame
                // (and its seeded view) is exported/saved in the same pass.
                .after(bevy_gantz_egui::settle_layout)
                .run_if(on_message::<DebouncedInputEvent>),
        )
        .run();
}

fn log_plugin() -> bevy::log::LogPlugin {
    bevy::log::LogPlugin {
        custom_layer: move |app| {
            // `get_resource_or_init`: this closure runs while `DefaultPlugins`
            // builds, and `GantzEguiPlugin`s later idempotent `init_resource`
            // shares the instance - so plugin order does not matter.
            let capture = app.world_mut().get_resource_or_init::<TraceCapture>();
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
    let gui_state = bevy_gantz_egui::storage::load_gui_state(&*storage);
    cmds.insert_resource(gui_state);
}

fn setup_open(
    storage: Res<Pkv>,
    mut registry: ResMut<bevy_gantz::Registry>,
    mut cmds: Commands,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
) {
    let loaded = bevy_gantz_egui::storage::load_open(&*storage, &mut *registry, timestamp());
    let focused_head = bevy_gantz::storage::load_focused_head(&*storage);

    // `OpenHead`'s required components cover the compile outcome; `vm::sync`
    // initializes the VMs on the first `Update`.
    for (head, graph, head_view) in loaded {
        let is_focused = focused_head.as_ref() == Some(&head);
        let entity = cmds
            .spawn((
                OpenHead,
                HeadRef(head),
                WorkingGraph(graph),
                head_view,
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
    gui_state: Res<GuiState>,
    mut storage: ResMut<Pkv>,
    mut ctxs: EguiContexts,
    tab_order: Res<HeadTabOrder>,
    focused: Res<FocusedHead>,
    heads_query: Query<OpenHeadDataReadOnly, With<OpenHead>>,
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
    // Save GUI state.
    bevy_gantz_egui::storage::save_gui_state(&mut *storage, &gui_state);
    // Save egui memory (widget states, tile layouts).
    if let Ok(ctx) = ctxs.ctx_mut() {
        bevy_gantz_egui::storage::save_egui_memory(&mut *storage, ctx);
    }
}
