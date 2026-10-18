//! A layout-only commit needs a layout edit. See `settle_layout`.

use bevy_app::{App, Update};
use bevy_ecs::prelude::*;
use bevy_egui::egui;
use bevy_gantz::{Registry, head, timestamp};
use gantz_ca::{CommitAddr, DataGraph, Head, Name};
use gantz_core::data::erase_node_typed;
use gantz_edit::NodeId;
use gantz_egui::node::Inspect;

fn name() -> Name {
    "g".parse().unwrap()
}

/// Commit a graph of `n` nodes to the registry, with a stored view that
/// places each node.
fn commit(app: &mut App, n: usize) -> CommitAddr {
    let mut registry = app.world_mut().resource_mut::<Registry>();
    let mut graph = DataGraph::default();
    let mut view = gantz_edit::SceneView::default();
    for ix in 0..n {
        graph.add_node(erase_node_typed(&Inspect).unwrap());
        let pos = egui::pos2(ix as f32 * 100.0, 0.0);
        view.layout.insert(NodeId::from_index(ix), pos);
    }
    let addr = gantz_ca::graph_addr(&graph);
    let commit = registry.commit_graph(timestamp(), None, addr, move || graph);
    gantz_edit::section::set_view(&mut registry.0, commit, &view);
    commit
}

/// An app with the view systems and head hooks, and the branch `g` open on a
/// two-node graph. `settle_layout` runs every update. Returns the head's
/// entity.
fn open_app() -> (App, Entity) {
    let mut app = super::test_app(|| {});
    app.init_resource::<crate::GuiState>()
        .add_observer(crate::on_head_opened)
        .add_observer(crate::on_head_changed)
        .add_systems(
            Update,
            (
                crate::persist_camera_and_seed,
                crate::settle_layout.after(crate::persist_camera_and_seed),
            ),
        );
    let c = commit(&mut app, 2);
    app.world_mut()
        .resource_mut::<Registry>()
        .set_head(name(), c);
    super::refresh_app_cache(&mut app);
    app.world_mut()
        .trigger(head::OpenEvent(Head::Branch(name())));
    app.update();
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<head::OpenHead>>();
    let entity = q.single(app.world()).expect("one open head");
    (app, entity)
}

fn head_commit(app: &App) -> CommitAddr {
    app.world().resource::<Registry>().head(&name()).unwrap()
}

/// Move the head's first node in its live layout.
fn move_node(app: &mut App, entity: Entity) {
    let mut view = app.world_mut().get_mut::<crate::GraphView>(entity).unwrap();
    view.0.layout.insert(NodeId(0), egui::pos2(10.0, 20.0));
}

fn marked(app: &App, entity: Entity) -> bool {
    app.world().entity(entity).contains::<crate::LayoutEdited>()
}

/// A live layout that differs from the baseline with no edit, for example
/// after the scene snaps a position, mints no commit.
#[test]
fn a_difference_without_an_edit_mints_no_commit() {
    let (mut app, entity) = open_app();
    let before = head_commit(&app);
    move_node(&mut app, entity);
    app.update();
    assert_eq!(head_commit(&app), before);
}

/// An edit that moved a node mints one layout commit on the same graph and
/// clears the marker.
#[test]
fn an_edit_mints_one_layout_commit() {
    let (mut app, entity) = open_app();
    let before = head_commit(&app);
    move_node(&mut app, entity);
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::LayoutEdited);
    app.update();
    let after = head_commit(&app);
    assert_ne!(after, before);
    assert!(!marked(&app, entity));
    let registry = app.world().resource::<Registry>();
    let commits = registry.commits();
    assert_eq!(commits[&after].parent, Some(before));
    assert_eq!(commits[&after].graph, commits[&before].graph);
    app.update();
    assert_eq!(head_commit(&app), after, "one commit per edit");
}

/// An edit that left every node in place only clears the marker.
#[test]
fn an_edit_without_a_difference_only_clears_the_marker() {
    let (mut app, entity) = open_app();
    let before = head_commit(&app);
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::LayoutEdited);
    app.update();
    assert_eq!(head_commit(&app), before);
    assert!(!marked(&app, entity));
}

/// Moving the head replaces its layout, so the move clears an edit made
/// before it.
#[test]
fn a_head_move_clears_the_marker() {
    let (mut app, entity) = open_app();
    let target = commit(&mut app, 3);
    super::refresh_app_cache(&mut app);
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::LayoutEdited);
    app.world_mut()
        .trigger(head::MoveHeadEvent { entity, target });
    app.world_mut().flush();
    assert_eq!(head_commit(&app), target, "setup");
    assert!(!marked(&app, entity));
}
