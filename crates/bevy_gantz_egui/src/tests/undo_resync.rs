//! Undo and redo inside a nested graph reach its parent, as an edit does.

use bevy_ecs::prelude::*;
use bevy_gantz::{Registry, head, timestamp};
use gantz_ca::{DataGraph, GraphAddr, Head, Name};
use gantz_core::data::erase_node_typed;
use gantz_egui::node::{Inspect, NamedRef};

/// The graph address that the sole `NamedRef` in `name`'s head graph points
/// at.
fn ref_target(app: &bevy_app::App, name: &Name) -> GraphAddr {
    let registry = app.world().resource::<Registry>();
    let commit = registry
        .head_commit(&Head::Branch(name.clone()))
        .expect("head commit");
    let graph = registry.graph(&commit.graph).expect("head graph");
    let (_, target, _) = gantz_egui::sync::named_refs(graph)
        .next()
        .expect("a named ref");
    target
}

/// A graph of `n` `inspect` nodes.
fn inspects(n: usize) -> DataGraph {
    let mut g = DataGraph::default();
    for _ in 0..n {
        g.add_node(erase_node_typed(&Inspect).unwrap());
    }
    g
}

#[test]
fn undo_and_redo_in_a_nested_graph_resync_its_parent() {
    let mut app = super::test_app(|| {});
    app.init_resource::<crate::GuiState>()
        .add_observer(crate::on_undo)
        .add_observer(crate::on_redo)
        .add_observer(crate::on_resync_refs);

    // The child `p:1` has an edit on top of its first graph. Its parent `p`
    // references the edited graph, as the resync after that edit leaves it.
    let parent: Name = "p".parse().unwrap();
    let child: Name = "p:1".parse().unwrap();
    let (v1, v2) = (inspects(1), inspects(2));
    let (v1_ca, v2_ca) = (gantz_ca::graph_addr(&v1), gantz_ca::graph_addr(&v2));
    let (c1, c2) = {
        let mut registry = app.world_mut().resource_mut::<Registry>();
        let c1 = registry.commit_graph(timestamp(), None, v1_ca, move || v1);
        let c2 = registry.commit_graph(timestamp(), Some(c1), v2_ca, move || v2);
        registry.set_head(child.clone(), c2);
        let mut p = DataGraph::default();
        let named_ref =
            NamedRef::with_sync(child.clone(), gantz_core::node::Ref::new(v2_ca.into()));
        p.add_node(erase_node_typed(&named_ref).unwrap());
        let p_ca = gantz_ca::graph_addr(&p);
        let pc = registry.commit_graph(timestamp(), None, p_ca, move || p);
        registry.set_head(parent.clone(), pc);
        (c1, c2)
    };
    super::refresh_app_cache(&mut app);

    // The child is open in a tab, as after entering it from its parent.
    app.world_mut()
        .trigger(head::OpenEvent(Head::Branch(child.clone())));
    app.update();
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<head::OpenHead>>();
    let e = q.single(app.world()).expect("one open head");
    assert_eq!(ref_target(&app, &parent), v2_ca, "setup");

    app.world_mut().trigger(crate::ForHead {
        head: e,
        data: gantz_egui::Undo,
    });
    app.update();
    let registry = app.world().resource::<Registry>();
    assert_eq!(registry.head(&child), Some(c1), "undo moves the child");
    assert_eq!(ref_target(&app, &parent), v1_ca, "undo reaches the parent");

    app.world_mut().trigger(crate::ForHead {
        head: e,
        data: gantz_egui::Redo,
    });
    app.update();
    let registry = app.world().resource::<Registry>();
    assert_eq!(registry.head(&child), Some(c2), "redo moves the child");
    assert_eq!(ref_target(&app, &parent), v2_ca, "redo reaches the parent");
}
