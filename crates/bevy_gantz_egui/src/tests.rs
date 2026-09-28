use bevy_app::{App, TaskPoolPlugin, Update};
use bevy_ecs::prelude::IntoScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;
use bevy_gantz::{EntrypointSet, GantzPlugin, VmSet};

mod await_;
mod load_bang;

/// The test app's `.gantz` sugar carrier. The codec macro requires it. The
/// tests never parse text.
struct NodeSet;

impl gantz_format::NodeSugar for NodeSet {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![&gantz_format::CoreSugar])
    }
}

/// The codec over the tests' node set, through which the runtime's
/// reified-graph cache serves stored graphs as typed nodes.
fn codec() -> gantz_egui::node::NodeCodec {
    gantz_egui::ui_node_codec! {
        NodeSet {
            crate::node::Await,
            crate::node::LoadBang,
            crate::node::Sleep,
            gantz_egui::node::Inspect,
        }
    }
}

/// A headless app with the gantz plugin, the test codec, the `vm::sync`
/// system over the crate's entrypoints, and the given entrypoint driver. The
/// minimal plumbing a driver needs.
fn test_app<M>(driver: impl IntoScheduleConfigs<ScheduleSystem, M>) -> App {
    let mut app = App::new();
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(GantzPlugin)
        .insert_resource(crate::NodeCodecRes(codec()))
        .init_resource::<crate::GraphCache>()
        .init_resource::<crate::BuiltinNodes>()
        .add_systems(
            Update,
            (
                crate::vm::sync.in_set(VmSet),
                driver.after(VmSet).in_set(EntrypointSet),
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
