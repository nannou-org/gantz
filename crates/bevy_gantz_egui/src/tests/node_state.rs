//! End-to-end tests of node state across a head's life in a headless app.

use bevy_ecs::prelude::*;
use bevy_gantz::{Registry, head, timestamp};
use gantz_ca::{Head, Name};
use gantz_core::node::state::{self, Snapshots};
use gantz_egui::node::{Init, State};
use steel::SteelVal;

/// A one node graph of a state node that starts at `init`.
fn graph(init: f64) -> gantz_ca::DataGraph {
    let node = State::new(Some(Init::Number(init)));
    let mut dg = gantz_ca::DataGraph::default();
    dg.add_node(gantz_core::data::erase_node_typed(&node).unwrap());
    dg
}

fn name(s: &str) -> Name {
    s.parse().unwrap()
}

/// Commit `dg` as the head of `name`.
fn commit(app: &mut bevy_app::App, n: &str, dg: gantz_ca::DataGraph) -> Head {
    let mut registry = app.world_mut().resource_mut::<Registry>();
    let addr = gantz_ca::graph_addr(&dg);
    let parent = registry.head(&name(n));
    let ca = registry.commit_graph(timestamp(), parent, addr, move || dg);
    registry.set_head(name(n), ca);
    Head::Branch(name(n))
}

/// Commit `dg` with no name.
fn unnamed_commit(app: &mut bevy_app::App, dg: gantz_ca::DataGraph) -> gantz_ca::CommitAddr {
    let mut registry = app.world_mut().resource_mut::<Registry>();
    let addr = gantz_ca::graph_addr(&dg);
    registry.commit_graph(timestamp(), None, addr, move || dg)
}

/// Run an event and the frame that compiles what it opened.
fn trigger<E: for<'a> Event<Trigger<'a>: Default>>(app: &mut bevy_app::App, event: E) {
    app.world_mut().trigger(event);
    super::refresh_app_cache(app);
    app.update();
}

fn entity(app: &mut bevy_app::App, h: &Head) -> Entity {
    let mut q = app
        .world_mut()
        .query_filtered::<(Entity, &head::HeadRef), With<head::OpenHead>>();
    q.iter(app.world())
        .find(|(_, r)| &r.0 == h)
        .map(|(e, _)| e)
        .expect("the head is open")
}

/// The commit of the head.
fn head_commit(app: &bevy_app::App, h: &Head) -> gantz_ca::CommitAddr {
    let registry = app.world().resource::<Registry>();
    registry.head_commit_ca(h).expect("the head has a commit")
}

/// The snapshot of the commit of the head.
fn snapshot(app: &bevy_app::App, h: &Head) -> Option<state::Snapshot> {
    let registry = &app.world().resource::<Registry>().0;
    gantz_ca::section_get::<Snapshots>(registry, &head_commit(app, h))
}

/// The state of the node of the head.
fn value(app: &mut bevy_app::App, h: &Head) -> SteelVal {
    let e = entity(app, h);
    let vms = app.world().non_send::<head::HeadVms>();
    state::extract_value(&vms.0[&e], &[0]).unwrap().unwrap()
}

fn set_value(app: &mut bevy_app::App, h: &Head, val: f64) {
    let e = entity(app, h);
    let mut vms = app.world_mut().non_send_mut::<head::HeadVms>();
    let vm = vms.0.get_mut(&e).unwrap();
    state::update_value(vm, &[0], SteelVal::NumV(val)).unwrap();
}

/// An app with `alpha` and `other` open, since the last head never closes.
fn app_with_two_heads() -> (bevy_app::App, Head, Head) {
    let mut app = super::test_app(|| {});
    let alpha = commit(&mut app, "alpha", graph(1.0));
    let other = commit(&mut app, "other", graph(2.0));
    trigger(&mut app, head::OpenEvent(alpha.clone()));
    trigger(&mut app, head::OpenEvent(other.clone()));
    (app, alpha, other)
}

/// Closing a head stores its state, and opening it again restores it.
#[test]
fn a_reopened_head_keeps_its_state() {
    let (mut app, alpha, _other) = app_with_two_heads();
    assert_eq!(value(&mut app, &alpha), SteelVal::NumV(1.0));
    set_value(&mut app, &alpha, 9.0);
    trigger(&mut app, head::CloseEvent(alpha.clone()));
    trigger(&mut app, head::OpenEvent(alpha.clone()));
    assert_eq!(value(&mut app, &alpha), SteelVal::NumV(9.0));
}

/// The state rides the registry, so a new process restores it.
#[test]
fn state_survives_a_restart() {
    let (mut app, alpha, _other) = app_with_two_heads();
    set_value(&mut app, &alpha, 9.0);
    trigger(&mut app, head::CloseEvent(alpha.clone()));
    let registry = app.world().resource::<Registry>().0.clone();

    let mut app = super::test_app(|| {});
    app.world_mut().resource_mut::<Registry>().0 = registry;
    trigger(&mut app, head::OpenEvent(alpha.clone()));
    assert_eq!(value(&mut app, &alpha), SteelVal::NumV(9.0));
}

/// Loading another head into the tab stores the state of the head it leaves.
#[test]
fn a_replaced_head_keeps_its_state() {
    let (mut app, alpha, other) = app_with_two_heads();
    set_value(&mut app, &other, 7.0);
    trigger(&mut app, head::CloseEvent(alpha.clone()));
    trigger(&mut app, head::ReplaceEvent(alpha.clone()));
    let nodes = snapshot(&app, &other).expect("the snapshot of the head it left");
    assert_eq!(nodes[0].1, gantz_ca::Datum::F64(7.0));
}

/// A commit with no snapshot starts from the initial state of its graph.
#[test]
fn a_commit_with_no_snapshot_starts_from_its_initial_state() {
    let (mut app, alpha, _other) = app_with_two_heads();
    set_value(&mut app, &alpha, 9.0);
    trigger(&mut app, head::CloseEvent(alpha.clone()));
    commit(&mut app, "alpha", graph(3.0));
    trigger(&mut app, head::OpenEvent(alpha.clone()));
    assert_eq!(value(&mut app, &alpha), SteelVal::NumV(3.0));
}

/// A head with no name keeps its state too, by its commit.
#[test]
fn an_unnamed_head_keeps_its_state() {
    let mut app = super::test_app(|| {});
    let other = commit(&mut app, "other", graph(2.0));
    let unnamed = Head::Commit(unnamed_commit(&mut app, graph(1.0)));
    trigger(&mut app, head::OpenEvent(unnamed.clone()));
    trigger(&mut app, head::OpenEvent(other));
    set_value(&mut app, &unnamed, 9.0);
    trigger(&mut app, head::CloseEvent(unnamed.clone()));
    trigger(&mut app, head::OpenEvent(unnamed.clone()));
    assert_eq!(value(&mut app, &unnamed), SteelVal::NumV(9.0));
}

/// The first snapshot of a commit replaces the one of its nearest ancestor,
/// so the section does not grow with the history of a graph.
#[test]
fn a_snapshot_replaces_the_one_of_its_ancestor() {
    let (mut app, alpha, _other) = app_with_two_heads();
    trigger(&mut app, head::CloseEvent(alpha.clone()));
    assert!(snapshot(&app, &alpha).is_some());
    let first = head_commit(&app, &alpha);
    commit(&mut app, "alpha", graph(3.0));
    trigger(&mut app, head::OpenEvent(alpha.clone()));
    trigger(&mut app, head::CloseEvent(alpha.clone()));
    assert!(snapshot(&app, &alpha).is_some());
    assert!(snapshot(&app, &Head::Commit(first)).is_none());
}

/// An ancestor that a name still points to keeps its snapshot, since a fork
/// leaves that name at its commit.
#[test]
fn a_snapshot_of_a_held_commit_stays() {
    let (mut app, alpha, _other) = app_with_two_heads();
    set_value(&mut app, &alpha, 9.0);
    trigger(&mut app, head::CloseEvent(alpha.clone()));
    let first = head_commit(&app, &alpha);
    let fork = name("fork");
    app.world_mut()
        .resource_mut::<Registry>()
        .set_head(fork.clone(), first);
    commit(&mut app, "alpha", graph(3.0));
    trigger(&mut app, head::OpenEvent(alpha.clone()));
    trigger(&mut app, head::CloseEvent(alpha.clone()));
    let fork = Head::Branch(fork);
    trigger(&mut app, head::OpenEvent(fork.clone()));
    assert_eq!(value(&mut app, &fork), SteelVal::NumV(9.0));
}
