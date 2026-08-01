//! Shared operations behind the GUI's graph-mutating response payloads.
//!
//! Each fn implements the state change for one payload (e.g.
//! [`CreateNode`]) over plain graph/view/VM/registry types
//! so that frontends (e.g. `bevy_gantz_egui` and the pure-egui demo) remain
//! thin adapters around identical behaviour. Frontend-specific effects
//! (clipboard access, file dialogs, head navigation) stay with the caller.

use crate::cycle::named_ref_of;
use crate::node::NodeCodec;
use crate::widget::gantz::OpenHeadState;
use crate::widget::graph_scene::NodeIndex;
use crate::{CreateNode, InspectEdge, PastePos, export, node::NamedRef};
use gantz_ca::{CommitAddr, DataGraph, GraphAddr, Name, NodeData};
use gantz_core::node::{self, GetNode};
use petgraph::visit::EdgeRef;
pub use session::{RevertCursor, redo, session_redo, session_undo, undo};
use std::collections::{BTreeSet, HashMap, HashSet};
use steel::steel_vm::engine::Engine;
pub use sync::{MergeHeadOutcome, SyncTipOutcome, merge_head, sync_remote_tip};

mod session;
mod sync;

/// The egui-graph layout id for the node at graph index `ix`.
fn node_id(ix: usize) -> egui_graph::NodeId {
    egui_graph::NodeId::from_u64(ix as u64)
}

/// The fallback position for the `i`th node a view migration could not map:
/// cascade down-right from the camera `center` so unplaced nodes stay
/// visible without overlapping.
fn cascade_pos(center: egui::Pos2, i: usize) -> egui::Pos2 {
    center + egui::vec2(20.0, 20.0) * i as f32
}

/// The centroid of the given nodes' positions in `view`, or `None` when none
/// of them has a layout entry.
fn selection_centroid(view: &crate::SceneView, nodes: &HashSet<NodeIndex>) -> Option<egui::Pos2> {
    let mut sum = egui::Vec2::ZERO;
    let mut count = 0usize;
    for &n in nodes {
        if let Some(&pos) = view.layout.get(&node_id(n.index())) {
            sum += pos.to_vec2();
            count += 1;
        }
    }
    (count > 0).then(|| egui::Pos2::new(sum.x / count as f32, sum.y / count as f32))
}

/// The first free `<parent>:<n>` leaf name in the registry.
fn next_child_name(registry: &gantz_ca::Registry, parent: &Name) -> Name {
    let mut n = 1u32;
    loop {
        let candidate = parent.child(n.to_string());
        if registry.head(&candidate).is_none() {
            return candidate;
        }
        n += 1;
    }
}

/// Branch a named node: commit the graph at the given (graph) content address
/// under a new name, and replace the node at `path` with a [`NamedRef`]
/// referencing it. `path`'s last element is the node's index within the
/// graph.
///
/// The newest existing commit pointing at the graph (if any) becomes the new
/// commit's parent, preserving the fork point's history.
pub fn branch_node(
    registry: &mut gantz_ca::Registry,
    timestamp: std::time::Duration,
    graph: &mut DataGraph,
    new_name: String,
    ca: gantz_ca::ContentAddr,
    path: &[node::Id],
) {
    let graph_addr = GraphAddr::from(ca);
    if registry.graph(&graph_addr).is_none() {
        log::error!("BranchNode: graph not found for {graph_addr:?}");
        return;
    }
    let parent = newest_commit_for_graph(registry, graph_addr);
    let new_commit_ca = registry.commit_graph(timestamp, parent, graph_addr, || {
        unreachable!("graph already exists in registry")
    });
    let name: Name = new_name.parse().expect("infallible");
    registry.set_head(name.clone(), new_commit_ca);

    // Replace the NamedRef node in the working graph.
    let Some(&node_ix) = path.last() else {
        log::error!("BranchNode: empty node path");
        return;
    };
    let node_id = node::graph::NodeIx::new(node_ix);
    // Carry the old reference's ext data over: the forked content is
    // identical, so domain flags still apply. `sync` deliberately resets - a
    // fork pins.
    let new_ref = match graph.node_weight(node_id).and_then(named_ref_of) {
        Some(old) => old.ref_().retarget(graph_addr.into()),
        None => node::Ref::new(graph_addr.into()),
    };
    let named_ref = NamedRef::new(name, new_ref);
    let node_data = match gantz_core::data::erase_node_typed(&named_ref) {
        Ok(node_data) => node_data,
        Err(e) => {
            log::error!("BranchNode: failed to erase the new `NamedRef`: {e}");
            return;
        }
    };
    if let Some(node) = graph.node_weight_mut(node_id) {
        *node = node_data;
    } else {
        log::error!("BranchNode: node not found at index {node_ix}");
    }
}

/// The newest commit pointing at the given graph, if any (ties broken by
/// address for determinism).
fn newest_commit_for_graph(
    registry: &gantz_ca::Registry,
    graph_addr: GraphAddr,
) -> Option<CommitAddr> {
    registry
        .commits()
        .iter()
        .filter(|(_, commit)| commit.graph == graph_addr)
        .max_by_key(|(ca, commit)| (commit.timestamp, **ca))
        .map(|(ca, _)| *ca)
}

/// Serialize the current selection to a `.gantz` clipboard payload.
///
/// Returns `None` when the selection is empty or serialization fails (logging
/// the cause). Writing the resulting string to the clipboard is the caller's
/// responsibility.
pub fn copy_nodes(
    registry: &gantz_ca::Registry,
    graph: &DataGraph,
    head_view: &crate::SceneView,
    selection: &HashSet<NodeIndex>,
    codec: &NodeCodec,
) -> Option<String> {
    if selection.is_empty() {
        return None;
    }
    let copied = export::copy(registry, graph, selection, &head_view.layout);
    match export::copied_to_string(&copied, codec) {
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
/// `new_node` produces the node's stored data form (see e.g.
/// [`crate::Env::create_node`]); the fresh node reifies once through
/// the `codec` for its VM registration step.
///
/// Returns the index of the new node.
#[allow(clippy::too_many_arguments)]
pub fn create_node(
    registry: &gantz_ca::Registry,
    editing: Option<&str>,
    codec: &NodeCodec,
    get_node: GetNode,
    new_node: impl FnOnce(&str) -> Option<NodeData>,
    graph: &mut DataGraph,
    view: &mut crate::SceneView,
    head_state: &mut OpenHeadState,
    vm: &mut Engine,
    cmd: CreateNode,
) -> Option<NodeIndex> {
    let CreateNode { node_type, pos } = cmd;
    // Refuse references that would form a cycle back to the editing graph; with
    // sync on such a cycle recommits endlessly (see `crate::cycle`). A nameless
    // (detached commit) head can't be the target of a name-based cycle.
    if editing.is_some_and(|editing| {
        let target: Name = node_type.parse().expect("infallible");
        let editing: Name = editing.parse().expect("infallible");
        crate::cycle::would_cycle(registry, &target, &editing)
    }) {
        log::warn!("CreateNode: '{node_type}' would create a reference cycle; skipping");
        return None;
    }
    let Some(node) = new_node(&node_type) else {
        log::error!("CreateNode: unknown node type: {node_type}");
        return None;
    };
    let node_ix = graph.add_node(node);

    // Register the new node with the VM (its one transient typed appearance).
    match codec.reify_ui(&graph[node_ix]) {
        Ok(inst) => {
            let node_path = [node_ix.index()];
            let reg_ctx = node::RegCtx::new(get_node, &node_path, vm);
            inst.node.register(reg_ctx);
        }
        Err(e) => log::error!("CreateNode: cannot register '{node_type}' with the VM: {e}"),
    }

    // Position the new node under the pointer, falling back to the center of the
    // current view.
    let pos = pos.unwrap_or_else(|| view.camera.center);
    let egui_id = node_id(node_ix.index());
    view.layout.insert(egui_id, pos);

    // Make the new node the sole selection (clearing the previous one).
    let sel = &mut head_state.scene.interaction.selection;
    sel.nodes.clear();
    sel.edges.clear();
    sel.nodes.insert(node_ix);

    Some(node_ix)
}

/// Create a nested graph: commit a fresh empty graph to the registry under the
/// name `<parent>:<n>` and insert a synced [`NamedRef`] to it in `graph`,
/// seeding its layout entry.
///
/// `parent` is the emitting head's name; the new graph is named with the first
/// free `<parent>:<n>` leaf. Returns the index of the new node.
pub fn create_nested_graph(
    registry: &mut gantz_ca::Registry,
    timestamp: std::time::Duration,
    graph: &mut DataGraph,
    view: &mut crate::SceneView,
    head_state: &mut OpenHeadState,
    pos: Option<egui::Pos2>,
    parent: &Name,
) -> Option<NodeIndex> {
    // Pick the first free `<parent>:<n>` leaf name.
    let name = next_child_name(registry, parent);

    // Commit a fresh empty graph under the chosen name.
    let nested_graph = DataGraph::default();
    let graph_ca = gantz_ca::graph_addr(&nested_graph);
    registry.commit_graph_to_name(timestamp, graph_ca, || nested_graph, &name);

    // Insert a synced reference to the new nested graph. The referenced graph is
    // empty, so the node has no state to register here; the next `vm::sync`
    // recompile re-registers the whole working graph.
    let named_ref = NamedRef::with_sync(name, node::Ref::new(graph_ca.into()));
    let node_data = match gantz_core::data::erase_node_typed(&named_ref) {
        Ok(node_data) => node_data,
        Err(e) => {
            log::error!("CreateNestedGraph: failed to erase the new `NamedRef`: {e}");
            return None;
        }
    };
    let node_ix = graph.add_node(node_data);

    // Position the new node under the pointer, falling back to the center of the
    // current view.
    let pos = pos.unwrap_or_else(|| view.camera.center);
    let egui_id = node_id(node_ix.index());
    view.layout.insert(egui_id, pos);

    // Make the new node the sole selection (clearing the previous one).
    let sel = &mut head_state.scene.interaction.selection;
    sel.nodes.clear();
    sel.edges.clear();
    sel.nodes.insert(node_ix);

    Some(node_ix)
}

/// Nest the selected nodes into a new nested graph node.
///
/// The selected nodes (and edges between them) are cut from the parent graph and
/// become the contents of a fresh nested graph, committed under the first free
/// `<parent>:<n>` name (see [`create_nested_graph`]). Edges crossing the
/// selection boundary become the nested graph's inlets/outlets - one per cut
/// point - and the parent graph is re-wired to the new synced [`NamedRef`]
/// node's sockets.
///
/// Returns the index of the new node, or `None` when the selection is empty or
/// a node cannot be erased.
#[allow(clippy::too_many_arguments)]
pub fn nest_nodes(
    registry: &mut gantz_ca::Registry,
    timestamp: std::time::Duration,
    graph: &mut DataGraph,
    vm: &mut Engine,
    view: &mut crate::SceneView,
    head_state: &mut OpenHeadState,
    instances: &mut crate::node::NodeInstances,
    nodes: &HashSet<NodeIndex>,
    parent: &Name,
) -> Option<NodeIndex> {
    if nodes.is_empty() {
        return None;
    }

    // Collect the edges crossing the selection boundary, split by direction:
    // incoming (external source -> selected target) and outgoing (selected source
    // -> external target). Each becomes one inlet/outlet on the nested node; the
    // sort below matches how the graph node numbers its sockets (by inlet/outlet
    // node index).
    let mut incoming: Vec<(NodeIndex, gantz_core::Edge, NodeIndex)> = Vec::new();
    let mut outgoing: Vec<(NodeIndex, gantz_core::Edge, NodeIndex)> = Vec::new();
    for edge in graph.edge_references() {
        let src = edge.source();
        let tgt = edge.target();
        let sel_src = nodes.contains(&src);
        let sel_tgt = nodes.contains(&tgt);
        if !sel_src && sel_tgt {
            incoming.push((tgt, edge.weight().clone(), src));
        } else if sel_src && !sel_tgt {
            outgoing.push((src, edge.weight().clone(), tgt));
        }
    }
    incoming.sort_by_key(|&(dst, ref w, _)| (dst.index(), w.input.0));
    outgoing.sort_by_key(|&(src, ref w, _)| (src.index(), w.output.0));

    // Capture the new node's position from the selection's centroid (falling back
    // to the view centre) before the nodes are removed.
    let pos = selection_centroid(view, nodes).unwrap_or(view.camera.center);

    // Build the nested graph: inlets, outlets, then the cut subgraph.
    let mut nested = DataGraph::default();
    let in_ixs: Vec<NodeIndex> = incoming
        .iter()
        .map(|_| {
            let inlet =
                gantz_core::data::erase_node_typed(&gantz_core::node::graph::Inlet::default())
                    .ok()?;
            Some(nested.add_node(inlet))
        })
        .collect::<Option<_>>()?;
    let out_ixs: Vec<NodeIndex> = outgoing
        .iter()
        .map(|_| {
            let outlet =
                gantz_core::data::erase_node_typed(&gantz_core::node::graph::Outlet::default())
                    .ok()?;
            Some(nested.add_node(outlet))
        })
        .collect::<Option<_>>()?;
    let subgraph = gantz_core::graph::extract_subgraph(graph, nodes);
    let new_indices = gantz_core::graph::add_subgraph(&mut nested, &subgraph);

    // Map old selected index -> subgraph index -> nested index, so the cut-point
    // connections can reach the moved nodes.
    let sorted: BTreeSet<_> = nodes.iter().copied().collect();
    let mut old_to_sub = HashMap::new();
    for (old_ix, sub_ix) in sorted.iter().zip(subgraph.node_indices()) {
        old_to_sub.insert(*old_ix, sub_ix);
    }
    let mut sub_to_nested = HashMap::new();
    for (sub_ix, &nested_ix) in subgraph.node_indices().zip(new_indices.iter()) {
        sub_to_nested.insert(sub_ix, nested_ix);
    }

    // Wire each inlet to its selected target and each selected source to its outlet.
    for (i, &(dst, ref w, _)) in incoming.iter().enumerate() {
        let dst_nested = sub_to_nested[&old_to_sub[&dst]];
        nested.add_edge(
            in_ixs[i],
            dst_nested,
            gantz_core::Edge::new(node::Output(0), w.input),
        );
    }
    for (j, &(src, ref w, _)) in outgoing.iter().enumerate() {
        let src_nested = sub_to_nested[&old_to_sub[&src]];
        nested.add_edge(
            src_nested,
            out_ixs[j],
            gantz_core::Edge::new(w.output, node::Input(0)),
        );
    }

    // Commit the fresh nested graph under the first free `<parent>:<n>` name.
    let graph_ca = gantz_ca::graph_addr(&nested);
    let name = next_child_name(registry, parent);
    registry.commit_graph_to_name(timestamp, graph_ca, || nested.clone(), &name);

    // Cut the selected nodes from the parent, then insert the new reference in
    // their place. Removal happens first so the new node's index is stable (as the
    // last node, unaffected by the swap-removals).
    remove_nodes(
        graph,
        vm,
        &mut view.layout,
        &mut head_state.scene.interaction.selection,
        instances,
        nodes.iter().copied(),
    );
    let named_ref = NamedRef::with_sync(name, node::Ref::new(graph_ca.into()));
    let node_data = gantz_core::data::erase_node_typed(&named_ref).ok()?;
    let new_ix = graph.add_node(node_data);

    // Re-wire the parent graph to the new node's sockets.
    for (i, &(_, ref w, external)) in incoming.iter().enumerate() {
        graph.add_edge(
            external,
            new_ix,
            gantz_core::Edge::new(w.output, node::Input(i as u16)),
        );
    }
    for (j, &(_, ref w, external)) in outgoing.iter().enumerate() {
        graph.add_edge(
            new_ix,
            external,
            gantz_core::Edge::new(node::Output(j as u16), w.input),
        );
    }

    // Position the new node and make it the sole selection.
    view.layout.insert(node_id(new_ix.index()), pos);
    let sel = &mut head_state.scene.interaction.selection;
    sel.nodes.clear();
    sel.edges.clear();
    sel.nodes.insert(new_ix);

    Some(new_ix)
}

/// A single node removal recorded by [`remove_nodes`]: the node at `removed`
/// was deleted, and (when `Some`) the node that was at `moved_from` was
/// swapped down into the `removed` slot.
#[derive(Clone, Copy, Debug)]
pub struct RemoveOp {
    pub removed: usize,
    pub moved_from: Option<usize>,
}

/// The ordered index changes performed by a [`remove_nodes`] call, for callers
/// that key persistent data by node index and must migrate it the same way (e.g.
/// detached node views - see `migrate_node_view_paths`).
#[derive(Clone, Debug, Default)]
pub struct Reindex(pub Vec<RemoveOp>);

impl Reindex {
    /// Whether any node was removed.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Replay the removals onto a single node index, returning its new index, or
    /// `None` if that node was the one removed. Mirrors how `remove_nodes`
    /// migrates state/layout/selection, so index-keyed data stays consistent.
    pub fn apply_to_index(&self, mut ix: usize) -> Option<usize> {
        for op in &self.0 {
            if ix == op.removed {
                return None;
            }
            if op.moved_from == Some(ix) {
                ix = op.removed;
            }
        }
        Some(ix)
    }
}

/// Remove `nodes` from `graph`, migrating the per-node state, layout,
/// selection and cached instances that are keyed by node index.
///
/// `petgraph::Graph::remove_node` swap-removes: the former-last node adopts the
/// removed index, so exactly one surviving node changes index per removal.
/// Targets are processed highest-index first, so a swap only ever pulls a
/// surviving node down into an already-freed higher slot and never invalidates a
/// pending target. The swapped node's state, layout entry and selection are then
/// moved to its new index. Edge selection is cleared because removing a node
/// drops its incident edges with compounded edge swaps.
///
/// Returns the ordered [`Reindex`] describing each removal/swap, so other
/// index-keyed data can be migrated the same way (any future reindexing edit
/// must do likewise).
///
/// Run this before the next recompile (`vm::sync`): the regenerated code reads
/// state by the new index, so the migration must already be in place.
pub fn remove_nodes(
    graph: &mut DataGraph,
    vm: &mut Engine,
    layout: &mut egui_graph::Layout,
    selection: &mut crate::widget::graph_scene::Selection,
    instances: &mut crate::node::NodeInstances,
    nodes: impl IntoIterator<Item = NodeIndex>,
) -> Reindex {
    let mut targets: Vec<NodeIndex> = nodes.into_iter().collect();
    targets.sort_unstable_by_key(|n| std::cmp::Reverse(n.index()));
    targets.dedup();
    let mut ops = Vec::new();
    for t in targets {
        if graph.node_weight(t).is_none() {
            continue;
        }
        let last = graph.node_count() - 1;
        // Drop the removed node's index-keyed data.
        let _ = node::state::remove_value(vm, &[t.index()]);
        layout.remove(&node_id(t.index()));
        selection.nodes.remove(&t);
        graph.remove_node(t);
        // Migrate the node that swapped into `t` (the former `last`), if any.
        let moved_from = (t.index() != last).then_some(last);
        if let Some(last) = moved_from {
            let _ = node::state::move_value(vm, &[last], &[t.index()]);
            if let Some(pos) = layout.remove(&node_id(last)) {
                layout.insert(node_id(t.index()), pos);
            }
            if selection.nodes.remove(&NodeIndex::new(last)) {
                selection.nodes.insert(t);
            }
        }
        ops.push(RemoveOp {
            removed: t.index(),
            moved_from,
        });
    }
    if !ops.is_empty() {
        selection.edges.clear();
    }
    let reindex = Reindex(ops);
    instances.apply_reindex(&reindex);
    reindex
}

/// Cut: serialize `nodes` to a `.gantz` clipboard payload, then remove them.
///
/// Returns the payload for the caller to write to the clipboard. Returns `None`
/// - removing nothing - when the selection is empty or serialization fails, so
/// a failed copy never loses nodes. Like [`remove_nodes`], run this before the
/// next recompile.
pub fn cut_nodes(
    registry: &gantz_ca::Registry,
    graph: &mut DataGraph,
    vm: &mut Engine,
    head_view: &mut crate::SceneView,
    selection: &mut crate::widget::graph_scene::Selection,
    instances: &mut crate::node::NodeInstances,
    nodes: &HashSet<NodeIndex>,
    codec: &NodeCodec,
) -> Option<String> {
    let text = copy_nodes(registry, graph, head_view, nodes, codec)?;
    remove_nodes(
        graph,
        vm,
        &mut head_view.layout,
        selection,
        instances,
        nodes.iter().copied(),
    );
    Some(text)
}

/// Insert an Inspect node on the given edge, splicing it between the
/// endpoints and positioning it at `cmd.pos`.
///
/// `new_inspect` produces the node's stored data form; the fresh node
/// reifies once through the `codec` for its VM registration step.
pub fn inspect_edge(
    codec: &NodeCodec,
    get_node: GetNode,
    new_inspect: impl FnOnce() -> Option<NodeData>,
    graph: &mut DataGraph,
    view: &mut crate::SceneView,
    vm: &mut Engine,
    cmd: InspectEdge,
) {
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

    // Register the new node with the VM (its one transient typed appearance).
    match codec.reify_ui(&graph[inspect_id]) {
        Ok(inst) => {
            let node_path = [inspect_id.index()];
            let reg_ctx = node::RegCtx::new(get_node, &node_path, vm);
            inst.node.register(reg_ctx);
        }
        Err(e) => log::error!("InspectEdge: cannot register the inspect node with the VM: {e}"),
    }

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
    let node_id = node_id(inspect_id.index());
    view.layout.insert(node_id, pos);
}

/// Paste a previously-copied clipboard payload into the graph at the head's
/// current path, and update the selection to the pasted nodes.
///
/// Returns `true` if a payload was pasted. The caller is responsible for
/// re-registering the root graph with the VM afterwards so pasted nodes get
/// their state initialized.
#[allow(clippy::too_many_arguments)]
pub fn paste(
    registry: &mut gantz_ca::Registry,
    editing: Option<&str>,
    graph: &mut DataGraph,
    head_view: &mut crate::SceneView,
    head_state: &mut OpenHeadState,
    text: &str,
    pos: &PastePos,
    codec: &NodeCodec,
) -> bool {
    let copied: export::Copied = match export::copied_from_str(text, codec) {
        Ok(c) => c,
        Err(e) => {
            log::debug!("Clipboard does not contain a valid gantz payload: {e}");
            return false;
        }
    };

    // Refuse the whole paste if any pasted `NamedRef` would reference the
    // editing graph (a cycle); with sync on such a cycle recommits endlessly
    // (see `crate::cycle`). Checked against the live registry before merging, so
    // a refused paste mutates nothing. A nameless (detached commit) head can't
    // be a name-based cycle target.
    if let Some(editing) = editing {
        let editing: Name = editing.parse().expect("infallible");
        if let Some(named) = copied
            .graph
            .node_weights()
            .filter_map(named_ref_of)
            .find(|nr| crate::cycle::would_cycle(registry, nr.name(), &editing))
        {
            log::warn!(
                "Paste: '{}' would create a reference cycle in '{editing}'; skipping paste",
                named.name()
            );
            return false;
        }
    }

    let offset = crate::resolve_paste_offset(pos, &copied.positions);

    let new_indices = export::paste(registry, graph, &mut head_view.layout, &copied, offset);

    // Update selection to the pasted nodes.
    head_state.scene.interaction.selection.nodes = new_indices.into_iter().collect();
    head_state.scene.interaction.selection.edges.clear();
    true
}

/// Duplicate `nodes` in place: serialize them, then [`paste`] at a small offset
/// (no clipboard involved). The selection becomes the new nodes.
///
/// Returns `true` if anything was duplicated. Like [`paste`], the caller
/// re-registers the root graph with the VM afterwards so the new nodes get
/// their state initialized.
pub fn duplicate_nodes(
    registry: &mut gantz_ca::Registry,
    editing: Option<&str>,
    graph: &mut DataGraph,
    head_view: &mut crate::SceneView,
    head_state: &mut OpenHeadState,
    nodes: &HashSet<NodeIndex>,
    codec: &NodeCodec,
) -> bool {
    let Some(text) = copy_nodes(registry, graph, head_view, nodes, codec) else {
        return false;
    };
    paste(
        registry,
        editing,
        graph,
        head_view,
        head_state,
        &text,
        &PastePos::Offset(egui::vec2(20.0, 20.0)),
        codec,
    )
}

/// Build a view for a navigation target commit that has no stored view by
/// carrying the live view's node positions forward through the navigation
/// node-identity `matching` (old index -> new index), keeping the live
/// camera.
///
/// Nodes of the new graph absent from the matching (e.g. merged-in from a
/// peer) are placed in a cascade from the camera centre. An empty live
/// layout, or a matching that
/// carries no positions at all, yields an empty result: the scene's one-shot
/// auto-layout is the right treatment for a genuinely layoutless graph (e.g.
/// a join placeholder), not a cascade of every node.
pub fn carry_layout(
    live: &crate::SceneView,
    matching: &gantz_ca::Matching,
    new_node_count: usize,
) -> crate::SceneView {
    let mut view = crate::SceneView {
        camera: live.camera,
        layout: Default::default(),
    };
    for (&old_ix, &new_ix) in matching {
        if let Some(pos) = live.layout.get(&node_id(old_ix)) {
            view.layout.insert(node_id(new_ix), *pos);
        }
    }
    if view.layout.is_empty() {
        return view;
    }
    let unmapped: Vec<usize> = (0..new_node_count)
        .filter(|&ix| !view.layout.contains_key(&node_id(ix)))
        .collect();
    for (i, ix) in unmapped.into_iter().enumerate() {
        let pos = cascade_pos(view.camera.center, i);
        view.layout.insert(node_id(ix), pos);
    }
    view
}

/// Build a view for a headlessly minted merge commit from the two parent
/// tips' stored views: each merged node takes its position from the first
/// (ours) tip's view where it survives there, falling back to the second
/// (theirs) tip's view, then to a cascade from the camera centre - the same
/// fallback as `apply_merge_migration`. The camera comes from whichever
/// side has a view, preferring the first.
///
/// When neither side has a view (or no position carries over at all) the
/// result is empty; callers must not store empty-layout views, as an
/// adopting peer would destructively auto-layout them.
pub fn merged_view(
    node_srcs: &[gantz_ca::merge::NodeSrc],
    first_view: Option<&crate::SceneView>,
    second_view: Option<&crate::SceneView>,
) -> crate::SceneView {
    let camera = first_view
        .or(second_view)
        .map(|v| v.camera)
        .unwrap_or_default();
    let mut view = crate::SceneView {
        camera,
        layout: Default::default(),
    };
    let mut missing = Vec::new();
    for (m, src) in node_srcs.iter().enumerate() {
        let pos = src
            .ours
            .and_then(|o| first_view.and_then(|v| v.layout.get(&node_id(o)).copied()))
            .or_else(|| {
                src.theirs
                    .and_then(|t| second_view.and_then(|v| v.layout.get(&node_id(t)).copied()))
            });
        match pos {
            Some(pos) => {
                view.layout.insert(node_id(m), pos);
            }
            None => missing.push(m),
        }
    }
    if view.layout.is_empty() {
        return view;
    }
    for (i, m) in missing.into_iter().enumerate() {
        let pos = cascade_pos(view.camera.center, i);
        view.layout.insert(node_id(m), pos);
    }
    view
}

/// Commit the current layout as a new commit on the head's *existing* graph
/// when node positions have changed since the head commit's frozen baseline
/// view, advancing `head` to the new commit.
///
/// The graph content is unchanged, so the new commit reuses the head's
/// [`gantz_ca::GraphAddr`]: the registry dedups the graph (the `graph` closure
/// passed to [`gantz_ca::Registry::commit_graph_to_head`] is never called) and
/// the VM does not need to recompile. Only `layout` (node positions) is
/// compared; the `camera` is excluded, so camera pan/zoom never produces a
/// layout commit.
///
/// Returns the new commit address when a layout commit was created, else `None`
/// (no baseline view yet - i.e. the head commit's view section entry has not
/// been seeded - or no node-position change). Seeding the new commit's view,
/// clearing the redo stack and migrating GUI state stay with the caller.
pub fn commit_layout(
    registry: &mut gantz_ca::Registry,
    timestamp: gantz_ca::Timestamp,
    head: &mut gantz_ca::Head,
    live: &crate::SceneView,
) -> Option<CommitAddr> {
    let head_commit_ca = registry.head_commit_ca(head)?;
    let baseline = crate::section::view(registry, &head_commit_ca)?;
    if baseline.layout == live.layout {
        return None;
    }
    let graph_addr = registry.commits().get(&head_commit_ca)?.graph;
    Some(registry.commit_graph_to_head(
        timestamp,
        graph_addr,
        || unreachable!("layout commit reuses an existing graph"),
        head,
    ))
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;
    use gantz_ca::Datum;

    /// A minimal erased node distinguished by its payload value.
    pub(crate) fn nd(v: u32) -> NodeData {
        let mut n = NodeData {
            tag: "Num".to_string(),
            data: Datum::Map(vec![("v".to_string(), Datum::I64(v as i64))]),
            refs: vec![],
            blobs: vec![],
        };
        n.canonicalize();
        n
    }

    /// The payload value a [`nd`]-built node carries.
    pub(crate) fn value(n: &NodeData) -> u32 {
        match n.data.get("v") {
            Some(&Datum::I64(v)) => v as u32,
            _ => panic!("not an `nd`-built node: {n:?}"),
        }
    }

    pub(crate) fn test_graph(nodes: &[u32]) -> DataGraph {
        let mut g = DataGraph::default();
        for &n in nodes {
            g.add_node(nd(n));
        }
        g
    }

    /// Commit `graph`, returning the new commit address.
    pub(crate) fn commit_test_graph(
        reg: &mut gantz_ca::Registry,
        secs: u64,
        parent: Option<CommitAddr>,
        graph: &DataGraph,
    ) -> CommitAddr {
        let ga = gantz_ca::graph_addr(graph);
        let dg = graph.clone();
        reg.commit_graph(std::time::Duration::from_secs(secs), parent, ga, || dg)
    }

    /// A registry where the branch `alpha` (returned as the head) and the
    /// branch `beta` diverge from a shared base.
    pub(crate) fn diverged_registry(
        base: &[u32],
        ours: &[u32],
        theirs: &[u32],
    ) -> (gantz_ca::Registry, gantz_ca::Head) {
        let mut reg = gantz_ca::Registry::default();
        let base_ca = commit_test_graph(&mut reg, 1, None, &test_graph(base));
        let ours_ca = commit_test_graph(&mut reg, 2, Some(base_ca), &test_graph(ours));
        let theirs_ca = commit_test_graph(&mut reg, 3, Some(base_ca), &test_graph(theirs));
        reg.set_head("alpha".parse().unwrap(), ours_ca);
        reg.set_head("beta".parse().unwrap(), theirs_ca);
        (reg, gantz_ca::Head::Branch("alpha".parse().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;
    use crate::widget::graph_scene::Selection;
    use gantz_core::node::graph::NodeIx;

    // Deleting a node swap-removes the former-last node into its slot; the
    // swapped node's layout entry and selection must follow it to the new index.
    #[test]
    fn remove_nodes_migrates_layout_and_selection() {
        // Five nodes 0..5 (weights 10..15) each with a distinct layout x.
        let mut graph = DataGraph::default();
        for w in 10u32..15 {
            graph.add_node(nd(w));
        }
        let mut layout = egui_graph::Layout::default();
        for i in 0..5 {
            layout.insert(node_id(i), egui::pos2(i as f32, 0.0));
        }
        let mut selection = Selection::default();
        selection.nodes.insert(NodeIx::new(4)); // select the (to-be-swapped) last

        let mut vm = Engine::new_base();

        // Seed a cache entry per node so the instance migration is exercised
        // too. The entries' weights are stand-ins (the `nd` weights aren't in
        // the test codec's manifest); `apply_reindex` migrates by key alone.
        let codec = crate::test_node::codec();
        let mut instances = crate::node::NodeInstances::default();
        let datas: Vec<_> = (0..5)
            .map(|i| {
                gantz_core::data::erase_node_typed(
                    &gantz_core::node::Expr::new(format!("(+ $l {i})")).unwrap(),
                )
                .unwrap()
            })
            .collect();
        for (i, d) in datas.iter().enumerate() {
            let entry = instances.take(&codec, i, d).unwrap();
            instances.put(i, entry);
        }

        // Delete index 1: node 4 (weight 14) swap-removes into slot 1.
        let reindex = remove_nodes(
            &mut graph,
            &mut vm,
            &mut layout,
            &mut selection,
            &mut instances,
            [NodeIx::new(1)],
        );
        assert!(!reindex.is_empty());
        // The reindex maps the swapped node (old index 4) down to 1, and reports
        // the deleted index 1 as gone.
        assert_eq!(reindex.apply_to_index(4), Some(1));
        assert_eq!(reindex.apply_to_index(1), None);

        // The swapped node now sits at index 1.
        assert_eq!(graph.node_count(), 4);
        assert_eq!(value(&graph[NodeIx::new(1)]), 14);

        // Layout followed the swap; the deleted and old-last slots are gone.
        assert_eq!(layout.len(), 4);
        assert_eq!(layout.get(&node_id(1)).copied(), Some(egui::pos2(4.0, 0.0)));
        assert!(!layout.contains_key(&node_id(4)));

        // Selection followed the swap: node 4 -> node 1.
        assert_eq!(
            selection.nodes.iter().copied().collect::<Vec<_>>(),
            vec![NodeIx::new(1)],
        );

        // Cached instances followed the swap: node 4's entry now lives at
        // index 1, the deleted and old-last slots are gone.
        assert_eq!(instances.len(), 4);
        assert!(instances.peek(1, &datas[4]).is_some());
        assert!(instances.peek(1, &datas[1]).is_none());
        assert!(instances.peek(4, &datas[4]).is_none());
    }

    // carry_layout maps live positions through the navigation matching, keeps
    // the camera, and cascades unmapped new nodes from the camera centre.
    #[test]
    fn carry_layout_remaps_and_places_new_nodes() {
        let mut live = crate::SceneView::default();
        live.camera.center = egui::pos2(100.0, 50.0);
        live.layout.insert(node_id(0), egui::pos2(1.0, 1.0));
        live.layout.insert(node_id(1), egui::pos2(2.0, 2.0));
        live.layout.insert(node_id(2), egui::pos2(3.0, 3.0));

        // 0 -> 1, 2 -> 0 (a swap-style remap); live node 1 was removed; the
        // new graph has an extra node at index 2 with no provenance.
        let matching: gantz_ca::Matching = [(0, 1), (2, 0)].into_iter().collect();
        let view = carry_layout(&live, &matching, 3);

        assert_eq!(view.camera, live.camera);
        assert_eq!(view.layout.len(), 3);
        assert_eq!(
            view.layout.get(&node_id(1)).copied(),
            Some(egui::pos2(1.0, 1.0))
        );
        assert_eq!(
            view.layout.get(&node_id(0)).copied(),
            Some(egui::pos2(3.0, 3.0))
        );
        // The unmapped new node cascades from the camera centre.
        assert_eq!(
            view.layout.get(&node_id(2)).copied(),
            Some(egui::pos2(100.0, 50.0))
        );
    }

    // An empty live layout carries nothing: the scene's one-shot auto-layout
    // is the right treatment for a genuinely layoutless graph.
    #[test]
    fn carry_layout_empty_live_yields_empty() {
        let live = crate::SceneView::default();
        let matching: gantz_ca::Matching = [(0, 0)].into_iter().collect();
        let view = carry_layout(&live, &matching, 4);
        assert!(view.layout.is_empty());
    }

    // When the matching carries no positions at all, the result stays empty
    // rather than cascading every node from the camera centre.
    #[test]
    fn carry_layout_no_carried_positions_yields_empty() {
        let mut live = crate::SceneView::default();
        live.layout.insert(node_id(5), egui::pos2(1.0, 1.0));
        let matching = gantz_ca::Matching::new();
        let view = carry_layout(&live, &matching, 3);
        assert!(view.layout.is_empty());
    }

    // merged_view sources each merged node's position from the first tip's
    // view where it survives there, falling back to the second tip's, then
    // cascading from the camera centre; the camera prefers the first side.
    #[test]
    fn merged_view_sources_ours_then_theirs_then_cascade() {
        let src = |ours: Option<usize>, theirs: Option<usize>| gantz_ca::merge::NodeSrc {
            base: None,
            ours,
            theirs,
        };
        let mut first = crate::SceneView::default();
        first.camera.center = egui::pos2(10.0, 10.0);
        first.layout.insert(node_id(0), egui::pos2(1.0, 0.0));
        let mut second = crate::SceneView::default();
        second.layout.insert(node_id(0), egui::pos2(2.0, 0.0));
        second.layout.insert(node_id(1), egui::pos2(3.0, 0.0));

        // Merged node 0 exists on both sides (ours wins); node 1 is
        // theirs-only; node 2 has no stored position anywhere.
        let srcs = [
            src(Some(0), Some(0)),
            src(None, Some(1)),
            src(Some(9), None),
        ];
        let view = merged_view(&srcs, Some(&first), Some(&second));

        assert_eq!(view.camera, first.camera);
        assert_eq!(
            view.layout.get(&node_id(0)).copied(),
            Some(egui::pos2(1.0, 0.0))
        );
        assert_eq!(
            view.layout.get(&node_id(1)).copied(),
            Some(egui::pos2(3.0, 0.0))
        );
        // The positionless node cascades from the camera centre.
        assert_eq!(
            view.layout.get(&node_id(2)).copied(),
            Some(egui::pos2(10.0, 10.0))
        );
    }

    // Without a stored view on either side, merged_view yields an empty
    // layout: callers must not store empty-layout views.
    #[test]
    fn merged_view_no_views_yields_empty() {
        let srcs = [gantz_ca::merge::NodeSrc {
            base: None,
            ours: Some(0),
            theirs: None,
        }];
        let view = merged_view(&srcs, None, None);
        assert!(view.layout.is_empty());
    }
}
