//! Graph-level actions from the Graphs and Graph Config panes.

use crate::ForApp;
use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head;
use gantz_ca as ca;
use gantz_core::node::state;

/// Create an empty graph and open it in a new tab.
pub(crate) fn on_new_graph(
    _trigger: On<ForApp<gantz_egui::NewGraph>>,
    mut registry: ResMut<Registry>,
    mut cmds: Commands,
) {
    let new_head = registry.init_head(bevy_gantz::timestamp());
    cmds.trigger(head::OpenEvent(new_head));
}

/// Remove a graph name. See [`remove_name`].
pub(crate) fn on_remove_name(
    trigger: On<ForApp<gantz_egui::RemoveName>>,
    mut registry: ResMut<Registry>,
    mut heads: Query<&mut head::HeadRef, With<head::OpenHead>>,
) {
    let gantz_egui::RemoveName(name) = &trigger.event().0;
    remove_name(&mut registry, heads.iter_mut(), name);
}

/// Set or clear the demo graph of a named graph. The association is keyed by
/// the graph's name, so it survives later edits, which mint new commits.
pub(crate) fn on_set_demo(
    trigger: On<ForApp<gantz_egui::SetDemo>>,
    mut registry: ResMut<Registry>,
) {
    let gantz_egui::SetDemo { name, demo } = &trigger.event().0;
    match demo {
        Some(demo) => gantz_edit::section::set_demo(&mut registry.0, name.clone(), demo.clone()),
        None => {
            gantz_edit::section::remove_demo(&mut registry.0, name);
        }
    }
}

/// Set the description of a named graph.
pub(crate) fn on_set_description(
    trigger: On<ForApp<gantz_egui::SetDescription>>,
    mut registry: ResMut<Registry>,
) {
    let gantz_egui::SetDescription { name, description } = &trigger.event().0;
    gantz_edit::section::set_description(&mut registry.0, name.clone(), description.clone());
}

/// Set whether a named graph keeps its node state. A graph that stops keeping
/// its state also loses the state it stored.
pub(crate) fn on_set_keep_state(
    trigger: On<ForApp<gantz_egui::SetKeepState>>,
    mut registry: ResMut<Registry>,
) {
    let gantz_egui::SetKeepState { name, keep } = &trigger.event().0;
    if *keep {
        ca::section_remove::<state::Volatile>(&mut registry.0, name);
    } else {
        ca::section_insert::<state::Volatile>(&mut registry.0, name.clone(), &true)
            .expect("a `bool` always encodes as a datum");
        head::forget_state(&mut registry.0, &ca::Head::Branch(name.clone()));
    }
}

/// Remove `name`, first detaching the open heads on it onto its current
/// commit, so they stay open as unnamed graphs.
pub(crate) fn remove_name<'a>(
    registry: &mut ca::Registry,
    open: impl Iterator<Item = Mut<'a, head::HeadRef>>,
    name: &ca::Name,
) {
    for mut head_ref in open {
        if !matches!(&**head_ref, ca::Head::Branch(n) if n == name) {
            continue;
        }
        if let Some(commit_ca) = registry.head_commit_ca(&head_ref) {
            **head_ref = ca::Head::Commit(commit_ca);
        }
    }
    registry.remove_head(name);
}
