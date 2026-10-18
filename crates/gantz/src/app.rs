//! Setup and persistence shared by the `gantz` and `update-base` binaries.

use crate::storage::Pkv;
use bevy::prelude::*;
use bevy_egui::EguiContexts;
use bevy_gantz::{
    FocusedHead, HeadRef, HeadTabOrder, OpenHead, OpenHeadDataReadOnly, Registry, WorkingGraph,
    timestamp,
};
use bevy_gantz_egui::TraceCapture;
use gantz_store::Save;

/// The log plugin, with a layer that feeds the GUI's trace capture.
pub fn log_plugin() -> bevy::log::LogPlugin {
    bevy::log::LogPlugin {
        custom_layer: move |app| {
            // `get_resource_or_init` shares the instance with
            // `GantzEguiPlugin`'s `init_resource`, so plugin order does not
            // matter.
            let capture = app.world_mut().get_resource_or_init::<TraceCapture>();
            Some(Box::new(capture.0.clone().layer()))
        },
        ..Default::default()
    }
}

pub fn setup_camera(mut cmds: Commands) {
    cmds.spawn(Camera2d);
}

/// Spawn the saved open heads and focus the saved focused head.
pub fn setup_open(
    storage: Res<Pkv>,
    mut registry: ResMut<Registry>,
    mut cmds: Commands,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
) {
    let loaded = bevy_gantz_egui::storage::load_open(&*storage, &mut *registry, timestamp());
    let focused_head = gantz_store::load_focused_head(&*storage);

    // `OpenHead`'s required components cover the compile outcome. `GraphView`'s
    // cover the rest of the per-head GUI state. `vm::sync` initializes the VMs
    // on the first `Update`.
    for (head, graph, head_view) in loaded {
        let is_focused = focused_head.as_ref() == Some(&head);
        let entity = cmds
            .spawn((OpenHead, HeadRef(head), WorkingGraph(graph), head_view))
            .id();

        tab_order.push(entity);

        if is_focused || (**focused).is_none() {
            **focused = Some(entity);
        }
    }
}

/// Load egui memory from storage once on first frame.
pub fn load_egui_memory(mut ctxs: EguiContexts, mut storage: ResMut<Pkv>, mut loaded: Local<bool>) {
    if !*loaded {
        if let Ok(ctx) = ctxs.ctx_mut() {
            bevy_gantz_egui::storage::load_egui_memory(&mut *storage, ctx);
            *loaded = true;
        }
    }
}

/// Save the open heads in tab order and the focused head.
pub fn save_open_heads(
    storage: &mut impl Save,
    tab_order: &HeadTabOrder,
    focused: &FocusedHead,
    heads_query: &Query<OpenHeadDataReadOnly, With<OpenHead>>,
) {
    let heads: Vec<_> = tab_order
        .iter()
        .filter_map(|&entity| {
            heads_query
                .get(entity)
                .ok()
                .map(|data| (**data.head_ref).clone())
        })
        .collect();
    gantz_store::save_open_heads(storage, &heads);
    if let Some(focused_entity) = **focused {
        if let Ok(data) = heads_query.get(focused_entity) {
            gantz_store::save_focused_head(storage, &**data.head_ref);
        }
    }
}
