//! The actions the `Gantz` widget emits for the app arrive as payloads, and
//! the plugin's dispatchers and observers apply them.

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_ecs::world::CommandQueue;
use bevy_gantz::{Registry, head, timestamp};
use gantz_ca::{DataGraph, Head, Name};
use gantz_egui::{DynResponse, ResponseData};
use std::any::TypeId;

/// A headless app with the GUI plugin. Dispatch triggers observers at once,
/// so the tests never run the GUI schedule.
fn app() -> App {
    let mut app = super::test_app(|| {});
    app.add_plugins(crate::GantzEguiPlugin::default());
    app
}

/// Dispatch `payload` as the `update` system does after a GUI pass.
fn dispatch<T: ResponseData>(app: &mut App, payload: T) {
    let world = app.world_mut();
    let dispatchers = &world.resource::<crate::ResponseDispatchers>().0;
    let dispatch = dispatchers[&TypeId::of::<T>()];
    let mut queue = CommandQueue::default();
    let mut cmds = Commands::new(&mut queue, world);
    dispatch(None, DynResponse::new(payload), &mut cmds);
    queue.apply(world);
}

fn registered<T: 'static>(app: &App) -> bool {
    let dispatchers = &app.world().resource::<crate::ResponseDispatchers>().0;
    dispatchers.contains_key(&TypeId::of::<T>())
}

/// Commit an empty graph to `name`.
fn commit_named(app: &mut App, name: &Name) -> gantz_ca::CommitAddr {
    let graph = DataGraph::default();
    let ga = gantz_ca::graph_addr(&graph);
    let mut registry = app.world_mut().resource_mut::<Registry>();
    let commit = registry.commit_graph(timestamp(), None, ga, move || graph);
    registry.set_head(name.clone(), commit);
    commit
}

fn open_heads(app: &mut App) -> Vec<Head> {
    let world = app.world_mut();
    let mut q = world.query_filtered::<&head::HeadRef, With<head::OpenHead>>();
    q.iter(world).map(|h| (**h).clone()).collect()
}

#[test]
fn every_widget_action_has_a_dispatcher() {
    let app = app();
    assert!(registered::<gantz_egui::OpenHead>(&app));
    assert!(registered::<gantz_egui::ReplaceHead>(&app));
    assert!(registered::<gantz_egui::CloseHead>(&app));
    assert!(registered::<gantz_egui::NewGraph>(&app));
    assert!(registered::<gantz_egui::BranchHead>(&app));
    assert!(registered::<gantz_egui::RemoveName>(&app));
    assert!(registered::<gantz_egui::Import>(&app));
    assert!(registered::<gantz_egui::widget::gantz::FileDrop>(&app));
    assert!(registered::<gantz_egui::SetDescription>(&app));
    assert!(registered::<gantz_egui::SetDemo>(&app));
    assert!(registered::<gantz_egui::SetBaseSource>(&app));
    assert!(registered::<gantz_egui::ResetBaseGraph>(&app));
    assert!(registered::<gantz_egui::ResetAllDemos>(&app));
    assert!(registered::<gantz_core::compile::Config>(&app));
    assert!(registered::<gantz_egui::ValidateChangeTracking>(&app));
}

#[test]
fn a_named_graph_takes_a_description_and_a_demo() {
    let mut app = app();
    let jam: Name = "jam".parse().unwrap();
    commit_named(&mut app, &jam);
    let description = "a jam".to_string();
    dispatch(
        &mut app,
        gantz_egui::SetDescription {
            name: jam.clone(),
            description,
        },
    );
    let demo = Some("demo-jam".to_string());
    dispatch(
        &mut app,
        gantz_egui::SetDemo {
            name: jam.clone(),
            demo,
        },
    );

    let registry = app.world().resource::<Registry>();
    let described = gantz_edit::section::description(registry, &jam);
    assert_eq!(described.as_deref(), Some("a jam"));
    let demo = gantz_edit::section::demo(registry, &jam);
    assert_eq!(demo.as_deref(), Some("demo-jam"));

    dispatch(
        &mut app,
        gantz_egui::SetDemo {
            name: jam.clone(),
            demo: None,
        },
    );
    let registry = app.world().resource::<Registry>();
    assert_eq!(gantz_edit::section::demo(registry, &jam), None);
}

/// A removed name's open head stays open on the name's last commit.
#[test]
fn removing_a_name_keeps_its_open_head() {
    let mut app = app();
    let jam: Name = "jam".parse().unwrap();
    let commit = commit_named(&mut app, &jam);
    let world = app.world_mut();
    world.trigger(head::OpenEvent(Head::Branch(jam.clone())));
    world.flush();
    assert_eq!(open_heads(&mut app), vec![Head::Branch(jam.clone())]);

    dispatch(&mut app, gantz_egui::RemoveName(jam.clone()));
    assert_eq!(open_heads(&mut app), vec![Head::Commit(commit)]);
    assert_eq!(app.world().resource::<Registry>().head(&jam), None);
}

#[test]
fn a_new_graph_opens() {
    let mut app = app();
    assert!(open_heads(&mut app).is_empty());
    dispatch(&mut app, gantz_egui::NewGraph);
    assert_eq!(open_heads(&mut app).len(), 1);
}

#[test]
fn settings_changes_reach_their_resources() {
    let mut app = app();
    let config = gantz_core::compile::Config {
        validate_ir: false,
        ..Default::default()
    };
    dispatch(&mut app, config);
    let applied = app.world().resource::<bevy_gantz::CompileConfig>().0;
    assert_eq!(applied, config);

    dispatch(&mut app, gantz_egui::ValidateChangeTracking(false));
    assert!(!app.world().resource::<bevy_gantz::ValidateCommitted>().0);
    dispatch(&mut app, gantz_egui::ValidateChangeTracking(true));
    assert!(app.world().resource::<bevy_gantz::ValidateCommitted>().0);
}
