use crate::NodeKindAppExt;
use bevy_app::{App, TaskPoolPlugin, Update};
use bevy_ecs::prelude::IntoScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;
use bevy_gantz::{EntrypointSet, GantzPlugin, NodeSet, VmSet};

mod app_response;
mod await_;
mod drop_unknown;
mod entities;
mod layout_settle;
mod load_bang;
mod node_state;
mod undo_resync;

/// The codec over the tests' node set, through which the runtime's
/// reified-graph cache serves stored graphs as typed nodes.
fn codec() -> gantz_egui::node::NodeCodec {
    gantz_egui::node::NodeCodec::new(
        gantz_format::Sugars::default(),
        [
            gantz_egui::node::kind::<crate::node::Await>(),
            gantz_egui::node::kind::<crate::node::LoadBang>(),
            gantz_egui::node::kind::<crate::node::Sleep>(),
            gantz_egui::node::kind::<gantz_egui::node::Inspect>(),
            gantz_egui::node::kind::<gantz_egui::node::State>(),
            gantz_egui::node::kind::<gantz_edit::NamedRef>(),
        ],
    )
}

/// A headless app with the gantz plugin, the test codec, the `vm::sync`
/// system over the crate's entrypoints, the node entity mirror and the given
/// entrypoint driver. The minimal plumbing a driver needs.
fn test_app<M>(driver: impl IntoScheduleConfigs<ScheduleSystem, M>) -> App {
    let mut app = App::new();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(GantzPlugin)
        .insert_resource(crate::NodeCodecRes(codec()))
        .init_resource::<crate::GraphCache>()
        .init_resource::<crate::BuiltinNodes>()
        .init_resource::<crate::NodeKinds>()
        .add_node_kind::<crate::node::Await>()
        .add_node_kind::<crate::node::LoadBang>()
        .add_systems(
            Update,
            (
                crate::vm::sync.in_set(VmSet),
                crate::entities::mirror.in_set(NodeSet),
                driver.in_set(EntrypointSet),
            ),
        );
    app.world_mut()
        .get_resource_or_init::<crate::vm::EntrypointFns>()
        .0
        .push(Box::new(crate::node::entrypoints));
    app
}

/// Reify the registry's committed graphs into the app's graph cache.
fn refresh_app_cache(app: &mut App) {
    app.world_mut()
        .resource_scope::<crate::GraphCache, _>(|world, mut cache| {
            let registry = world.resource::<bevy_gantz::Registry>();
            crate::refresh_cache(registry, &mut cache, &codec());
        });
}
