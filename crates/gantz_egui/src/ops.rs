//! Shared operations behind the GUI's graph-mutating response payloads.
//!
//! Each fn implements the state change for one payload (e.g.
//! [`CreateNode`]) over plain graph/view/VM/registry types
//! so that frontends (e.g. `bevy_gantz_egui` and the pure-egui demo) remain
//! thin adapters around identical behaviour. Frontend-specific effects
//! (clipboard access, file dialogs, head navigation) stay with the caller.

use crate::widget::gantz::OpenHeadState;
use crate::widget::graph_scene::NodeIndex;
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
/// commit. `path`'s last element is the node's index within the graph.
pub fn branch_node<N>(
    registry: &mut gantz_ca::Registry<Graph<N>>,
    timestamp: std::time::Duration,
    graph: &mut Graph<N>,
    new_name: String,
    ca: gantz_ca::ContentAddr,
    path: &[node::Id],
) where
    N: From<NamedRef>,
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
    let Some(&node_ix) = path.last() else {
        log::error!("BranchNode: empty node path");
        return;
    };
    let node_id = node::graph::NodeIx::new(node_ix);
    let new_ref = node::Ref::new(new_commit_ca.into());
    let named_ref = NamedRef::new(new_name, new_ref);
    if let Some(node) = graph.node_weight_mut(node_id) {
        *node = N::from(named_ref);
    } else {
        log::error!("BranchNode: node not found at index {node_ix}");
    }
}

/// Serialize the current selection to a `.gantz` clipboard payload.
///
/// Returns `None` when the selection is empty or serialization fails (logging
/// the cause). Writing the resulting string to the clipboard is the caller's
/// responsibility.
pub fn copy_nodes<N>(
    registry: &gantz_ca::Registry<Graph<N>>,
    all_views: &HashMap<CommitAddr, GraphViews>,
    graph: &Graph<N>,
    head_views: &GraphViews,
    selection: &HashSet<NodeIndex>,
) -> Option<String>
where
    N: gantz_core::Node + Clone + Serialize + DeserializeOwned + CaHash + 'static,
{
    if selection.is_empty() {
        return None;
    }
    let layout = head_views
        .get(&Vec::new())
        .map(|v| &v.layout)
        .cloned()
        .unwrap_or_default();
    let copied = export::copy(registry, all_views, graph, selection, &layout);
    match export::copied_to_string(&copied) {
        Ok(text) => Some(text),
        Err(e) => {
            log::error!("CopyNodes: failed to serialize: {e}");
            None
        }
    }
}

/// Create a node of the given type in `graph`, register it with the VM, and
/// ensure it has a layout entry.
///
/// Returns the index of the new node.
pub fn create_node<N>(
    get_node: GetNode,
    new_node: impl FnOnce(&str) -> Option<N>,
    graph: &mut Graph<N>,
    views: &mut GraphViews,
    vm: &mut Engine,
    cmd: CreateNode,
) -> Option<NodeIndex>
where
    N: gantz_core::Node,
{
    let CreateNode { node_type } = cmd;
    let Some(node) = new_node(&node_type) else {
        log::error!("CreateNode: unknown node type: {node_type}");
        return None;
    };
    let node_ix = graph.add_node(node);

    // Register the new node with the VM.
    let node_path = [node_ix.index()];
    let reg_ctx = node::RegCtx::new(get_node, &node_path, vm);
    graph[node_ix].register(reg_ctx);

    // Position the new node at the scene center (or use layout default).
    let egui_id = egui_graph::NodeId::from_u64(node_ix.index() as u64);
    let view = views.entry(Vec::new()).or_default();
    view.layout.entry(egui_id).or_insert(egui::Pos2::ZERO);

    Some(node_ix)
}

/// Create a nested graph: commit a fresh empty graph to the registry under the
/// name `<parent>:<n>` and insert a synced [`NamedRef`] to it in `graph`,
/// seeding its layout entry.
///
/// `parent` is the emitting head's name; the new graph is named with the first
/// free `<parent>:<n>` leaf. Returns the index of the new node.
pub fn create_nested_graph<N>(
    registry: &mut gantz_ca::Registry<Graph<N>>,
    timestamp: std::time::Duration,
    graph: &mut Graph<N>,
    views: &mut GraphViews,
    parent: &str,
) -> Option<NodeIndex>
where
    N: gantz_core::Node + From<NamedRef> + CaHash,
{
    // Pick the first free `<parent>:<n>` leaf name.
    let sep = crate::node::NESTED_SEP;
    let mut n = 1u32;
    let name = loop {
        let candidate = format!("{parent}{sep}{n}");
        if !registry.names().contains_key(&candidate) {
            break candidate;
        }
        n += 1;
    };

    // Commit a fresh empty graph under the chosen name.
    let nested_graph = Graph::<N>::default();
    let graph_ca = gantz_ca::graph_addr(&nested_graph);
    let commit_ca = registry.commit_graph_to_name(timestamp, graph_ca, || nested_graph, &name);

    // Insert a synced reference to the new nested graph. The referenced graph is
    // empty, so the node has no state to register here; the next `vm::sync`
    // recompile re-registers the whole working graph.
    let named_ref = NamedRef::with_sync(name, node::Ref::new(commit_ca.into()));
    let node_ix = graph.add_node(N::from(named_ref));

    // Position the new node at the scene center (or use layout default).
    let egui_id = egui_graph::NodeId::from_u64(node_ix.index() as u64);
    let view = views.entry(Vec::new()).or_default();
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
    N: gantz_core::Node,
{
    let InspectEdge { edge, pos } = cmd;

    // Get edge endpoints and weight.
    let Some((src_node, dst_node)) = graph.edge_endpoints(edge) else {
        log::error!("InspectEdge: edge not found");
        return;
    };
    let edge_weight = *graph.edge_weight(edge).unwrap();

    // Remove the edge.
    graph.remove_edge(edge);

    // Create a new Inspect node.
    let Some(inspect_node) = new_inspect() else {
        log::error!("InspectEdge: could not create inspect node");
        return;
    };
    let inspect_id = graph.add_node(inspect_node);

    // Register the new node with the VM.
    let node_path = [inspect_id.index()];
    let reg_ctx = node::RegCtx::new(get_node, &node_path, vm);
    graph[inspect_id].register(reg_ctx);

    // Add edge: src -> inspect (using original output, input 0).
    graph.add_edge(
        src_node,
        inspect_id,
        gantz_core::Edge::new(edge_weight.output, node::Input(0)),
    );

    // Add edge: inspect -> dst (using output 0, original input).
    graph.add_edge(
        inspect_id,
        dst_node,
        gantz_core::Edge::new(node::Output(0), edge_weight.input),
    );

    // Position the new node at the click position.
    let node_id = egui_graph::NodeId::from_u64(inspect_id.index() as u64);
    let view = views.entry(Vec::new()).or_default();
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
    N: Clone + Serialize + DeserializeOwned + CaHash + 'static,
{
    let copied: export::Copied<N> = match export::copied_from_str(text) {
        Ok(c) => c,
        Err(e) => {
            log::debug!("Clipboard does not contain a valid gantz payload: {e}");
            return false;
        }
    };
    let offset = crate::resolve_paste_offset(pos, &copied.positions);

    let view = head_views.entry(Vec::new()).or_default();
    let new_indices = export::paste(
        registry,
        all_views,
        all_demos,
        graph,
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
