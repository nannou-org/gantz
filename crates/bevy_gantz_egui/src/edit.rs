//! Observers for the graph edits that the GUI emits for a head.

use crate::{
    BuiltinNodes, ForHead, GraphCache, GraphView, GuiState, HeadNodeInstances, NodeCodecRes, env,
    refresh_cache,
};
use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head;
use bevy_log as log;
use gantz_ca as ca;

/// Handle create node payloads.
pub fn on_create_node(
    trigger: On<ForHead<gantz_egui::CreateNode>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
    mut views_query: Query<&mut GraphView, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!("CreateNode: head not found for entity {:?}", event.head);
        return;
    };
    let editing = match &**data.head_ref {
        ca::Head::Branch(name) => Some(name.to_string()),
        ca::Head::Commit(_) => None,
    };
    let Ok(mut views) = views_query.get_mut(event.head) else {
        log::error!("CreateNode: views not found for entity {:?}", event.head);
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("CreateNode: VM not found for entity {:?}", event.head);
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**data.head_ref) else {
        log::error!("CreateNode: GUI state not found for head");
        return;
    };

    let node_reg = env(&registry, &cache, &builtins, &codec);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    gantz_egui::ops::create_node(
        node_reg.registry,
        editing.as_deref(),
        &codec.0,
        &get_node,
        |node_type| node_reg.create_node(node_type),
        &mut data.working_graph,
        &mut views,
        head_state,
        vm,
        event.data.clone(),
    );
    // See `head::WorkingGraph`.
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle replace node payloads.
pub fn on_replace_node(
    trigger: On<ForHead<gantz_egui::ReplaceNode>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!("ReplaceNode: head not found for entity {:?}", event.head);
        return;
    };
    let editing = match &**data.head_ref {
        ca::Head::Branch(name) => Some(name.to_string()),
        ca::Head::Commit(_) => None,
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("ReplaceNode: VM not found for entity {:?}", event.head);
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**data.head_ref) else {
        log::error!("ReplaceNode: GUI state not found for head");
        return;
    };

    let node_reg = env(&registry, &cache, &builtins, &codec);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    let replaced = gantz_egui::ops::replace_node(
        node_reg.registry,
        editing.as_deref(),
        &codec.0,
        &get_node,
        |node_type| node_reg.create_node(node_type),
        &mut data.working_graph,
        head_state,
        vm,
        event.data.clone(),
    );
    if !replaced {
        return;
    }
    // See `head::WorkingGraph`.
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle drop unknown data payloads. The rewrite commits like any edit, so
/// undo restores the dropped data.
pub fn on_drop_unknown_data(
    trigger: On<ForHead<gantz_egui::DropUnknownData>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!(
            "DropUnknownData: head not found for entity {:?}",
            event.head
        );
        return;
    };
    if !gantz_egui::ops::drop_unknown_data(&codec.0, &mut data.working_graph, event.data) {
        return;
    }
    // See `head::WorkingGraph`.
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle branch node payloads.
///
/// Creates a new commit with the same graph content, a new timestamp and the
/// original as parent. Inserts the new name and replaces the NamedRef node in
/// the working graph.
pub fn on_branch_node(
    trigger: On<ForHead<gantz_egui::BranchNode>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!("BranchNode: head not found for entity {:?}", event.head);
        return;
    };
    gantz_egui::ops::branch_node(
        &mut registry,
        bevy_gantz::reg::timestamp(),
        &mut data.working_graph,
        event.data.new_name.clone(),
        event.data.ca,
        &event.data.path,
    );
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle create nested graph payloads.
///
/// Commits a fresh empty graph named `<parent>:<n>`, where `<parent>` is the
/// head's branch name. Inserts a synced `NamedRef` to it in the head's
/// working graph. Requires the head to be named.
pub fn on_create_nested_graph(
    trigger: On<ForHead<gantz_egui::CreateNestedGraph>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
    mut views_query: Query<&mut GraphView, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!(
            "CreateNestedGraph: head not found for entity {:?}",
            event.head
        );
        return;
    };
    let ca::Head::Branch(parent) = data.head_ref.0.clone() else {
        log::warn!("CreateNestedGraph: name the graph before adding a nested graph");
        return;
    };
    let Ok(mut views) = views_query.get_mut(event.head) else {
        log::error!(
            "CreateNestedGraph: views not found for entity {:?}",
            event.head
        );
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**data.head_ref) else {
        log::error!("CreateNestedGraph: GUI state not found for head");
        return;
    };
    gantz_egui::ops::create_nested_graph(
        &mut registry,
        bevy_gantz::reg::timestamp(),
        &mut data.working_graph,
        &mut views,
        head_state,
        event.data.pos,
        &parent,
    );
    // The fresh nested graph must be reified before its NamedRef resolves.
    refresh_cache(&registry, &mut cache, &codec.0);
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle inspect edge payloads.
pub fn on_inspect_edge(
    trigger: On<ForHead<gantz_egui::InspectEdge>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
    mut views_query: Query<&mut GraphView, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!("InspectEdge: head not found for entity {:?}", event.head);
        return;
    };
    let Ok(mut views) = views_query.get_mut(event.head) else {
        log::error!("InspectEdge: views not found for entity {:?}", event.head);
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("InspectEdge: VM not found for entity {:?}", event.head);
        return;
    };

    let node_reg = env(&registry, &cache, &builtins, &codec);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    gantz_egui::ops::inspect_edge(
        &codec.0,
        &get_node,
        || node_reg.create_node("inspect"),
        &mut data.working_graph,
        &mut views,
        vm,
        event.data.clone(),
    );
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle copy selection payloads.
///
/// Serializes the selected nodes and their registry dependencies to a
/// `.gantz` document and writes the result to the system clipboard via
/// [`bevy_egui::EguiClipboard`].
pub fn on_copy_nodes(
    trigger: On<ForHead<gantz_egui::CopyNodes>>,
    registry: Res<Registry>,
    codec: Res<NodeCodecRes>,
    mut clipboard: ResMut<bevy_egui::EguiClipboard>,
    mut heads: Query<(&mut head::WorkingGraph, &GraphView), With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok((wg, gv)) = heads.get_mut(event.head) else {
        log::error!("CopySelection: head not found for entity {:?}", event.head);
        return;
    };

    let text = gantz_egui::ops::copy_nodes(&registry, &wg, gv, &event.data.0, &codec.0);
    if let Some(text) = text {
        clipboard.set_text(&text);
    }
}

/// Handle paste selection payloads.
///
/// Resolves the clipboard text via [`bevy_egui::EguiClipboard`] when the
/// payload does not carry it. Parses it into a [`gantz_egui::export::Copied`],
/// merges registry dependencies, adds the subgraph, maps positions, and
/// selects the pasted nodes.
pub fn on_paste(
    trigger: On<ForHead<gantz_egui::Paste>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut clipboard: ResMut<bevy_egui::EguiClipboard>,
    mut cmds: Commands,
    mut heads: Query<
        (&mut head::HeadRef, &mut head::WorkingGraph, &mut GraphView),
        With<head::OpenHead>,
    >,
) {
    let event = trigger.event();
    let Some(text) = event.data.text.clone().or_else(|| clipboard.get_text()) else {
        return;
    };
    let Ok((mut head_ref, mut wg, mut gv)) = heads.get_mut(event.head) else {
        log::error!("PasteSelection: head not found for entity {:?}", event.head);
        return;
    };
    let editing = match &**head_ref {
        ca::Head::Branch(name) => Some(name.to_string()),
        ca::Head::Commit(_) => None,
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("PasteSelection: GUI state not found for head");
        return;
    };

    let pasted = gantz_egui::ops::paste(
        &mut registry,
        editing.as_deref(),
        &mut wg,
        &mut gv,
        head_state,
        &text,
        &event.data.pos,
        &codec.0,
    );

    // Re-register the full root graph so pasted nodes get their state
    // initialized with the correct nested hashmap structure. Idempotent for
    // existing nodes. Registration reifies the graph transiently. The paste
    // may have merged new dependency graphs into the registry, so refresh
    // the cache first.
    if pasted {
        refresh_cache(&registry, &mut cache, &codec.0);
        if let Some(vm) = vms.get_mut(&event.head) {
            let node_reg = env(&registry, &cache, &builtins, &codec);
            let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
            match codec.0.reify_graph(&wg.0) {
                Ok(g) => gantz_core::graph::register(&get_node, &g, &[], vm),
                Err(e) => log::error!("Paste: cannot re-register the pasted graph: {e}"),
            }
        }
    }

    bevy_gantz::commit_working_graph(&mut registry, &mut cmds, event.head, &mut head_ref.0, &wg.0);
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle cut payloads. Copy the selection to the clipboard, then remove it.
pub fn on_cut_nodes(
    trigger: On<ForHead<gantz_egui::CutNodes>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut clipboard: ResMut<bevy_egui::EguiClipboard>,
    mut cmds: Commands,
    mut heads: Query<
        (
            &mut head::HeadRef,
            &mut head::WorkingGraph,
            &mut GraphView,
            &mut HeadNodeInstances,
        ),
        With<head::OpenHead>,
    >,
) {
    let event = trigger.event();
    let Ok((mut head_ref, mut wg, mut gv, mut instances)) = heads.get_mut(event.head) else {
        log::error!("CutNodes: head not found for entity {:?}", event.head);
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("CutNodes: GUI state not found for head");
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("CutNodes: VM not found for head");
        return;
    };

    let text = gantz_egui::ops::cut_nodes(
        &registry,
        &mut wg,
        vm,
        &mut gv,
        &mut head_state.scene.interaction.selection,
        &mut instances.0,
        &event.data.0,
        &codec.0,
    );
    if let Some(text) = text {
        clipboard.set_text(&text);
    }

    bevy_gantz::commit_working_graph(&mut registry, &mut cmds, event.head, &mut head_ref.0, &wg.0);
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle nest payloads. Cut the selected nodes into a new nested graph node.
pub fn on_nest_nodes(
    trigger: On<ForHead<gantz_egui::NestNodes>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<
        (
            &mut head::HeadRef,
            &mut head::WorkingGraph,
            &mut GraphView,
            &mut HeadNodeInstances,
        ),
        With<head::OpenHead>,
    >,
) {
    let event = trigger.event();
    let Ok((mut head_ref, mut wg, mut gv, mut instances)) = heads.get_mut(event.head) else {
        log::error!("NestNodes: head not found for entity {:?}", event.head);
        return;
    };
    let ca::Head::Branch(parent) = head_ref.0.clone() else {
        log::warn!("NestNodes: name the graph before nesting nodes");
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("NestNodes: GUI state not found for head");
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("NestNodes: VM not found for head");
        return;
    };

    gantz_egui::ops::nest_nodes(
        &mut registry,
        bevy_gantz::reg::timestamp(),
        &mut wg,
        vm,
        &mut gv,
        head_state,
        &mut instances.0,
        &event.data.0,
        &parent,
    );

    // The fresh nested graph must be reified before its NamedRef resolves.
    refresh_cache(&registry, &mut cache, &codec.0);
    bevy_gantz::commit_working_graph(&mut registry, &mut cmds, event.head, &mut head_ref.0, &wg.0);
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle duplicate payloads. Copy the selection, then paste it at an offset.
pub fn on_duplicate_nodes(
    trigger: On<ForHead<gantz_egui::DuplicateNodes>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<
        (&mut head::HeadRef, &mut head::WorkingGraph, &mut GraphView),
        With<head::OpenHead>,
    >,
) {
    let event = trigger.event();
    let Ok((mut head_ref, mut wg, mut gv)) = heads.get_mut(event.head) else {
        log::error!("DuplicateNodes: head not found for entity {:?}", event.head);
        return;
    };
    let editing = match &**head_ref {
        ca::Head::Branch(name) => Some(name.to_string()),
        ca::Head::Commit(_) => None,
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("DuplicateNodes: GUI state not found for head");
        return;
    };

    let duplicated = gantz_egui::ops::duplicate_nodes(
        &mut registry,
        editing.as_deref(),
        &mut wg,
        &mut gv,
        head_state,
        &event.data.0,
        &codec.0,
    );

    // Re-register the full root graph so the new nodes get their state
    // initialized. Idempotent for existing nodes. Registration reifies the
    // graph transiently.
    if duplicated {
        refresh_cache(&registry, &mut cache, &codec.0);
        if let Some(vm) = vms.get_mut(&event.head) {
            let node_reg = env(&registry, &cache, &builtins, &codec);
            let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
            match codec.0.reify_graph(&wg.0) {
                Ok(g) => gantz_core::graph::register(&get_node, &g, &[], vm),
                Err(e) => log::error!("DuplicateNodes: cannot re-register the graph: {e}"),
            }
        }
    }

    bevy_gantz::commit_working_graph(&mut registry, &mut cmds, event.head, &mut head_ref.0, &wg.0);
    refresh_cache(&registry, &mut cache, &codec.0);
}
