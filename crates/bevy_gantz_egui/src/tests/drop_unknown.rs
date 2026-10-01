//! Discarding a node's unrecognised settings is an edit, so undo restores
//! them.

use bevy_ecs::prelude::*;
use bevy_gantz::{Registry, head, timestamp};
use gantz_ca::{DataGraph, Datum, Head, Name, NodeData};
use gantz_core::data::erase_node_typed;
use gantz_egui::node::NamedRef;

/// The first node of `name`'s head graph.
fn first_node(app: &bevy_app::App, name: &Name) -> NodeData {
    let registry = app.world().resource::<Registry>();
    let commit = registry
        .head_commit(&Head::Branch(name.clone()))
        .expect("head commit");
    let graph = registry.graph(&commit.graph).expect("head graph");
    graph.node_weights().next().expect("a node").clone()
}

#[test]
fn discarding_unrecognised_settings_commits_and_undo_restores_them() {
    let mut app = super::test_app(|| {});
    app.init_resource::<crate::GuiState>()
        .add_observer(crate::on_drop_unknown_data)
        .add_observer(crate::on_undo);

    // A ref node that a newer gantz wrote with a setting this build lacks.
    let jam: Name = "jam".parse().unwrap();
    let target = gantz_ca::ContentAddr::from([7; 32]);
    let named_ref = NamedRef::new("riff".parse().unwrap(), gantz_core::node::Ref::new(target));
    let known = erase_node_typed(&named_ref).unwrap();
    let mut stored = known.clone();
    let Datum::Map(entries) = &mut stored.data else {
        panic!("a map");
    };
    entries.push(("future".to_string(), Datum::Bool(true)));
    stored.canonicalize();
    {
        let mut graph = DataGraph::default();
        graph.add_node(stored.clone());
        let ga = gantz_ca::graph_addr(&graph);
        let mut registry = app.world_mut().resource_mut::<Registry>();
        let commit = registry.commit_graph(timestamp(), None, ga, move || graph);
        registry.set_head(jam.clone(), commit);
    }
    super::refresh_app_cache(&mut app);
    app.world_mut()
        .trigger(head::OpenEvent(Head::Branch(jam.clone())));
    app.update();
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<head::OpenHead>>();
    let e = q.single(app.world()).expect("one open head");

    let node = petgraph::graph::NodeIndex::new(0);
    app.world_mut().trigger(crate::ForHead {
        head: e,
        data: gantz_egui::DropUnknownData { node },
    });
    app.update();
    assert_eq!(first_node(&app, &jam), known, "the discard commits");

    app.world_mut().trigger(crate::ForHead {
        head: e,
        data: gantz_egui::Undo,
    });
    app.update();
    assert_eq!(first_node(&app, &jam), stored, "undo restores the setting");
}
