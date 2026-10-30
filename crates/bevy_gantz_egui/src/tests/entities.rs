//! The node entity mirror. See `crate::entities`.

use crate::node::Await;
use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_gantz::node::{NodeOf, NodePath, PushEntry};
use bevy_gantz::{Registry, head, timestamp};
use gantz_ca::{DataGraph, Head, Name};
use gantz_core::data::erase_node_typed;
use gantz_egui::node::Inspect;
use petgraph::graph::NodeIndex;

fn name() -> Name {
    "g".parse().unwrap()
}

/// Commit `graph` to the branch `g`.
fn commit(app: &mut App, graph: DataGraph) {
    let addr = gantz_ca::graph_addr(&graph);
    app.world_mut()
        .resource_mut::<Registry>()
        .commit_graph_to_name(timestamp(), addr, move || graph, &name());
    super::refresh_app_cache(app);
}

/// An `inspect` node followed by `n_awaits` `await` nodes.
fn graph(n_awaits: usize) -> DataGraph {
    let mut graph = DataGraph::default();
    graph.add_node(erase_node_typed(&Inspect).unwrap());
    for _ in 0..n_awaits {
        graph.add_node(erase_node_typed(&Await).unwrap());
    }
    graph
}

/// Each node entity with its path and whether it holds an `Await` and a push
/// entrypoint, sorted by path.
fn nodes(app: &mut App) -> Vec<(Entity, Vec<usize>, bool, bool)> {
    let mut q = app
        .world_mut()
        .query::<(Entity, &NodePath, Has<Await>, Has<PushEntry>)>();
    let mut nodes: Vec<_> = q
        .iter(app.world())
        .map(|(e, path, is_await, push)| (e, path.0.clone(), is_await, push))
        .collect();
    nodes.sort_by(|a, b| a.1.cmp(&b.1));
    nodes
}

fn open(app: &mut App) -> Entity {
    app.world_mut()
        .trigger(head::OpenEvent(Head::Branch(name())));
    app.update();
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<head::OpenHead>>();
    q.single(app.world()).expect("one open head")
}

/// Each node of an open head's graph is an entity of the head, and a
/// registered node type is a component on its entity.
#[test]
fn nodes_of_an_open_head_are_entities() {
    let mut app = super::test_app(|| {});
    commit(&mut app, graph(2));
    let head = open(&mut app);
    let nodes = nodes(&mut app);
    let shape: Vec<_> = nodes
        .iter()
        .map(|(_, path, is_await, push)| (path.clone(), *is_await, *push))
        .collect();
    assert_eq!(
        shape,
        vec![
            (vec![0], false, false),
            (vec![1], true, true),
            (vec![2], true, true),
        ]
    );
    for (entity, ..) in nodes {
        assert_eq!(app.world().get::<NodeOf>(entity), Some(&NodeOf(head)));
    }
}

/// An edit keeps the entities of the nodes it keeps, at their new paths. The
/// removed node's entity despawns. Equal nodes keep their own entities.
#[test]
fn an_edit_keeps_the_entities_of_kept_nodes() {
    let mut app = super::test_app(|| {});
    commit(&mut app, graph(2));
    let head = open(&mut app);
    let before = nodes(&mut app);

    // Removing node 1 swaps node 2 into its index. The editor migrates the
    // node state the same way, see `gantz_egui::ops::remove_nodes`.
    let mut edited = graph(2);
    edited.remove_node(NodeIndex::new(1));
    let mut vms = app.world_mut().non_send_mut::<head::HeadVms>();
    let vm = vms.get_mut(&head).expect("head VM");
    gantz_core::node::state::remove_value(vm, &[1]).unwrap();
    gantz_core::node::state::move_value(vm, &[2], &[1]).unwrap();
    commit(&mut app, edited);
    app.update();
    let after = nodes(&mut app);

    let ids = |nodes: &[(Entity, Vec<usize>, bool, bool)]| -> Vec<_> {
        nodes
            .iter()
            .map(|(e, path, ..)| (*e, path.clone()))
            .collect()
    };
    assert_eq!(
        ids(&after),
        vec![(before[0].0, vec![0]), (before[2].0, vec![1])]
    );
    assert!(app.world().get_entity(before[1].0).is_err());
}

/// Despawning a head despawns its node entities.
#[test]
fn a_despawned_head_despawns_its_nodes() {
    let mut app = super::test_app(|| {});
    commit(&mut app, graph(1));
    let head = open(&mut app);
    assert_eq!(nodes(&mut app).len(), 2);
    app.world_mut().entity_mut(head).despawn();
    assert!(nodes(&mut app).is_empty());
}

/// A node whose type changes at a path that keeps its state gets a new
/// entity, and the entity of the old type despawns.
#[test]
fn a_new_type_at_a_kept_path_replaces_the_entity() {
    let mut app = super::test_app(|| {});
    commit(&mut app, graph(1));
    open(&mut app);
    let before = nodes(&mut app);

    let mut edited = DataGraph::default();
    edited.add_node(erase_node_typed(&Inspect).unwrap());
    edited.add_node(erase_node_typed(&Inspect).unwrap());
    commit(&mut app, edited);
    app.update();
    let after = nodes(&mut app);

    assert_eq!(after.len(), 2);
    assert_eq!(after[0].0, before[0].0, "the kept node keeps its entity");
    assert_ne!(after[1].0, before[1].0, "the new type gets a new entity");
    assert!(!after[1].2, "the new entity holds no `Await`");
    assert!(app.world().get_entity(before[1].0).is_err());
}
