//! Shared operations behind the GUI's graph-mutating response payloads.
//!
//! Each fn implements the state change for one payload (e.g.
//! [`CreateNode`]) over plain graph/view/VM/registry types
//! so that frontends (e.g. `bevy_gantz_egui` and the pure-egui demo) remain
//! thin adapters around identical behaviour. Frontend-specific effects
//! (clipboard access, file dialogs, head navigation) stay with the caller.

use crate::widget::gantz::OpenHeadState;
use crate::widget::graph_scene::{self, NodeIndex, ToGraphMut};
use crate::{CreateNode, GraphViews, InspectEdge, PastePos, export, node::NamedRef};
use gantz_ca::{CaHash, CommitAddr};
use gantz_core::node::{self, GetNode, graph::Graph};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::{HashMap, HashSet};
use steel::steel_vm::engine::Engine;

/// Branch a named node: create a new commit for the given content address
/// (original commit as parent), insert the new name pointing at it, and
/// replace the node at `path` with a [`NamedRef`] referencing the fresh
/// commit.
pub fn branch_node<N>(
    registry: &mut gantz_ca::Registry<Graph<N>>,
    timestamp: std::time::Duration,
    graph: &mut Graph<N>,
    new_name: String,
    ca: gantz_ca::ContentAddr,
    path: &[node::Id],
) where
    N: From<NamedRef> + ToGraphMut<Node = N>,
{
    let commit_ca = CommitAddr::from(ca);
    let Some(commit) = registry.commits().get(&commit_ca) else {
        log::error!("BranchNode: commit not found for {commit_ca:?}");
        return;
    };
    let graph_addr = commit.graph;
    let new_commit_ca = registry.commit_graph(timestamp, Some(commit_ca), graph_addr, || {
        unreachable!("graph already exists in registry")
    });
    registry.insert_name(new_name.clone(), new_commit_ca);

    // Replace the NamedRef node in the working graph.
    let Some((&node_ix, parent_path)) = path.split_last() else {
        log::error!("BranchNode: empty node path");
        return;
    };
    let Some(g) = graph_scene::index_path_graph_mut(graph, parent_path) else {
        log::error!("BranchNode: could not find graph at path {parent_path:?}");
        return;
    };
    let node_id = node::graph::NodeIx::new(node_ix);
    let new_ref = node::Ref::new(new_commit_ca.into());
    let named_ref = NamedRef::new(new_name, new_ref);
    if let Some(node) = g.node_weight_mut(node_id) {
        *node = N::from(named_ref);
    } else {
        log::error!("BranchNode: node not found at index {node_ix}");
    }
}

/// Serialize the selection at `path` to a `.gantz` clipboard payload.
///
/// Returns `None` when the selection is empty, the path is invalid, or
/// serialization fails (logging the cause). Writing the resulting string to
/// the clipboard is the caller's responsibility.
pub fn copy_nodes<N>(
    registry: &gantz_ca::Registry<Graph<N>>,
    all_views: &HashMap<CommitAddr, GraphViews>,
    graph: &mut Graph<N>,
    head_views: &GraphViews,
    path: &[node::Id],
    selection: &HashSet<NodeIndex>,
) -> Option<String>
where
    N: gantz_core::Node
        + Clone
        + Serialize
        + DeserializeOwned
        + CaHash
        + ToGraphMut<Node = N>
        + 'static,
{
    if selection.is_empty() {
        return None;
    }
    let Some(g) = graph_scene::index_path_graph_mut(graph, path) else {
        log::error!("CopyNodes: could not find graph at path {path:?}");
        return None;
    };
    let layout = head_views
        .get(path)
        .map(|v| &v.layout)
        .cloned()
        .unwrap_or_default();
    let copied = export::copy(registry, all_views, g, selection, &layout);
    match export::copied_to_string(&copied) {
        Ok(text) => Some(text),
        Err(e) => {
            log::error!("CopyNodes: failed to serialize: {e}");
            None
        }
    }
}

/// Create a node of the given type at `cmd.path`, register it with the VM,
/// and ensure it has a layout entry.
///
/// Returns the index of the new node within the graph at `cmd.path`.
pub fn create_node<N>(
    get_node: GetNode,
    new_node: impl FnOnce(&str) -> Option<N>,
    graph: &mut Graph<N>,
    views: &mut GraphViews,
    vm: &mut Engine,
    cmd: CreateNode,
) -> Option<NodeIndex>
where
    N: gantz_core::Node + ToGraphMut<Node = N>,
{
    let CreateNode { path, node_type } = cmd;
    let Some(nested) = graph_scene::index_path_graph_mut(graph, &path) else {
        log::error!("CreateNode: could not find graph at path {path:?}");
        return None;
    };
    let Some(node) = new_node(&node_type) else {
        log::error!("CreateNode: unknown node type: {node_type}");
        return None;
    };
    let node_ix = nested.add_node(node);

    // Register the new node with the VM.
    let node_path: Vec<_> = path.iter().copied().chain(Some(node_ix.index())).collect();
    let reg_ctx = node::RegCtx::new(get_node, &node_path, vm);
    nested[node_ix].register(reg_ctx);

    // Position the new node at the scene center (or use layout default).
    let egui_id = egui_graph::NodeId::from_u64(node_ix.index() as u64);
    let view = views.entry(path).or_default();
    view.layout.entry(egui_id).or_insert(egui::Pos2::ZERO);

    Some(node_ix)
}

/// Insert an Inspect node on the given edge, splicing it between the
/// endpoints and positioning it at `cmd.pos`.
pub fn inspect_edge<N>(
    get_node: GetNode,
    new_inspect: impl FnOnce() -> Option<N>,
    graph: &mut Graph<N>,
    views: &mut GraphViews,
    vm: &mut Engine,
    cmd: InspectEdge,
) where
    N: gantz_core::Node + ToGraphMut<Node = N>,
{
    let InspectEdge { path, edge, pos } = cmd;

    // Navigate to the nested graph at the path.
    let Some(nested) = graph_scene::index_path_graph_mut(graph, &path) else {
        log::error!("InspectEdge: could not find graph at path {path:?}");
        return;
    };

    // Get edge endpoints and weight.
    let Some((src_node, dst_node)) = nested.edge_endpoints(edge) else {
        log::error!("InspectEdge: edge not found");
        return;
    };
    let edge_weight = *nested.edge_weight(edge).unwrap();

    // Remove the edge.
    nested.remove_edge(edge);

    // Create a new Inspect node.
    let Some(inspect_node) = new_inspect() else {
        log::error!("InspectEdge: could not create inspect node");
        return;
    };
    let inspect_id = nested.add_node(inspect_node);

    // Determine the node path and register it with the VM.
    let node_path: Vec<_> = path
        .iter()
        .copied()
        .chain(Some(inspect_id.index()))
        .collect();
    let reg_ctx = node::RegCtx::new(get_node, &node_path, vm);
    nested[inspect_id].register(reg_ctx);

    // Add edge: src -> inspect (using original output, input 0).
    nested.add_edge(
        src_node,
        inspect_id,
        gantz_core::Edge::new(edge_weight.output, node::Input(0)),
    );

    // Add edge: inspect -> dst (using output 0, original input).
    nested.add_edge(
        inspect_id,
        dst_node,
        gantz_core::Edge::new(node::Output(0), edge_weight.input),
    );

    // Position the new node at the click position.
    let node_id = egui_graph::NodeId::from_u64(inspect_id.index() as u64);
    let view = views.entry(path).or_default();
    view.layout.insert(node_id, pos);
}

/// Paste a previously-copied clipboard payload into the graph at the head's
/// current path, and update the selection to the pasted nodes.
///
/// Returns `true` if a payload was pasted. The caller is responsible for
/// re-registering the root graph with the VM afterwards so pasted nodes get
/// their state initialized.
pub fn paste<N>(
    registry: &mut gantz_ca::Registry<Graph<N>>,
    all_views: &mut HashMap<CommitAddr, GraphViews>,
    all_demos: &mut HashMap<CommitAddr, String>,
    graph: &mut Graph<N>,
    head_views: &mut GraphViews,
    head_state: &mut OpenHeadState,
    text: &str,
    pos: &PastePos,
) -> bool
where
    N: Clone + Serialize + DeserializeOwned + CaHash + ToGraphMut<Node = N> + 'static,
{
    let copied: export::Copied<N> = match export::copied_from_str(text) {
        Ok(c) => c,
        Err(e) => {
            log::debug!("Clipboard does not contain a valid gantz payload: {e}");
            return false;
        }
    };
    let offset = crate::resolve_paste_offset(pos, &copied.positions);

    let path = head_state.path.clone();
    let Some(g) = graph_scene::index_path_graph_mut(graph, &path) else {
        log::error!("Paste: could not find graph at path {path:?}");
        return false;
    };
    let view = head_views.entry(path).or_default();
    let new_indices = export::paste(
        registry,
        all_views,
        all_demos,
        g,
        &mut view.layout,
        &copied,
        offset,
    );

    // Update selection to the pasted nodes.
    head_state.scene.interaction.selection.nodes = new_indices.into_iter().collect();
    head_state.scene.interaction.selection.edges.clear();
    true
}

/// Undo: push the head's current commit onto its redo stack and return the
/// parent commit to navigate to.
///
/// Returns `None` when the head has no parent commit to return to.
/// Navigation itself is frontend-specific and stays with the caller.
pub fn undo<G>(
    registry: &gantz_ca::Registry<G>,
    redo_stacks: &mut HashMap<gantz_ca::Head, Vec<CommitAddr>>,
    head: &gantz_ca::Head,
) -> Option<CommitAddr> {
    let commit_ca = registry.head_commit_ca(head).copied()?;
    let parent = registry.commits().get(&commit_ca)?.parent?;
    redo_stacks.entry(head.clone()).or_default().push(commit_ca);
    Some(parent)
}

/// Redo: pop the most recently undone commit from the head's redo stack.
///
/// Navigation itself is frontend-specific and stays with the caller.
pub fn redo(
    redo_stacks: &mut HashMap<gantz_ca::Head, Vec<CommitAddr>>,
    head: &gantz_ca::Head,
) -> Option<CommitAddr> {
    redo_stacks.get_mut(head)?.pop()
}
