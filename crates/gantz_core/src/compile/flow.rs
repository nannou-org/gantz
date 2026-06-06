//! Items related to constructing a view of the control flow of a gantz graph.

use super::{
    EntrypointId, Meta, MetaGraph,
    error::{InvalidInputIndex, InvalidOutputIndex, NodeConnsError, TooManyConns},
    meta::EdgeKind,
    push_eval_neighbors, push_reachable,
};
use crate::node;
use petgraph::{
    graph::NodeIndex,
    visit::{EdgeRef, IntoEdgeReferences},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    fmt, ops,
};

/// Error when computing node inputs from graph edges.
#[derive(Debug, thiserror::Error)]
pub enum NodeInputsError {
    /// The node has too many inputs.
    #[error(transparent)]
    TooManyInputs(#[from] TooManyConns),
    /// An edge references an invalid input index.
    #[error(transparent)]
    InvalidIndex(#[from] InvalidInputIndex),
}

/// Error when computing node outputs from graph edges.
#[derive(Debug, thiserror::Error)]
pub enum NodeOutputsError {
    /// The node has too many outputs.
    #[error(transparent)]
    TooManyOutputs(#[from] TooManyConns),
    /// An edge references an invalid output index.
    #[error(transparent)]
    InvalidIndex(#[from] InvalidOutputIndex),
}

/// Represents all control flow graphs for all entrypoints in a single gantz graph.
///
/// This includes all branches on the edges, and unique node configurations as
/// nodes.
#[derive(Debug)]
pub struct Flow {
    /// Control flow graph from all inlets to all outlets, or empty in the case
    /// that the graph has no inlets or outlets (i.e. is not nested).
    pub nested: FlowGraph,
    /// Control flow graph for each entrypoint at this graph level.
    /// An entrypoint appears here only if it has sources at this level.
    pub entrypoints: BTreeMap<EntrypointId, FlowGraph>,
    /// For each entrypoint, what its FlowGraph reaches in terms of this graph's
    /// outlets. Only populated for entrypoints whose flow reaches at least one
    /// outlet.
    pub outlet_reach: BTreeMap<EntrypointId, OutletReach>,
}

/// What one entrypoint's flow graph reaches among its graph's outlets, for
/// push-through-outlet propagation to the parent.
#[derive(Clone, Debug)]
pub struct OutletReach {
    /// The outlet node ids reached (the union over all branch outcomes).
    pub reached: BTreeSet<node::Id>,
    /// The distinct external branch masks (over this graph's outputs) the push
    /// can produce, or empty when it always produces the same outlets (so no
    /// branch-aware propagation is needed). See `branch_patterns_from_flow`.
    pub patterns: Vec<node::Conns>,
}

/// Represents a basic, linear block of node function calls.
#[derive(Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct Block(pub Vec<NodeConf>);

/// The control flow graph.
///
/// Nodes represent basic blocks of node function calls, edges represent the
/// unique output branching that leads between blocks.
pub type FlowGraph = petgraph::stable_graph::StableDiGraph<Block, Branch>;

/// A branch from a node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct Branch {
    /// The index that indicates the branch was taken.
    pub ix: usize,
    /// The active outputs for the branch.
    pub conns: node::Conns,
}

/// A node within the control flow graph.
///
/// Maps directly to a node function.
#[derive(Copy, Clone, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct NodeConf {
    pub id: node::Id,
    pub conns: NodeConns,
}

/// The connectedness of a node for a particular evaluation step.
#[derive(Copy, Clone, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct NodeConns {
    /// The active inputs.
    pub inputs: node::Conns,
    /// Includes all connected outputs (whether conditional or not).
    pub outputs: node::Conns,
}

impl fmt::Debug for NodeConf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:?}: {:?})", self.id, self.conns)
    }
}

impl fmt::Debug for NodeConns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "([{}], [{}])", self.inputs, self.outputs)
    }
}

impl ops::Deref for Block {
    type Target = Vec<NodeConf>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for Block {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<NodeInputsError> for NodeConnsError {
    fn from(e: NodeInputsError) -> Self {
        match e {
            NodeInputsError::TooManyInputs(e) => NodeConnsError::TooManyConns(e),
            NodeInputsError::InvalidIndex(e) => NodeConnsError::InvalidInputIndex(e),
        }
    }
}

impl From<NodeOutputsError> for NodeConnsError {
    fn from(e: NodeOutputsError) -> Self {
        match e {
            NodeOutputsError::TooManyOutputs(e) => NodeConnsError::TooManyConns(e),
            NodeOutputsError::InvalidIndex(e) => NodeConnsError::InvalidOutputIndex(e),
        }
    }
}

/// Given a meta graph and set of push and pull eval fn nodes, construct a full
/// control flow graph.
///
/// Each weakly-connected component of the reachable subgraph is lowered
/// independently - so independent chains and parallel branches each form their
/// own flow-graph component - then merged into a single flow graph.
pub fn flow_graph(
    meta: &Meta,
    push: impl IntoIterator<Item = (node::Id, node::Conns)>,
    pull: impl IntoIterator<Item = (node::Id, node::Conns)>,
) -> Result<FlowGraph, NodeConnsError> {
    flow_graph_with_extra(meta, push, pull, &BTreeMap::new())
}

/// Like [`flow_graph`], but treats `extra_branches` (graph-node id -> per-arm
/// output masks) as branch nodes in addition to `meta.branches`.
///
/// Used for branch-aware push-through-outlet propagation, where a bridged graph
/// node branches for a *specific entrypoint only* (so its masks can't live in
/// the graph-wide `meta.branches`).
pub(crate) fn flow_graph_with_extra(
    meta: &Meta,
    push: impl IntoIterator<Item = (node::Id, node::Conns)>,
    pull: impl IntoIterator<Item = (node::Id, node::Conns)>,
    extra_branches: &BTreeMap<node::Id, Vec<node::Conns>>,
) -> Result<FlowGraph, NodeConnsError> {
    let order: Vec<_> = super::eval_order(&meta.graph, push, pull).collect();
    let included: HashSet<_> = order.iter().copied().collect();
    let mg = reachable_subgraph(&meta.graph, &included);
    // Graph-wide branches plus this entrypoint's push-through branch nodes.
    let mut all_branches = meta.branches.clone();
    all_branches.extend(extra_branches.iter().map(|(&id, m)| (id, m.clone())));
    let mut fg = FlowGraph::default();
    let no_shared = HashMap::new();
    build_flow_graph(meta, &mg, &mg, &mut fg, &no_shared, None, &all_branches)?;
    let branching: BTreeSet<node::Id> = all_branches.keys().copied().collect();
    flow_graph_edge_contraction(&mut fg, &branching);
    Ok(fg)
}

/// Build a [`NodeConf`] for `n` from its connectivity within the (possibly
/// arm-local) `mg`, treating `Static` edges from `outer_mg` as still in scope.
fn make_conf(
    meta: &Meta,
    mg: &MetaGraph,
    outer_mg: &MetaGraph,
    n: node::Id,
) -> Result<NodeConf, NodeConnsError> {
    let n_inputs = meta.inputs.get(&n).copied().unwrap_or(0);
    let n_outputs = meta.outputs.get(&n).copied().unwrap_or(0);
    let inputs = node_inputs_in_scope(mg, outer_mg, n, n_inputs)?;
    let outputs = node_outputs(mg, n, n_outputs)?;
    Ok(NodeConf {
        id: n,
        conns: NodeConns { inputs, outputs },
    })
}

/// The connected inputs of `n`, counting an incoming edge when its source is in
/// the current arm subgraph `mg`, or when the edge is `Static` (always active,
/// so reachable from an enclosing scope).
fn node_inputs_in_scope(
    mg: &MetaGraph,
    outer_mg: &MetaGraph,
    n: node::Id,
    n_inputs: usize,
) -> Result<node::Conns, NodeInputsError> {
    let mut inputs = node::Conns::unconnected(n_inputs).map_err(|_| TooManyConns(n_inputs))?;
    for e_ref in outer_mg.edges_directed(n, petgraph::Incoming) {
        let src = e_ref.source();
        for (edge, kind) in e_ref.weight() {
            let in_arm = mg.contains_node(src);
            let static_from_outer = matches!(kind, EdgeKind::Static);
            if !(in_arm || static_from_outer) {
                continue;
            }
            let index = edge.input.0 as usize;
            inputs
                .set(index, true)
                .map_err(|_| InvalidInputIndex { index, n_inputs })?;
        }
    }
    Ok(inputs)
}

/// Lower the meta graph `mg` into flow-graph blocks, returning this call's entry
/// blocks (those with no incoming edge added here).
///
/// Branches are lowered by *duplicating* per-arm: each arm recursion allocates
/// fresh blocks, so a node reached by only some arms appears once per reaching
/// arm. A node reached by *every* live arm via either an intermediate or
/// multiple distinct outputs is a true reconvergence point ("join"); it is
/// pre-allocated as a single shared block so all arms converge on it, and a
/// continuation pass lowers its downstream exactly once.
///
/// - `outer_mg` is the full graph (for `Static`-edge input scoping in arms).
/// - `shared` maps join node ids to their pre-allocated block.
/// - `skip` is the branching node owning this arm recursion (already emitted by
///   the caller); present only in arm recursions.
fn build_flow_graph(
    meta: &Meta,
    mg: &MetaGraph,
    outer_mg: &MetaGraph,
    fg: &mut FlowGraph,
    shared: &HashMap<node::Id, NodeIndex>,
    skip: Option<node::Id>,
    all_branches: &BTreeMap<node::Id, Vec<node::Conns>>,
) -> Result<Vec<NodeIndex>, NodeConnsError> {
    let mut topo = petgraph::visit::Topo::new(mg);
    let mut last: Option<NodeIndex> = None;
    let mut entries: Vec<NodeIndex> = Vec::new();
    let mut consumed: HashSet<node::Id> = HashSet::new();
    // Blocks allocated/reused in this call, so a shared block can link from real
    // meta predecessors processed earlier here.
    let mut local_blocks: HashMap<node::Id, NodeIndex> = HashMap::new();
    // In an arm recursion, shared (join) blocks are arm terminals: chain into
    // them but stop; the owner's continuation pass chains out of them once.
    let stop_at_shared = skip.is_some();

    while let Some(n) = topo.next(mg) {
        if consumed.contains(&n) || Some(n) == skip {
            continue;
        }

        // Reuse a shared join block, else allocate a fresh one. Fresh allocation
        // gives the same node id distinct identity in sibling arms (duplication).
        let is_shared = shared.contains_key(&n);
        let block_ix = match shared.get(&n) {
            Some(&ix) => ix,
            None => {
                let conf = make_conf(meta, mg, outer_mg, n)?;
                fg.add_node(Block(vec![conf]))
            }
        };
        local_blocks.insert(n, block_ix);

        // Incoming edges: shared blocks link from their real meta predecessors
        // (so sibling joins don't get fake chain edges); non-shared blocks chain
        // via `last` so parallel siblings collapse into one block after
        // contraction.
        let mut incoming_added = false;
        if is_shared {
            for e_ref in mg.edges_directed(n, petgraph::Incoming) {
                let src_id = e_ref.source();
                if Some(src_id) == skip {
                    continue;
                }
                if let Some(&src_block) = local_blocks.get(&src_id) {
                    let branch = default_branch(fg, src_block);
                    fg.add_edge(src_block, block_ix, branch);
                    incoming_added = true;
                }
            }
        } else if let Some(prev_ix) = last {
            let branch = default_branch(fg, prev_ix);
            fg.add_edge(prev_ix, block_ix, branch);
            incoming_added = true;
        }

        // No incoming edge added: a fresh entry into this recursion. The arm
        // caller links the arm-branch edge to it.
        if !incoming_added {
            entries.push(block_ix);
        }

        // In an arm recursion, stop at shared blocks; the continuation owns them.
        if is_shared && stop_at_shared {
            last = None;
            continue;
        }

        if let Some(branches) = all_branches.get(&n) {
            let per_arm: Vec<HashSet<node::Id>> = branches
                .iter()
                .map(|conns| push_reachable(mg, n, &push_eval_neighbors(mg, n, conns)).collect())
                .collect();

            // Live arms reach beyond `n`; a dead arm reaching nothing must not
            // veto a join.
            let live_arms = per_arm
                .iter()
                .filter(|r| r.iter().any(|&id| id != n))
                .count();
            let mut arm_count: HashMap<node::Id, usize> = HashMap::new();
            for r in &per_arm {
                for &id in r.iter().filter(|&&id| id != n) {
                    *arm_count.entry(id).or_default() += 1;
                }
            }

            // Joins: reached by every live arm AND fed via an intermediate or by
            // multiple distinct outputs of `n` (so a phi var must consolidate
            // the per-arm value). A "pure parallel sibling" - a direct successor
            // via a single output reached by every arm - is excluded and instead
            // duplicated per arm, so codegen keeps each arm body intact.
            let joins: HashSet<node::Id> = arm_count
                .iter()
                .filter(|&(_, &c)| c == live_arms && c > 1)
                .filter(|&(&id, _)| is_join(mg, n, id))
                .map(|(&id, _)| id)
                .collect();

            // Pre-allocate fresh blocks for newly-discovered joins so all arms
            // converge on the same block; inherited joins keep their block.
            let mut sub_shared = shared.clone();
            for &id in &joins {
                if !sub_shared.contains_key(&id) {
                    let conf = make_conf(meta, mg, outer_mg, id)?;
                    let join_ix = fg.add_node(Block(vec![conf]));
                    sub_shared.insert(id, join_ix);
                }
            }

            // Nodes strictly downstream of any join: lowered once in the
            // continuation pass, excluded from arm recursions.
            let downstream = strictly_downstream(mg, &joins);

            for (ix, (conns, reachable)) in branches.iter().zip(&per_arm).enumerate() {
                let arm_reachable: HashSet<_> = reachable
                    .iter()
                    .copied()
                    .filter(|id| !downstream.contains(id))
                    .collect();
                let arm_mg = reachable_subgraph(mg, &arm_reachable);
                let arm_entries = build_flow_graph(
                    meta,
                    &arm_mg,
                    outer_mg,
                    fg,
                    &sub_shared,
                    Some(n),
                    all_branches,
                )?;
                let arm_branch = Branch { ix, conns: *conns };
                for entry in arm_entries {
                    fg.add_edge(block_ix, entry, arm_branch);
                }
            }

            // Continuation: chain each newly-allocated join's downstream once.
            if !joins.is_empty() {
                let mut cont: HashSet<node::Id> = HashSet::new();
                for &j in &joins {
                    let mut bfs = petgraph::visit::Bfs::new(mg, j);
                    while let Some(d) = bfs.next(mg) {
                        cont.insert(d);
                    }
                }
                let cont_mg = reachable_subgraph(mg, &cont);
                build_flow_graph(
                    meta,
                    &cont_mg,
                    outer_mg,
                    fg,
                    &sub_shared,
                    None,
                    all_branches,
                )?;
            }

            for r in &per_arm {
                consumed.extend(r.iter().copied().filter(|&id| id != n));
            }
            last = None;
            continue;
        }

        last = Some(block_ix);
    }

    Ok(entries)
}

/// The default (non-branching) [`Branch`] label for an edge out of `src_block`.
fn default_branch(fg: &FlowGraph, src_block: NodeIndex) -> Branch {
    let conns = fg[src_block]
        .last()
        .expect("flow graph block must be non-empty")
        .conns
        .outputs;
    Branch { ix: 0, conns }
}

/// Whether `id` (reached by every live arm of branch `n`) is a true phi-join:
/// fed via an intermediate (a source other than `n`) or by more than one
/// distinct output of `n`.
fn is_join(mg: &MetaGraph, n: node::Id, id: node::Id) -> bool {
    let via_intermediate = mg
        .edges_directed(id, petgraph::Incoming)
        .any(|e_ref| e_ref.source() != n);
    if via_intermediate {
        return true;
    }
    let mut outputs: HashSet<usize> = HashSet::new();
    for e_ref in mg.edges_directed(n, petgraph::Outgoing) {
        if e_ref.target() == id {
            outputs.extend(e_ref.weight().iter().map(|(e, _)| e.output.0 as usize));
        }
    }
    outputs.len() > 1
}

/// The nodes strictly downstream of any node in `roots` (excluding the roots).
fn strictly_downstream(mg: &MetaGraph, roots: &HashSet<node::Id>) -> HashSet<node::Id> {
    let mut downstream = HashSet::new();
    for &r in roots {
        let mut bfs = petgraph::visit::Bfs::new(mg, r);
        let _ = bfs.next(mg); // skip `r` itself
        while let Some(d) = bfs.next(mg) {
            downstream.insert(d);
        }
    }
    downstream
}

/// Filter unreachable nodes from the given metagraph.
fn reachable_subgraph(g: &MetaGraph, reachable: &HashSet<node::Id>) -> MetaGraph {
    g.all_edges()
        .filter(|(a, b, _)| reachable.contains(a) && reachable.contains(b))
        .map(|(a, b, w)| (a, b, w.clone()))
        .collect()
}

/// For the given flow graph, contract all edges into basic blocks where
/// possible.
///
/// Ie for each edge, if that edge is the only output for the source node, and
/// the only input for the destination node, remove the edge and merge the src
/// and dst nodes.
fn flow_graph_edge_contraction(g: &mut FlowGraph, branching: &BTreeSet<node::Id>) {
    // Maintain a stack of all edges that require reducing.
    let mut edges: Vec<_> = g.edge_references().map(|e_ref| e_ref.id()).collect();
    while let Some(e) = edges.pop() {
        let Some((src, dst)) = g.edge_endpoints(e) else {
            continue; // Edge was removed when its node was merged.
        };

        // Never contract edges from branching nodes - even with a single
        // active branch edge, the codegen must handle branch destructuring.
        if let Some(last) = g[src].last() {
            if branching.contains(&last.id) {
                continue;
            }
        }

        // Check whether or not this is the only edge between src and dst.
        let mergeable = g.edges_directed(src, petgraph::Outgoing).take(2).count() == 1
            && g.edges_directed(dst, petgraph::Incoming).take(2).count() == 1;
        if !mergeable {
            continue;
        }

        // Merge the src and dst blocks.
        let (src_blk, dst_blk) = g.index_twice_mut(src, dst);
        src_blk.0.append(&mut dst_blk.0);

        // Re-attach the dst output edges to the src.
        let new_edges: Vec<_> = g
            .edges_directed(dst, petgraph::Outgoing)
            .map(|e_ref| (e_ref.target(), *e_ref.weight()))
            .collect();
        for (new_dst, w) in new_edges {
            let new_e = g.add_edge(src, new_dst, w);
            // Add the edges to the stack in case they need to be re-checked.
            // FIXME: Shouldn't be necessary to re-add edges to the stack if we
            // check all edges in reverse topo order? I think we are but we
            // don't explicitly assert this anywhere.
            edges.push(new_e);
        }

        // Remove the dst node now that it's been merged.
        g.remove_node(dst);
    }
}

/// Given some node within a given meta graph with an expected total number of
/// outputs, return the list of outputs that are actually connected.
fn node_outputs(
    g: &MetaGraph,
    n: node::Id,
    n_outputs: usize,
) -> Result<node::Conns, NodeOutputsError> {
    let mut outputs = node::Conns::unconnected(n_outputs).map_err(|_| TooManyConns(n_outputs))?;
    for e_ref in g.edges_directed(n, petgraph::Outgoing) {
        for (edge, _kind) in e_ref.weight() {
            let index = edge.output.0 as usize;
            outputs
                .set(index, true)
                .map_err(|_| InvalidOutputIndex { index, n_outputs })?;
        }
    }
    Ok(outputs)
}

/// All root blocks (no incoming edges) of the flow graph.
///
/// A flow graph may comprise multiple disconnected components - e.g. when
/// independent inlet→outlet chains (such as parallel inner branches) are
/// evaluated together - and each component contributes its own root.
pub(crate) fn flow_graph_roots(fg: &FlowGraph) -> Vec<NodeIndex> {
    let mut roots: Vec<_> = fg
        .node_indices()
        .filter(|&n| fg.edges_directed(n, petgraph::Incoming).next().is_none())
        .collect();
    // Sort by the block's first node id for stable output independent of block
    // allocation order.
    roots.sort_by_key(|&ix| fg[ix].first().map(|c| c.id).unwrap_or(node::Id::MAX));
    roots
}

/// The distinct sets of outlets that may be simultaneously active across every
/// combination of inner branch outcomes, computed by walking the flow graph as
/// a decision tree.
///
/// `branching` maps each branching node's id to its declared arm count. A
/// *world* assigns one arm to each branch block actually reached; the reached
/// outlets of a world are the outlets appearing in blocks reachable when each
/// branch follows only its assigned arm's edge. A declared arm with no matching
/// edge is dead - it extends reachability no further.
///
/// Because this walks the *same* flow graph the codegen lowers, the resulting
/// patterns are aligned with the code that actually sets each outlet.
pub(crate) fn outlet_patterns(
    fg: &FlowGraph,
    outlets: &BTreeSet<node::Id>,
    branching: &BTreeMap<node::Id, usize>,
) -> BTreeSet<BTreeSet<node::Id>> {
    let roots = flow_graph_roots(fg);
    let mut patterns = BTreeSet::new();
    // Each entry is a partial world: an arm assignment per branch block.
    let mut stack: Vec<BTreeMap<NodeIndex, usize>> = vec![BTreeMap::new()];
    while let Some(world) = stack.pop() {
        match reach_world(fg, &roots, outlets, branching, &world) {
            WorldReach::Complete(reached) => {
                patterns.insert(reached);
            }
            WorldReach::Frontier(block, n_arms) => {
                for arm in 0..n_arms {
                    let mut next = world.clone();
                    next.insert(block, arm);
                    stack.push(next);
                }
            }
        }
    }
    patterns
}

/// The distinct external branch masks for a flow graph reaching `outlet_ids`
/// (ascending id order), or an empty `Vec` when fewer than two distinct outlet
/// patterns are reachable (i.e. no external branching).
///
/// Maps each reached-outlet set from [`outlet_patterns`] to a `Conns` over
/// `outlet_ids`. Used both by `GraphNode`'s node-style `branches()` (inlets ->
/// outlets) and by branch-aware push-through-outlet propagation (a push source
/// inside the graph -> outlets).
pub(crate) fn branch_patterns_from_flow(
    fg: &FlowGraph,
    outlet_ids: &[node::Id],
    branching: &BTreeMap<node::Id, usize>,
) -> Result<Vec<node::Conns>, TooManyConns> {
    let n = outlet_ids.len();
    let outlets: BTreeSet<node::Id> = outlet_ids.iter().copied().collect();
    let mut patterns: BTreeSet<node::Conns> = BTreeSet::new();
    for reached in outlet_patterns(fg, &outlets, branching) {
        let mut conns = node::Conns::unconnected(n).map_err(|_| TooManyConns(n))?;
        for (i, id) in outlet_ids.iter().enumerate() {
            if reached.contains(id) {
                conns.set(i, true).map_err(|_| TooManyConns(n))?;
            }
        }
        patterns.insert(conns);
    }
    if patterns.len() < 2 {
        return Ok(vec![]);
    }
    Ok(patterns.into_iter().collect())
}

/// Outcome of walking one (partial) world in [`outlet_patterns`].
enum WorldReach {
    /// Every reached branch block was assigned; these outlets were reached.
    Complete(BTreeSet<node::Id>),
    /// A reached branch block (with the given arm count) was unassigned.
    Frontier(NodeIndex, usize),
}

/// BFS the flow graph following only assigned branch arms. Returns the
/// deterministic-min unassigned reached branch block, or - when every reached
/// branch is assigned - the reached outlets.
fn reach_world(
    fg: &FlowGraph,
    roots: &[NodeIndex],
    outlets: &BTreeSet<node::Id>,
    branching: &BTreeMap<node::Id, usize>,
    world: &BTreeMap<NodeIndex, usize>,
) -> WorldReach {
    let mut visited: HashSet<NodeIndex> = HashSet::new();
    let mut queue: VecDeque<NodeIndex> = roots.iter().copied().collect();
    let mut reached: BTreeSet<node::Id> = BTreeSet::new();
    let mut frontier: Option<NodeIndex> = None;
    while let Some(blk) = queue.pop_front() {
        if !visited.insert(blk) {
            continue;
        }
        let block = &fg[blk];
        for conf in block.iter() {
            if outlets.contains(&conf.id) {
                reached.insert(conf.id);
            }
        }
        let last = block.last().expect("flow block must not be empty");
        if branching.contains_key(&last.id) {
            // A branch block only proceeds along its assigned arm's edge.
            match world.get(&blk) {
                None => frontier = Some(frontier.map_or(blk, |f| f.min(blk))),
                Some(&arm) => {
                    for e_ref in fg.edges_directed(blk, petgraph::Outgoing) {
                        if e_ref.weight().ix == arm {
                            queue.push_back(e_ref.target());
                        }
                    }
                }
            }
        } else {
            for e_ref in fg.edges_directed(blk, petgraph::Outgoing) {
                queue.push_back(e_ref.target());
            }
        }
    }
    match frontier {
        Some(blk) => {
            let id = fg[blk].last().expect("non-empty block").id;
            WorldReach::Frontier(blk, branching[&id])
        }
        None => WorldReach::Complete(reached),
    }
}
