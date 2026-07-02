//! Shared operations behind the GUI's graph-mutating response payloads.
//!
//! Each fn implements the state change for one payload (e.g.
//! [`CreateNode`]) over plain graph/view/VM/registry types
//! so that frontends (e.g. `bevy_gantz_egui` and the pure-egui demo) remain
//! thin adapters around identical behaviour. Frontend-specific effects
//! (clipboard access, file dialogs, head navigation) stay with the caller.

use crate::sync::AsNamedRef;
use crate::widget::gantz::OpenHeadState;
use crate::widget::graph_scene::NodeIndex;
use crate::{CreateNode, InspectEdge, PastePos, export, node::NamedRef};
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
    all_views: &HashMap<CommitAddr, crate::SceneView>,
    graph: &Graph<N>,
    head_view: &crate::SceneView,
    selection: &HashSet<NodeIndex>,
) -> Option<String>
where
    N: gantz_core::Node
        + Clone
        + Serialize
        + DeserializeOwned
        + CaHash
        + gantz_format::NodeSugar
        + 'static,
{
    if selection.is_empty() {
        return None;
    }
    let copied = export::copy(registry, all_views, graph, selection, &head_view.layout);
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
    registry: &gantz_ca::Registry<Graph<N>>,
    editing: Option<&str>,
    get_node: GetNode,
    new_node: impl FnOnce(&str) -> Option<N>,
    graph: &mut Graph<N>,
    view: &mut crate::SceneView,
    head_state: &mut OpenHeadState,
    vm: &mut Engine,
    cmd: CreateNode,
) -> Option<NodeIndex>
where
    N: gantz_core::Node + crate::sync::AsNamedRef,
{
    let CreateNode { node_type, pos } = cmd;
    // Refuse references that would form a cycle back to the editing graph; with
    // sync on such a cycle recommits endlessly (see `crate::cycle`). A nameless
    // (detached commit) head can't be the target of a name-based cycle.
    if editing.is_some_and(|editing| crate::cycle::would_cycle(registry, &node_type, editing)) {
        log::warn!("CreateNode: '{node_type}' would create a reference cycle; skipping");
        return None;
    }
    let Some(node) = new_node(&node_type) else {
        log::error!("CreateNode: unknown node type: {node_type}");
        return None;
    };
    let node_ix = graph.add_node(node);

    // Register the new node with the VM.
    let node_path = [node_ix.index()];
    let reg_ctx = node::RegCtx::new(get_node, &node_path, vm);
    graph[node_ix].register(reg_ctx);

    // Position the new node under the pointer, falling back to the center of the
    // current view.
    let pos = pos.unwrap_or_else(|| view.camera.center);
    let egui_id = egui_graph::NodeId::from_u64(node_ix.index() as u64);
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
pub fn create_nested_graph<N>(
    registry: &mut gantz_ca::Registry<Graph<N>>,
    timestamp: std::time::Duration,
    graph: &mut Graph<N>,
    view: &mut crate::SceneView,
    head_state: &mut OpenHeadState,
    pos: Option<egui::Pos2>,
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

    // Position the new node under the pointer, falling back to the center of the
    // current view.
    let pos = pos.unwrap_or_else(|| view.camera.center);
    let egui_id = egui_graph::NodeId::from_u64(node_ix.index() as u64);
    view.layout.insert(egui_id, pos);

    // Make the new node the sole selection (clearing the previous one).
    let sel = &mut head_state.scene.interaction.selection;
    sel.nodes.clear();
    sel.edges.clear();
    sel.nodes.insert(node_ix);

    Some(node_ix)
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

/// Remove `nodes` from `graph`, migrating the per-node state, layout and
/// selection that are keyed by node index.
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
pub fn remove_nodes<N>(
    graph: &mut Graph<N>,
    vm: &mut Engine,
    layout: &mut egui_graph::Layout,
    selection: &mut crate::widget::graph_scene::Selection,
    nodes: impl IntoIterator<Item = NodeIndex>,
) -> Reindex {
    let node_id = |ix: usize| egui_graph::NodeId::from_u64(ix as u64);
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
    Reindex(ops)
}

/// Cut: serialize `nodes` to a `.gantz` clipboard payload, then remove them.
///
/// Returns the payload for the caller to write to the clipboard. Returns `None`
/// - removing nothing - when the selection is empty or serialization fails, so
/// a failed copy never loses nodes. Like [`remove_nodes`], run this before the
/// next recompile.
pub fn cut_nodes<N>(
    registry: &gantz_ca::Registry<Graph<N>>,
    all_views: &HashMap<CommitAddr, crate::SceneView>,
    graph: &mut Graph<N>,
    vm: &mut Engine,
    head_view: &mut crate::SceneView,
    selection: &mut crate::widget::graph_scene::Selection,
    nodes: &HashSet<NodeIndex>,
) -> Option<String>
where
    N: gantz_core::Node
        + Clone
        + Serialize
        + DeserializeOwned
        + CaHash
        + gantz_format::NodeSugar
        + 'static,
{
    let text = copy_nodes(registry, all_views, graph, head_view, nodes)?;
    remove_nodes(
        graph,
        vm,
        &mut head_view.layout,
        selection,
        nodes.iter().copied(),
    );
    Some(text)
}

/// Insert an Inspect node on the given edge, splicing it between the
/// endpoints and positioning it at `cmd.pos`.
pub fn inspect_edge<N>(
    get_node: GetNode,
    new_inspect: impl FnOnce() -> Option<N>,
    graph: &mut Graph<N>,
    view: &mut crate::SceneView,
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
    editing: Option<&str>,
    all_views: &mut HashMap<CommitAddr, crate::SceneView>,
    all_demos: &mut HashMap<String, String>,
    graph: &mut Graph<N>,
    head_view: &mut crate::SceneView,
    head_state: &mut OpenHeadState,
    text: &str,
    pos: &PastePos,
) -> bool
where
    N: Clone
        + Serialize
        + DeserializeOwned
        + CaHash
        + AsNamedRef
        + gantz_format::NodeSugar
        + 'static,
{
    let copied: export::Copied<N> = match export::copied_from_str(text) {
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
        if let Some(named) = copied
            .graph
            .node_weights()
            .filter_map(|n| n.as_named_ref())
            .find(|nr| crate::cycle::would_cycle(registry, nr.name(), editing))
        {
            log::warn!(
                "Paste: '{}' would create a reference cycle in '{editing}'; skipping paste",
                named.name()
            );
            return false;
        }
    }

    let offset = crate::resolve_paste_offset(pos, &copied.positions);

    let new_indices = export::paste(
        registry,
        all_views,
        all_demos,
        graph,
        &mut head_view.layout,
        &copied,
        offset,
    );

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
pub fn duplicate_nodes<N>(
    registry: &mut gantz_ca::Registry<Graph<N>>,
    editing: Option<&str>,
    all_views: &mut HashMap<CommitAddr, crate::SceneView>,
    all_demos: &mut HashMap<String, String>,
    graph: &mut Graph<N>,
    head_view: &mut crate::SceneView,
    head_state: &mut OpenHeadState,
    nodes: &HashSet<NodeIndex>,
) -> bool
where
    N: gantz_core::Node
        + Clone
        + Serialize
        + DeserializeOwned
        + CaHash
        + AsNamedRef
        + gantz_format::NodeSugar
        + 'static,
{
    let Some(text) = copy_nodes(registry, all_views, graph, head_view, nodes) else {
        return false;
    };
    paste(
        registry,
        editing,
        all_views,
        all_demos,
        graph,
        head_view,
        head_state,
        &text,
        &PastePos::Offset(egui::vec2(20.0, 20.0)),
    )
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

/// The result of a [`merge_head`] call.
#[derive(Debug)]
pub enum MergeHeadOutcome {
    /// Ours had no changes since the merge base: nothing was mutated and no
    /// commit was made. The caller navigates the head to this commit, which
    /// reloads the working graph and views.
    FastForward(CommitAddr),
    /// The merge was applied to the working graph and committed with two
    /// parents; `head` has been advanced. `mapping` records where each of the
    /// pre-merge graph's nodes ended up (old index to new index; absent =
    /// removed), for any remaining index-keyed data of the caller's. The
    /// caller re-registers the graph with the VM (merged-in nodes need their
    /// state initialized) and fires its committed/resync machinery.
    Merged {
        new_commit: CommitAddr,
        mapping: gantz_ca::Matching,
    },
    /// Conflicts (without `auto_resolve`) or hard blockers refused the merge;
    /// nothing was mutated. Carries the rendered reasons.
    Refused(Vec<String>),
    /// Nothing to do: unknown source, unrelated histories, or already up to
    /// date.
    Noop,
}

/// Merge the branch named `source` into `head`, applying the result to the
/// head's working `graph` in place (see [`gantz_ca::merge_commits`]).
///
/// On a true merge this migrates the index-keyed VM state, layout and
/// selection through the merged graph's node mapping (an identity mapping
/// whenever the source branch removed no nodes), seeds layout for merged-in
/// nodes from the source branch's persisted view in `all_views` (falling back
/// to placement near the view centre), and commits the result with two
/// parents via [`gantz_ca::Registry::commit_merge_to_head`] - upholding the
/// committed-working-graph invariant, so callers must not commit again.
///
/// Conflicts refuse the merge unless `auto_resolve` accepts the given
/// `resolutions`; hard blockers (a merged-in reference cycle) always refuse.
/// Fast-forwards mutate nothing - the caller navigates the head instead.
#[allow(clippy::too_many_arguments)]
pub fn merge_head<N>(
    registry: &mut gantz_ca::Registry<Graph<N>>,
    all_views: &HashMap<CommitAddr, crate::SceneView>,
    timestamp: gantz_ca::Timestamp,
    head: &mut gantz_ca::Head,
    graph: &mut Graph<N>,
    vm: &mut Engine,
    head_view: &mut crate::SceneView,
    selection: &mut crate::widget::graph_scene::Selection,
    source: &str,
    resolutions: gantz_ca::Resolutions,
    auto_resolve: bool,
) -> MergeHeadOutcome
where
    N: Clone + CaHash + AsNamedRef,
{
    let node_id = |ix: usize| egui_graph::NodeId::from_u64(ix as u64);
    let Some(&ours_tip) = registry.head_commit_ca(head) else {
        log::error!("MergeHead: no commit for head {head}");
        return MergeHeadOutcome::Noop;
    };
    let Some(&theirs_tip) = registry.names().get(source) else {
        log::error!("MergeHead: unknown source branch '{source}'");
        return MergeHeadOutcome::Noop;
    };
    let outcome = match gantz_ca::merge_commits(registry, ours_tip, theirs_tip, resolutions) {
        Err(e) => {
            log::warn!("MergeHead: cannot merge '{source}': {e}");
            return MergeHeadOutcome::Noop;
        }
        Ok(gantz_ca::MergeResolution::AlreadyUpToDate) => return MergeHeadOutcome::Noop,
        Ok(gantz_ca::MergeResolution::FastForward) => {
            return MergeHeadOutcome::FastForward(theirs_tip);
        }
        Ok(gantz_ca::MergeResolution::Diverged { outcome, .. }) => outcome,
    };

    // Refuse on hard blockers, and on conflicts unless the caller opted into
    // the selected resolutions.
    let blockers = crate::merge::merge_blockers(registry, head, &outcome.graph);
    if !blockers.is_empty() {
        return MergeHeadOutcome::Refused(blockers);
    }
    if !outcome.conflicts.is_empty() && !auto_resolve {
        return MergeHeadOutcome::Refused(crate::merge::conflict_strings(&outcome.conflicts));
    }

    // Where each pre-merge (ours) node ended up, and where each source
    // (theirs) node ended up. By the committed-working-graph invariant the
    // working graph *is* ours' tip graph, so "ours" indices are the working
    // graph's.
    let mut ours_map = gantz_ca::Matching::new();
    let mut theirs_only = Vec::new();
    for (m, src) in outcome.node_srcs.iter().enumerate() {
        match (src.ours, src.theirs) {
            (Some(o), _) => {
                ours_map.insert(o, m);
            }
            (None, Some(t)) => theirs_only.push((m, t)),
            (None, None) => unreachable!("a merged node comes from somewhere"),
        }
    }

    // Migrate the index-keyed VM state, layout and selection. When the source
    // branch removed no nodes the mapping is identity and this is a no-op.
    if let Err(e) = node::state::remap_root(vm, &ours_map) {
        log::error!("MergeHead: failed to remap node state: {e}");
    }
    let old_layout = std::mem::take(&mut head_view.layout);
    for (&o, &m) in &ours_map {
        if let Some(pos) = old_layout.get(&node_id(o)) {
            head_view.layout.insert(node_id(m), *pos);
        }
    }
    selection.nodes = selection
        .nodes
        .iter()
        .filter_map(|n| ours_map.get(&n.index()).map(|&m| NodeIndex::new(m)))
        .collect();
    selection.edges.clear();

    // Seed layout for merged-in nodes from the source branch's persisted view
    // (positions are compatible: both branches share the base's coordinates),
    // falling back to placement near the view centre.
    let theirs_view = all_views.get(&theirs_tip);
    for (i, &(m, t)) in theirs_only.iter().enumerate() {
        let pos = theirs_view
            .and_then(|v| v.layout.get(&node_id(t)).copied())
            .unwrap_or_else(|| head_view.camera.center + egui::vec2(20.0, 20.0) * i as f32);
        head_view.layout.insert(node_id(m), pos);
    }

    // Swap in the merged graph and commit it with both parents.
    *graph = outcome.graph;
    let new_commit = registry.commit_merge_to_head(
        timestamp,
        gantz_ca::graph_addr(&*graph),
        || graph.clone(),
        theirs_tip,
        head,
    );
    MergeHeadOutcome::Merged {
        new_commit,
        mapping: ours_map,
    }
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
/// (no baseline view yet - i.e. the head commit's layout has not been seeded -
/// or no node-position change). Seeding `views[new]`, clearing the redo stack
/// and migrating GUI state stay with the caller.
pub fn commit_layout<G>(
    registry: &mut gantz_ca::Registry<G>,
    views: &HashMap<CommitAddr, crate::SceneView>,
    timestamp: gantz_ca::Timestamp,
    head: &mut gantz_ca::Head,
    live: &crate::SceneView,
) -> Option<CommitAddr> {
    let head_commit_ca = *registry.head_commit_ca(head)?;
    let baseline = views.get(&head_commit_ca)?;
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
mod tests {
    use super::*;
    use crate::widget::graph_scene::Selection;
    use gantz_core::ROOT_STATE;
    use gantz_core::node::graph::NodeIx;
    use steel::SteelVal;

    // Deleting a node swap-removes the former-last node into its slot; the
    // swapped node's layout entry and selection must follow it to the new index.
    #[test]
    fn remove_nodes_migrates_layout_and_selection() {
        let node_id = |i: usize| egui_graph::NodeId::from_u64(i as u64);

        // Five nodes 0..5 (weights 10..15) each with a distinct layout x.
        let mut graph: Graph<u32> = Graph::default();
        for w in 10u32..15 {
            graph.add_node(w);
        }
        let mut layout = egui_graph::Layout::default();
        for i in 0..5 {
            layout.insert(node_id(i), egui::pos2(i as f32, 0.0));
        }
        let mut selection = Selection::default();
        selection.nodes.insert(NodeIx::new(4)); // select the (to-be-swapped) last

        let mut vm = Engine::new_base();

        // Delete index 1: node 4 (weight 14) swap-removes into slot 1.
        let reindex = remove_nodes(
            &mut graph,
            &mut vm,
            &mut layout,
            &mut selection,
            [NodeIx::new(1)],
        );
        assert!(!reindex.is_empty());
        // The reindex maps the swapped node (old index 4) down to 1, and reports
        // the deleted index 1 as gone.
        assert_eq!(reindex.apply_to_index(4), Some(1));
        assert_eq!(reindex.apply_to_index(1), None);

        // The swapped node now sits at index 1.
        assert_eq!(graph.node_count(), 4);
        assert_eq!(graph[NodeIx::new(1)], 14);

        // Layout followed the swap; the deleted and old-last slots are gone.
        assert_eq!(layout.len(), 4);
        assert_eq!(layout.get(&node_id(1)).copied(), Some(egui::pos2(4.0, 0.0)));
        assert!(!layout.contains_key(&node_id(4)));

        // Selection followed the swap: node 4 -> node 1.
        assert_eq!(
            selection.nodes.iter().copied().collect::<Vec<_>>(),
            vec![NodeIx::new(1)],
        );
    }

    /// A minimal node type satisfying [`merge_head`]'s bounds.
    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestNode(u32);

    impl CaHash for TestNode {
        fn hash(&self, hasher: &mut gantz_ca::Hasher) {
            CaHash::hash(&self.0, hasher);
        }
    }

    impl AsNamedRef for TestNode {
        fn as_named_ref(&self) -> Option<&NamedRef> {
            None
        }
    }

    fn test_graph(nodes: &[u32]) -> Graph<TestNode> {
        let mut g = Graph::default();
        for &n in nodes {
            g.add_node(TestNode(n));
        }
        g
    }

    fn node_id(ix: usize) -> egui_graph::NodeId {
        egui_graph::NodeId::from_u64(ix as u64)
    }

    /// A registry where the branch `alpha` (returned as the head) and the
    /// branch `beta` diverge from a shared base.
    fn diverged_registry(
        base: &[u32],
        ours: &[u32],
        theirs: &[u32],
    ) -> (gantz_ca::Registry<Graph<TestNode>>, gantz_ca::Head) {
        let secs = |s| std::time::Duration::from_secs(s);
        let mut reg = gantz_ca::Registry::default();
        let g = test_graph(base);
        let base_ca = reg.commit_graph(secs(1), None, gantz_ca::graph_addr(&g), || g);
        let g = test_graph(ours);
        let ours_ca = reg.commit_graph(secs(2), Some(base_ca), gantz_ca::graph_addr(&g), || g);
        let g = test_graph(theirs);
        let theirs_ca = reg.commit_graph(secs(3), Some(base_ca), gantz_ca::graph_addr(&g), || g);
        reg.insert_name("alpha".to_string(), ours_ca);
        reg.insert_name("beta".to_string(), theirs_ca);
        (reg, gantz_ca::Head::Branch("alpha".to_string()))
    }

    #[allow(clippy::type_complexity)]
    fn run_merge(
        reg: &mut gantz_ca::Registry<Graph<TestNode>>,
        head: &mut gantz_ca::Head,
        graph: &mut Graph<TestNode>,
        vm: &mut Engine,
        view: &mut crate::SceneView,
        selection: &mut Selection,
        auto_resolve: bool,
    ) -> MergeHeadOutcome {
        merge_head(
            reg,
            &HashMap::new(),
            std::time::Duration::from_secs(9),
            head,
            graph,
            vm,
            view,
            selection,
            "beta",
            gantz_ca::Resolutions::default(),
            auto_resolve,
        )
    }

    // Ours edited a node while theirs added one: the merge keeps ours' indices
    // (identity mapping), applies theirs' addition, and commits two parents.
    #[test]
    fn merge_head_applies_theirs_and_commits_two_parents() {
        let (mut reg, mut head) = diverged_registry(&[1, 2], &[1, 20], &[1, 2, 3]);
        let ours_tip = *reg.head_commit_ca(&head).unwrap();
        let theirs_tip = reg.names()["beta"];
        let mut graph = test_graph(&[1, 20]);
        let mut vm = Engine::new_base();
        let mut view = crate::SceneView::default();
        view.layout.insert(node_id(0), egui::pos2(0.0, 0.0));
        view.layout.insert(node_id(1), egui::pos2(1.0, 0.0));
        let mut selection = Selection::default();
        selection.nodes.insert(NodeIx::new(1));

        let outcome = run_merge(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            false,
        );
        let MergeHeadOutcome::Merged { new_commit, .. } = outcome else {
            panic!("expected Merged, got {outcome:?}");
        };

        // The merged graph keeps ours' nodes in place and appends theirs' add.
        let weights: Vec<u32> = graph.node_weights().map(|n| n.0).collect();
        assert_eq!(weights, vec![1, 20, 3]);
        // Ours' layout and selection are untouched; the merged-in node has a
        // (fallback) layout entry.
        assert_eq!(view.layout.get(&node_id(1)), Some(&egui::pos2(1.0, 0.0)));
        assert!(view.layout.contains_key(&node_id(2)));
        assert!(selection.nodes.contains(&NodeIx::new(1)));
        // The commit joins both parents and the head advanced to it.
        let commit = &reg.commits()[&new_commit];
        assert_eq!(commit.parent, Some(ours_tip));
        assert_eq!(commit.merge_parents, vec![theirs_tip]);
        assert_eq!(reg.head_commit_ca(&head), Some(&new_commit));
    }

    // Theirs removed a node: ours' surviving state/layout/selection migrate
    // through the returned mapping.
    #[test]
    fn merge_head_migrates_state_layout_selection_on_removal() {
        let (mut reg, mut head) = diverged_registry(&[1, 2], &[1, 2], &[2]);
        let mut graph = test_graph(&[1, 2]);
        let mut vm = Engine::new_base();
        vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
        node::state::update_value(&mut vm, &[1], SteelVal::IntV(42)).unwrap();
        let mut view = crate::SceneView::default();
        view.layout.insert(node_id(0), egui::pos2(0.0, 0.0));
        view.layout.insert(node_id(1), egui::pos2(1.0, 0.0));
        let mut selection = Selection::default();
        selection.nodes.insert(NodeIx::new(1));

        let outcome = run_merge(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            false,
        );
        let MergeHeadOutcome::Merged { mapping, .. } = outcome else {
            panic!("expected Merged, got {outcome:?}");
        };

        // Node 2 (ours ix 1) survives at merged ix 0.
        assert_eq!(mapping, gantz_ca::Matching::from([(1, 0)]));
        let weights: Vec<u32> = graph.node_weights().map(|n| n.0).collect();
        assert_eq!(weights, vec![2]);
        // Its state, layout and selection followed.
        let state = node::state::extract_value(&vm, &[0]).unwrap();
        assert_eq!(state, Some(SteelVal::IntV(42)));
        assert_eq!(view.layout.len(), 1);
        assert_eq!(view.layout.get(&node_id(0)), Some(&egui::pos2(1.0, 0.0)));
        assert_eq!(
            selection.nodes.iter().copied().collect::<Vec<_>>(),
            vec![NodeIx::new(0)],
        );
    }

    // Conflicting edits refuse the merge (mutating nothing) unless the caller
    // opts into the default resolutions.
    #[test]
    fn merge_head_refuses_conflicts_unless_auto_resolve() {
        let (mut reg, mut head) = diverged_registry(&[1, 2], &[1, 20], &[1, 30]);
        let ours_tip = *reg.head_commit_ca(&head).unwrap();
        let mut graph = test_graph(&[1, 20]);
        let mut vm = Engine::new_base();
        let mut view = crate::SceneView::default();
        let mut selection = Selection::default();

        let outcome = run_merge(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            false,
        );
        let MergeHeadOutcome::Refused(reasons) = outcome else {
            panic!("expected Refused, got {outcome:?}");
        };
        assert!(!reasons.is_empty());
        // Nothing moved.
        assert_eq!(reg.head_commit_ca(&head), Some(&ours_tip));
        assert_eq!(
            graph.node_weights().map(|n| n.0).collect::<Vec<_>>(),
            [1, 20]
        );

        // Opting in applies the default resolution (ours wins).
        let outcome = run_merge(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            true,
        );
        assert!(matches!(outcome, MergeHeadOutcome::Merged { .. }));
        assert_eq!(
            graph.node_weights().map(|n| n.0).collect::<Vec<_>>(),
            [1, 20]
        );
        assert_ne!(reg.head_commit_ca(&head), Some(&ours_tip));
    }

    // A source branch that is strictly ahead fast-forwards without a commit.
    #[test]
    fn merge_head_fast_forwards() {
        let secs = |s| std::time::Duration::from_secs(s);
        let mut reg = gantz_ca::Registry::default();
        let g = test_graph(&[1]);
        let base_ca = reg.commit_graph(secs(1), None, gantz_ca::graph_addr(&g), || g);
        let g = test_graph(&[1, 2]);
        let theirs_ca = reg.commit_graph(secs(2), Some(base_ca), gantz_ca::graph_addr(&g), || g);
        reg.insert_name("alpha".to_string(), base_ca);
        reg.insert_name("beta".to_string(), theirs_ca);
        let mut head = gantz_ca::Head::Branch("alpha".to_string());
        let mut graph = test_graph(&[1]);
        let mut vm = Engine::new_base();
        let mut view = crate::SceneView::default();
        let mut selection = Selection::default();

        let outcome = run_merge(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            false,
        );
        let MergeHeadOutcome::FastForward(target) = outcome else {
            panic!("expected FastForward, got {outcome:?}");
        };
        assert_eq!(target, theirs_ca);
        // Nothing mutated: navigation is the caller's job.
        assert_eq!(reg.names()["alpha"], base_ca);
        assert_eq!(graph.node_count(), 1);
    }
}
