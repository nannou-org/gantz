//! End-to-end test for the `load!` node through its bevy driver in a
//! headless app.

use crate::node::{LoadBang, Sleep, load_bang};
use bevy_ecs::prelude::*;
use bevy_gantz::{Registry, head, timestamp};
use gantz_core::node;
use steel::SteelVal;

/// The state of the root node at `ix` in the head's VM.
fn state(app: &bevy_app::App, head: Entity, ix: usize) -> SteelVal {
    let vms = app.world().non_send::<head::HeadVms>();
    let vm = vms.0.get(&head).expect("head VM");
    node::state::extract_value(vm, &[ix])
        .expect("state read")
        .expect("state present")
}

/// Clear the state of the root node at `ix`, so a later firing shows.
fn clear(app: &mut bevy_app::App, head: Entity, ix: usize) {
    let mut vms = app.world_mut().non_send_mut::<head::HeadVms>();
    let vm = vms.0.get_mut(&head).expect("head VM");
    node::state::update_value(vm, &[ix], SteelVal::Void).expect("state write");
}

/// `load!` fires once when a head opens and again when a replace loads a
/// head into the tab. It does not fire on later updates or when a move
/// navigates the head's history.
#[test]
fn load_fires_on_open_and_replace_but_not_on_move() {
    let mut app = super::test_app(load_bang::drive_load_bangs);
    let bang = SteelVal::ListV(Default::default());

    // The base graph is `load! -> inspect`. The child graph adds a filler
    // node, so the move to it recompiles.
    let mut base_dg = gantz_ca::DataGraph::default();
    let load = base_dg.add_node(gantz_core::data::erase_node_typed(&LoadBang).unwrap());
    let inspect =
        base_dg.add_node(gantz_core::data::erase_node_typed(&gantz_egui::node::Inspect).unwrap());
    base_dg.add_edge(load, inspect, gantz_ca::Edge::from((0, 0)));
    let mut child_dg = base_dg.clone();
    child_dg.add_node(gantz_core::data::erase_node_typed(&Sleep::default()).unwrap());
    let (base, child) = {
        let mut registry = app.world_mut().resource_mut::<Registry>();
        let base_ca = gantz_ca::graph_addr(&base_dg);
        let base = registry.commit_graph(timestamp(), None, base_ca, move || base_dg);
        let child_ca = gantz_ca::graph_addr(&child_dg);
        let child = registry.commit_graph(timestamp(), Some(base), child_ca, move || child_dg);
        (base, child)
    };
    super::refresh_app_cache(&mut app);

    // Open fires.
    app.world_mut()
        .trigger(head::OpenEvent(gantz_ca::Head::Commit(base)));
    app.update();
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<head::OpenHead>>();
    let e = q.single(app.world()).expect("one open head");
    assert_eq!(state(&app, e, inspect.index()), bang, "open fires");

    // Later updates do not fire again.
    clear(&mut app, e, inspect.index());
    app.update();
    app.update();
    assert_eq!(
        state(&app, e, inspect.index()),
        SteelVal::Void,
        "fires once"
    );

    // A move navigates history and does not fire.
    app.world_mut().trigger(head::MoveHeadEvent {
        entity: e,
        target: child,
    });
    app.update();
    assert_eq!(state(&app, e, inspect.index()), SteelVal::Void, "move");

    // A replace loads the head into the tab and fires.
    app.world_mut()
        .trigger(head::ReplaceEvent(gantz_ca::Head::Commit(base)));
    app.update();
    assert_eq!(state(&app, e, inspect.index()), bang, "replace fires");
}
