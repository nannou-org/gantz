//! Items related to generating steel code from a gantz graph, primarily the
//! [`module`] fn.

use crate::{
    Edge,
    node::{self, Node},
};
pub(crate) use codegen::entry_fn_body;
#[doc(inline)]
pub use codegen::{OutletActivity, entry_fn_name};
pub(crate) use codegen::{branch_selector, outlet_values_expr};
#[doc(inline)]
pub use entrypoint::{
    Entrypoint, EntrypointId, EvalKind, EvalSource, pull_source, push_pull_entrypoints, push_source,
};
#[doc(inline)]
pub use error::ModuleError;
#[doc(inline)]
pub use flow::{Block, Flow, FlowGraph, NodeConf, NodeConns, OutletReach, flow_graph};
pub(crate) use flow::{branch_patterns_from_flow, flow_graph_roots, inner_flow_graph_for};
pub(crate) use loops::LoopTable;
use meta::MetaTree;
#[doc(inline)]
pub use meta::{EdgeKind, Meta, MetaGraph};
use petgraph::visit::{
    Data, Dfs, EdgeRef, GraphBase, GraphRef, IntoEdgesDirected, IntoNeighbors, IntoNodeReferences,
    NodeIndexable, Topo, Visitable, Walker,
};
pub(crate) use rosetree::RoseTree;
use std::{collections::HashSet, hash::Hash};
use steel::parser::ast::ExprKind;

mod codegen;
pub mod entrypoint;
pub mod error;
mod flow;
mod loops;
mod meta;
mod rosetree;

/// Allows for remaining generic over graph type edges that represent one or
/// more gantz [`Edge`]s.
///
/// This is necessary to support calling the `reachable` fns with both the
/// `Meta` graph and graphs of different types.
pub trait Edges {
    /// Produce an iterator yielding all the [`Edge`]s.
    fn edges(&self) -> impl Iterator<Item = Edge>;
}

impl Edges for Edge {
    fn edges(&self) -> impl Iterator<Item = Edge> {
        std::iter::once(*self)
    }
}

impl Edges for Vec<Edge> {
    fn edges(&self) -> impl Iterator<Item = Edge> {
        self.iter().copied()
    }
}

/// Given a graph and an eval src, return the set of direct neighbors that
/// will be included in the initial traversal.
fn eval_neighbors<G>(
    g: G,
    n: G::NodeId,
    conns: &node::Conns,
    src_conn: impl Fn(&Edge) -> usize,
) -> HashSet<G::NodeId>
where
    G: IntoEdgesDirected,
    G::EdgeWeight: Edges,
    G::NodeId: Eq + Hash,
{
    let mut set = HashSet::new();
    for e_ref in g.edges_directed(n, petgraph::Outgoing) {
        for edge in e_ref.weight().edges() {
            let conn_ix = src_conn(&edge);
            let include = conns.get(conn_ix).unwrap();
            if include {
                set.insert(e_ref.target());
            }
        }
    }
    set
}

/// Given a graph and a `EvalConf` src, return the set of direct neighbors that
/// will be included in the initial traversal.
fn push_eval_neighbors<G>(g: G, n: G::NodeId, ev: &node::Conns) -> HashSet<G::NodeId>
where
    G: IntoEdgesDirected,
    G::EdgeWeight: Edges,
    G::NodeId: Eq + Hash,
{
    eval_neighbors(g, n, ev, |edge| edge.output.0 as usize)
}

/// Given a graph and a `EvalConf` src, return the set of direct neighbors that
/// will be included in the initial traversal.
fn pull_eval_neighbors<G>(g: G, n: G::NodeId, ev: &node::Conns) -> HashSet<G::NodeId>
where
    G: IntoEdgesDirected,
    G::EdgeWeight: Edges,
    G::NodeId: Eq + Hash,
{
    let rev_g = petgraph::visit::Reversed(g);
    eval_neighbors(rev_g, n, ev, |edge| edge.input.0 as usize)
}

/// An iterator yielding all nodes reachable via pushing from the given node
/// over the given set of neighbors.
fn reachable<G>(
    g: G,
    src: G::NodeId,
    src_neighbors: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
    G::NodeId: Eq + Hash,
{
    /// A filter around a graph's `IntoNeighbors` implementation that discludes
    /// edges from the root that are not in the given src neighbors set.
    #[derive(Clone, Copy)]
    struct EvalFilter<'a, G: GraphBase> {
        g: G,
        /// The node that is the source of evaluation.
        src: G::NodeId,
        /// The eval src node's neighbors that are included in the eval set.
        src_neighbors: &'a HashSet<G::NodeId>,
    }

    /// The iterator filter applied to the inner graph's neighbors iterator.
    /// If we're iterating over the eval src's neighbors, this will only yield
    /// those specified in the included set.
    struct EvalFilterNeighbors<'a, I: Iterator> {
        neighbors: I,
        // Whether or not `neighbors` are from the eval src.
        is_src: bool,
        /// The eval src node's neighbors that are included in the eval set.
        src_neighbors: &'a HashSet<I::Item>,
    }

    impl<'a, G> GraphBase for EvalFilter<'a, G>
    where
        G: GraphBase,
    {
        type NodeId = G::NodeId;
        type EdgeId = G::EdgeId;
    }

    impl<'a, G: GraphRef> GraphRef for EvalFilter<'a, G> {}

    impl<'a, G> Visitable for EvalFilter<'a, G>
    where
        G: GraphRef + Visitable,
    {
        type Map = G::Map;
        fn visit_map(&self) -> Self::Map {
            self.g.visit_map()
        }
        fn reset_map(&self, map: &mut Self::Map) {
            self.g.reset_map(map);
        }
    }

    impl<'a, I> Iterator for EvalFilterNeighbors<'a, I>
    where
        I: Iterator,
        I::Item: Eq + Hash,
    {
        type Item = I::Item;
        fn next(&mut self) -> Option<Self::Item> {
            while let Some(n) = self.neighbors.next() {
                if !self.is_src || self.src_neighbors.contains(&n) {
                    return Some(n);
                }
            }
            None
        }
    }

    impl<'a, G> IntoNeighbors for EvalFilter<'a, G>
    where
        G: IntoNeighbors,
        G::NodeId: Eq + Hash,
    {
        type Neighbors = EvalFilterNeighbors<'a, G::Neighbors>;
        fn neighbors(self, a: Self::NodeId) -> Self::Neighbors {
            let neighbors = self.g.neighbors(a);
            EvalFilterNeighbors {
                neighbors,
                is_src: self.src == a,
                src_neighbors: &self.src_neighbors,
            }
        }
    }

    // Wrap the graph in a filter to only include src neighbors that appear in
    // the eval set.
    let g = EvalFilter {
        g,
        src,
        src_neighbors,
    };

    Dfs::new(g, src).iter(g)
}

/// An iterator yielding all nodes reachable via pushing from the given node
/// over the given set of direct neighbors.
fn push_reachable<G>(
    g: G,
    n: G::NodeId,
    nbs: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
    G::NodeId: Eq + Hash,
{
    reachable(g, n, nbs)
}

/// An iterator yielding all nodes reachable via pulling from the given node
/// over the given set of direct neighbors.
fn pull_reachable<G>(
    g: G,
    n: G::NodeId,
    nbs: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
    G::NodeId: Eq + Hash,
{
    let rev_g = petgraph::visit::Reversed(g);
    reachable(rev_g, n, nbs)
}

/// The evaluation order given any number of simultaneously pushing and pulling
/// nodes.
///
/// Evaluation order is equivalent to a topological ordering of the connected
/// components reachable via DFS from each push node and reversed-edge DFS from
/// each pull node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes
/// implement `Node`. Direction of edges indicate the flow of data through the
/// graph.
pub(crate) fn eval_order<G, A, B>(g: G, push: A, pull: B) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + IntoNodeReferences + Visitable,
    G::EdgeWeight: Edges,
    G::NodeId: Copy + Eq + Hash + Ord,
    A: IntoIterator<Item = (G::NodeId, node::Conns)>,
    B: IntoIterator<Item = (G::NodeId, node::Conns)>,
{
    let mut reachable = HashSet::new();
    reachable.extend(push.into_iter().flat_map(|(n, conns)| {
        let ps = push_eval_neighbors(g, n, &conns);
        push_reachable(g, n, &ps).collect::<Vec<_>>()
    }));
    reachable.extend(pull.into_iter().flat_map(|(n, conns)| {
        let pl = pull_eval_neighbors(g, n, &conns);
        pull_reachable(g, n, &pl).collect::<Vec<_>>()
    }));
    // Topologically order the acyclic part, then append any reachable nodes on a
    // directed cycle (which `Topo` never yields) in node-id order. The sole
    // caller collects this into a set, so order only affects determinism; cyclic
    // nodes must still be *included* so they reach the flow graph.
    let topo_order: Vec<G::NodeId> = Topo::new(g)
        .iter(g)
        .filter(|n| reachable.contains(n))
        .collect();
    let in_topo: HashSet<G::NodeId> = topo_order.iter().copied().collect();
    let mut leftover: Vec<G::NodeId> = reachable
        .into_iter()
        .filter(|n| !in_topo.contains(n))
        .collect();
    leftover.sort();
    topo_order.into_iter().chain(leftover)
}

/// Group entrypoint sources by graph level (parent path).
///
/// Returns a map from level_path -> Vec<(EntrypointId, sources_at_this_level)>.
/// A single entrypoint with cross-level sources appears at multiple levels.
fn group_sources_by_level(
    entrypoints: &[Entrypoint],
) -> std::collections::BTreeMap<Vec<node::Id>, Vec<(EntrypointId, Vec<&EvalSource>)>> {
    let mut map: std::collections::BTreeMap<
        Vec<node::Id>,
        std::collections::BTreeMap<EntrypointId, Vec<&EvalSource>>,
    > = std::collections::BTreeMap::new();
    for ep in entrypoints {
        let id = ep.id();
        for src in &ep.0 {
            let parent = src.path[..src.path.len() - 1].to_vec();
            map.entry(parent)
                .or_default()
                .entry(id.clone())
                .or_default()
                .push(src);
        }
    }
    map.into_iter()
        .map(|(level, ep_map)| (level, ep_map.into_iter().collect()))
        .collect()
}

/// Bitwise-OR of the branch patterns (all of equal width): the set of outputs a
/// branching push-through can produce across all its branch outcomes.
fn or_patterns(patterns: &[node::Conns]) -> Result<node::Conns, error::NodeConnsError> {
    let n = patterns.first().map_or(0, node::Conns::len);
    let bits: Vec<bool> = (0..n)
        .map(|i| patterns.iter().any(|p| p.get(i).unwrap_or(false)))
        .collect();
    node::Conns::try_from_slice(&bits).map_err(|_| error::TooManyConns(n).into())
}

/// Recursively build a flow tree, routing entrypoint sources to each nesting
/// level.
///
/// Children are recursed into first so that outlet-reaching push sources can
/// be combined with direct sources before building each entrypoint's flow
/// graph. This ensures each flow graph is built exactly once with complete
/// source information.
fn build_flow_tree<'a>(
    meta_tree: &'a RoseTree<Meta>,
    level_sources: &std::collections::BTreeMap<
        Vec<node::Id>,
        Vec<(EntrypointId, Vec<&EvalSource>)>,
    >,
    current_path: Vec<node::Id>,
) -> Result<RoseTree<(&'a Meta, Flow)>, error::NodeConnsError> {
    let sources = level_sources
        .get(&current_path)
        .map(|v| &v[..])
        .unwrap_or(&[]);
    let nested_fg = flow::flow_graph(
        &meta_tree.elem,
        meta_tree
            .elem
            .inlets
            .iter()
            .map(|&n| (n, node::Conns::connected(1).unwrap())),
        meta_tree
            .elem
            .outlets
            .iter()
            .map(|&n| (n, node::Conns::connected(1).unwrap())),
    )?;
    let mut nested = std::collections::BTreeMap::new();

    // 1. Recurse into children, collect outlet-reaching push sources. When a
    //    child's push reaches its outlets through branching (>= 2 distinct outlet
    //    patterns), record those patterns as the bridged graph node's branches
    //    for this entrypoint, so the parent gates its continuation per branch.
    let mut outlet_push: std::collections::BTreeMap<EntrypointId, Vec<(node::Id, node::Conns)>> =
        std::collections::BTreeMap::new();
    let mut extra_branches: std::collections::BTreeMap<
        EntrypointId,
        std::collections::BTreeMap<node::Id, Vec<node::Conns>>,
    > = std::collections::BTreeMap::new();
    for (&id, subtree) in &meta_tree.nested {
        let mut child_path = current_path.clone();
        child_path.push(id);
        let child_tree = build_flow_tree(subtree, level_sources, child_path)?;
        let (_, ref child_flow) = child_tree.elem;
        let n_outputs = meta_tree.elem.outputs.get(&id).copied().unwrap_or(0);
        if n_outputs > 0 {
            for (ep_id, reach) in &child_flow.outlet_reach {
                let conns = if reach.patterns.len() >= 2 {
                    extra_branches
                        .entry(ep_id.clone())
                        .or_default()
                        .insert(id, reach.patterns.clone());
                    // Only the outputs some branch can produce reach downstream.
                    or_patterns(&reach.patterns)?
                } else {
                    node::Conns::connected(n_outputs).map_err(|_| error::TooManyConns(n_outputs))?
                };
                outlet_push
                    .entry(ep_id.clone())
                    .or_default()
                    .push((id, conns));
            }
        }
        nested.insert(id, child_tree);
    }

    // 2. Collect all push/pull sources per entrypoint: direct + outlet.
    let mut ep_push: std::collections::BTreeMap<EntrypointId, Vec<(node::Id, node::Conns)>> =
        std::collections::BTreeMap::new();
    let mut ep_pull: std::collections::BTreeMap<EntrypointId, Vec<(node::Id, node::Conns)>> =
        std::collections::BTreeMap::new();
    for (ep_id, srcs) in sources {
        for src in srcs {
            let node_id = *src.path.last().unwrap();
            let map = match src.kind {
                EvalKind::Push => &mut ep_push,
                EvalKind::Pull => &mut ep_pull,
            };
            map.entry(ep_id.clone())
                .or_default()
                .push((node_id, src.conns));
        }
    }
    for (ep_id, srcs) in outlet_push {
        ep_push.entry(ep_id).or_default().extend(srcs);
    }

    // 3. Build one flow graph per entrypoint with complete sources, treating any
    //    branch-aware bridged graph nodes as branch nodes for that entrypoint.
    let mut entrypoints = std::collections::BTreeMap::new();
    let mut outlet_reach = std::collections::BTreeMap::new();
    let mut loops = std::collections::BTreeMap::new();
    let ep_ids: std::collections::BTreeSet<_> =
        ep_push.keys().chain(ep_pull.keys()).cloned().collect();
    let outlet_ids: Vec<node::Id> = meta_tree.elem.outlets.iter().copied().collect();
    for ep_id in ep_ids {
        let push = ep_push.remove(&ep_id).unwrap_or_default();
        let pull = ep_pull.remove(&ep_id).unwrap_or_default();
        let extra = extra_branches.remove(&ep_id).unwrap_or_default();
        let (fg, ep_loops) = flow::flow_graph_with_extra(&meta_tree.elem, push, pull, &extra)?;
        loops.insert(ep_id.clone(), ep_loops);
        let reached: std::collections::BTreeSet<node::Id> = fg
            .node_weights()
            .flat_map(|blk| blk.iter())
            .map(|conf| conf.id)
            .filter(|nid| meta_tree.elem.outlets.contains(nid))
            .collect();
        if !reached.is_empty() {
            // Patterns of this graph's outlets reachable from this entrypoint -
            // for the parent's branch-aware propagation. Account for this graph's
            // own branches plus this entrypoint's push-through (extra) branches.
            let mut arm_counts: std::collections::BTreeMap<node::Id, usize> = meta_tree
                .elem
                .branches
                .iter()
                .map(|(&i, v)| (i, v.len()))
                .collect();
            arm_counts.extend(extra.iter().map(|(&i, m)| (i, m.len())));
            let patterns = flow::branch_patterns_from_flow(&fg, &outlet_ids, &arm_counts)?;
            outlet_reach.insert(ep_id.clone(), flow::OutletReach { reached, patterns });
        }
        entrypoints.insert(ep_id, fg);
    }

    let flow = Flow {
        nested: nested_fg,
        entrypoints,
        outlet_reach,
        loops,
    };
    Ok(RoseTree {
        elem: (&meta_tree.elem, flow),
        nested,
    })
}

/// Given a root gantz graph, generate the full module with all the necessary
/// functions for executing it.
///
/// This includes:
///
/// 1. A function for each node (and for each node input configuration).
/// 2. A function for each entrypoint's evaluation.
/// 3. The above for all nested graphs.
pub fn module<'a, G>(
    get_node: node::GetNode<'a>,
    g: G,
    entrypoints: &[Entrypoint],
) -> Result<Vec<ExprKind>, ModuleError>
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    // Create a `Meta` for each graph (including nested) in a tree.
    let mut meta_tree = MetaTree::default();
    crate::graph::visit(get_node, g, &[], &mut meta_tree);
    if !meta_tree.errors.is_empty() {
        return Err(error::MetaErrors(meta_tree.errors).into());
    }

    // Group entrypoint sources by graph level, then recursively build a
    // flow tree that routes each level's sources to the correct nested graph.
    let level_sources = group_sources_by_level(entrypoints);
    let flow_tree = build_flow_tree(&meta_tree.tree, &level_sources, vec![])?;

    // Collect node fns. Build the per-level conf tree top-down so each node is
    // defined for every variant it is actually called with - including the
    // reduced inner variants a nested graph needs when invoked with a subset of
    // its inlets active.
    let node_confs_tree = build_node_confs_tree(&meta_tree.tree, &flow_tree)?;
    let node_fns = codegen::node_fns(get_node, g, &node_confs_tree)?;

    // Collect eval fns.
    let entry_fns = codegen::entry_fns(&flow_tree)?;
    Ok(node_fns.into_iter().chain(entry_fns).collect())
}

/// Build the per-level node-conf tree consumed by `node_fns`, defining every
/// variant each node is actually called with.
///
/// A level's confs come from its own flow graphs ([`codegen::unique_node_confs`]:
/// this level's entrypoint flows + all-connected `nested` flow) plus, when the
/// level is a nested graph invoked with a reduced active-input-set, the confs
/// from [`inner_flow_graph_for`] for that subset. Active-sets propagate top-down:
/// a child's are the distinct confs it takes across every flow graph that lowers
/// its parent (the parent's entrypoint + nested flows, and its reduced flows). So
/// when `node::graph::nested_expr` lowers a nested graph with a reduced active set
/// (a "cold" inlet push, or a push-through reaching only some inlets) the reduced
/// inner variants it calls are also defined here.
fn build_node_confs_tree(
    meta_tree: &RoseTree<Meta>,
    flow_tree: &RoseTree<(&Meta, Flow)>,
) -> Result<RoseTree<std::collections::BTreeSet<NodeConf>>, error::NodeConnsError> {
    // The root graph is not invoked as a node, so it has no reduced active-sets.
    build_level_confs(meta_tree, flow_tree, &std::collections::BTreeSet::new())
}

/// Recursive worker for [`build_node_confs_tree`]: build the confs for one level
/// (and its descendants) given the active-input-sets this level is invoked with.
fn build_level_confs(
    meta_tree: &RoseTree<Meta>,
    flow_tree: &RoseTree<(&Meta, Flow)>,
    active_sets: &std::collections::BTreeSet<node::Conns>,
) -> Result<RoseTree<std::collections::BTreeSet<NodeConf>>, error::NodeConnsError> {
    let meta = &meta_tree.elem;
    let (_, flow) = &flow_tree.elem;
    let inlet_ids: Vec<node::Id> = meta.inlets.iter().copied().collect();

    // This level's own flow graphs, plus a reduced flow per proper-subset active
    // set it is invoked with. Keep the reduced flows so children can read their
    // confs from them too.
    let mut confs = codegen::unique_node_confs(flow);
    let mut reduced: Vec<FlowGraph> = Vec::new();
    for active in active_sets {
        let active_inlets = active_inlets_from_conns(&inlet_ids, active);
        // All-active coincides with the `nested` flow already in `confs`.
        if active_inlets.len() == meta.inlets.len() {
            continue;
        }
        let (fg, _loops) = inner_flow_graph_for(meta, &active_inlets)?;
        confs.extend(fg.node_weights().flat_map(|blk| blk.iter().copied()));
        reduced.push(fg);
    }

    // Each child's active-sets = the distinct confs it takes across every flow
    // graph that lowers this level (entrypoints + nested + the reduced flows).
    let mut nested = std::collections::BTreeMap::new();
    for (&cid, child_meta) in &meta_tree.nested {
        let mut child_sets = active_input_sets_for(flow, cid);
        for fg in &reduced {
            child_sets.extend(confs_of(fg, cid).map(|conf| conf.conns.inputs));
        }
        let child_flow = &flow_tree.nested[&cid];
        nested.insert(cid, build_level_confs(child_meta, child_flow, &child_sets)?);
    }

    Ok(RoseTree {
        elem: confs,
        nested,
    })
}

/// The distinct active-input-sets node `id` is invoked with across `flow`'s
/// entrypoint flow graphs and its all-connected `nested` flow.
fn active_input_sets_for(flow: &Flow, id: node::Id) -> std::collections::BTreeSet<node::Conns> {
    let mut sets = std::collections::BTreeSet::new();
    for fg in flow
        .entrypoints
        .values()
        .chain(std::iter::once(&flow.nested))
    {
        sets.extend(confs_of(fg, id).map(|conf| conf.conns.inputs));
    }
    sets
}

/// The `NodeConf`s for node `id` appearing in flow graph `fg`.
fn confs_of(fg: &FlowGraph, id: node::Id) -> impl Iterator<Item = &NodeConf> {
    fg.node_weights()
        .flat_map(|blk| blk.iter())
        .filter(move |conf| conf.id == id)
}

/// The inlet ids active for a parent input-conns mask (bit `i` <=> `inlet_ids[i]`,
/// the existing "input i -> inlet i" contract in `node::graph::nested_expr`).
fn active_inlets_from_conns(
    inlet_ids: &[node::Id],
    conns: &node::Conns,
) -> std::collections::BTreeSet<node::Id> {
    inlet_ids
        .iter()
        .enumerate()
        .filter(|(i, _)| conns.get(*i).unwrap_or(false))
        .map(|(_, &id)| id)
        .collect()
}
