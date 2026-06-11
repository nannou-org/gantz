//! Items related to generating steel code from a gantz graph, primarily the
//! [`module`] fn.

use crate::{
    Edge,
    node::{self, Node},
};
#[doc(inline)]
pub use codegen::{OutletActivity, entry_fn_body, entry_fn_name};
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
mod emit;
pub mod entrypoint;
pub mod error;
mod flow;
mod ir;
mod lower;
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
    G::NodeId: Eq + Hash,
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
    Topo::new(g).iter(g).filter(move |n| reachable.contains(&n))
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
    let ep_ids: std::collections::BTreeSet<_> =
        ep_push.keys().chain(ep_pull.keys()).cloned().collect();
    let outlet_ids: Vec<node::Id> = meta_tree.elem.outlets.iter().copied().collect();
    for ep_id in ep_ids {
        let push = ep_push.remove(&ep_id).unwrap_or_default();
        let pull = ep_pull.remove(&ep_id).unwrap_or_default();
        let extra = extra_branches.remove(&ep_id).unwrap_or_default();
        let fg = flow::flow_graph_with_extra(&meta_tree.elem, push, pull, &extra)?;
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

/// Like [`module`], but lowering through the join-point IR pipeline
/// (`lower` -> `ir` -> `emit`) instead of the flow-graph codegen.
///
/// Nested graphs compile *call-based*: each level becomes one
/// `graph-fn-{path}-i{mask}` per active-inlet variant, which parents call
/// like a node fn (so `GraphNode::expr`'s re-derivation is bypassed). An
/// entrypoint sourced inside a nested graph becomes a per-level fn whose
/// result - `(list value state')`, with `value` a `(list branch-ix vals)`
/// pair when the push reaches the outlets through branching - the parent
/// level continues from (push-through-outlet as an ordinary value return
/// instead of the flow pipeline's outlet bridges).
pub fn module_v2<'a, G>(
    get_node: node::GetNode<'a>,
    g: G,
    entrypoints: &[Entrypoint],
) -> Result<Vec<ExprKind>, ModuleError>
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    let mut meta_tree = MetaTree::default();
    crate::graph::visit(get_node, g, &[], &mut meta_tree);
    if !meta_tree.errors.is_empty() {
        return Err(error::MetaErrors(meta_tree.errors).into());
    }
    let meta_tree = meta_tree.tree;

    // Reuse the flow analyses for outlet reach: per (level, entrypoint), the
    // outlets the evaluation reaches and the distinct activation patterns.
    let level_sources = group_sources_by_level(entrypoints);
    let flow_tree = build_flow_tree(&meta_tree, &level_sources, vec![])?;

    let mut v2 = V2 {
        meta_tree: &meta_tree,
        level_sources: &level_sources,
        confs: std::collections::BTreeMap::new(),
        graph_fns: Vec::new(),
        fns: Vec::new(),
    };

    // One entry fn per entrypoint; the recursive level walk emits any nested
    // level fns and yields the root statements inline.
    let ep_ids: std::collections::BTreeSet<EntrypointId> =
        entrypoints.iter().map(|ep| ep.id()).collect();
    for ep_id in &ep_ids {
        let mut stmts = vec![emit::l([
            emit::a("define"),
            emit::a(crate::GRAPH_STATE),
            emit::a(crate::ROOT_STATE),
        ])];
        if let Some((level_stmts, _)) = v2.ep_level_stmts(ep_id, &meta_tree, &flow_tree, &[])? {
            stmts.extend(level_stmts);
        }
        stmts.push(emit::l([
            emit::a("set!"),
            emit::a(crate::ROOT_STATE),
            emit::a(crate::GRAPH_STATE),
        ]));
        v2.fns
            .push(emit::fn_def(&codegen::entry_fn_name(ep_id), &[], stmts));
    }

    // Generate a graph fn per (nested level, active-inlet variant) reachable
    // from the lowered bodies, to a fixpoint (graph fns may call deeper
    // variants).
    let mut emitted: std::collections::BTreeSet<(Vec<node::Id>, node::Conns)> =
        std::collections::BTreeSet::new();
    loop {
        let mut queue = Vec::new();
        for (path, confs) in &v2.confs {
            let sub = v2.meta_tree.tree(path).expect("conf path exists");
            for conf in confs {
                if sub.nested.contains_key(&conf.id) {
                    let mut gpath = path.clone();
                    gpath.push(conf.id);
                    let key = (gpath, conf.conns.inputs);
                    if !emitted.contains(&key) {
                        queue.push(key);
                    }
                }
            }
        }
        if queue.is_empty() {
            break;
        }
        for (gpath, imask) in queue {
            if emitted.insert((gpath.clone(), imask)) {
                v2.graph_fn(&gpath, &imask)?;
            }
        }
    }

    // A node fn per non-graph variant. The conf tree mirrors the full meta
    // tree so the generating visitor finds every level; nested-graph confs
    // are excluded (they are graph fns, not node fns).
    let confs_tree = v2_conf_tree(&meta_tree, &v2.confs, &mut Vec::new());
    let node_fns = codegen::node_fns(get_node, g, &confs_tree)?;

    // Definition order: node fns, graph fns deepest-first, then level and
    // entry fns (callers always after their callees).
    v2.graph_fns.sort_by(|(a, _), (b, _)| b.cmp(a));
    Ok(node_fns
        .into_iter()
        .chain(v2.graph_fns.into_iter().map(|(_, f)| f))
        .chain(v2.fns)
        .collect())
}

/// A nested level's contribution to its parent's evaluation of one
/// entrypoint.
struct LevelPiece {
    /// The emitted level fn's name.
    fn_name: String,
    /// Whether the level's interior holds any state (the fn then threads the
    /// level's state hashmap in and out).
    stateful: bool,
    /// `Some` when the level's evaluation reaches its outlets: the distinct
    /// activation patterns (>= 2 = the parent dispatches per pattern).
    patterns: Option<Vec<node::Conns>>,
}

/// The working state of one `module_v2` invocation.
struct V2<'a> {
    meta_tree: &'a RoseTree<Meta>,
    level_sources:
        &'a std::collections::BTreeMap<Vec<node::Id>, Vec<(EntrypointId, Vec<&'a EvalSource>)>>,
    /// Node variants called by the lowered bodies, per level path.
    confs: std::collections::BTreeMap<Vec<node::Id>, std::collections::BTreeSet<NodeConf>>,
    /// Emitted graph fns with their level depth. A graph fn calls only
    /// deeper graph fns, so they are defined deepest-first, before all level
    /// and entry fns (Steel resolves free identifiers at definition).
    graph_fns: Vec<(usize, ExprKind)>,
    /// Emitted level and entry fns, child levels before their parents.
    fns: Vec<ExprKind>,
}

impl V2<'_> {
    /// Build the statements evaluating `ep` at the level `path`: calls to
    /// child level fns (with state threading and result binding) followed by
    /// this level's own lowered body. Returns `None` when neither this level
    /// nor any descendant has sources for `ep`.
    fn ep_level_stmts(
        &mut self,
        ep: &EntrypointId,
        meta_node: &RoseTree<Meta>,
        flow_node: &RoseTree<(&Meta, Flow)>,
        path: &[node::Id],
    ) -> Result<Option<(Vec<emit::Sexp>, Vec<lower::OutletVal>)>, ModuleError> {
        use emit::{Sexp, a, l};

        let meta = &meta_node.elem;
        let mut glue: Vec<Sexp> = Vec::new();
        let mut push: Vec<(node::Id, node::Conns)> = Vec::new();
        let mut pull: Vec<(node::Id, node::Conns)> = Vec::new();
        let mut extra_branches = std::collections::BTreeMap::new();
        let mut prebound = std::collections::BTreeSet::new();
        let mut prebound_vars: Vec<ir::Var> = Vec::new();
        let mut any_child = false;

        // Children first (post-order): each contributing child evaluates via
        // its level fn; one that reaches its outlets becomes a pre-bound
        // source this level continues from.
        for (&gid, sub_meta) in &meta_node.nested {
            let sub_flow = &flow_node.nested[&gid];
            let mut child_path = path.to_vec();
            child_path.push(gid);
            let Some(piece) = self.ep_level(ep, sub_meta, sub_flow, &child_path)? else {
                continue;
            };
            any_child = true;
            let key = a(format!("'{gid}"));
            let pair = a(format!("node-{gid}"));
            if piece.stateful {
                let r = format!("%lvl-r-{gid}");
                glue.push(l([
                    a("define"),
                    a(r.clone()),
                    l([
                        a(piece.fn_name),
                        l([a("hash-ref"), a(crate::GRAPH_STATE), key.clone()]),
                    ]),
                ]));
                glue.push(l([
                    a("set!"),
                    a(crate::GRAPH_STATE),
                    l([
                        a("hash-insert"),
                        a(crate::GRAPH_STATE),
                        key,
                        l([a("list-ref"), a(r.clone()), a("1")]),
                    ]),
                ]));
                if piece.patterns.is_some() {
                    glue.push(l([
                        a("define"),
                        pair.clone(),
                        l([a("list-ref"), a(r), a("0")]),
                    ]));
                }
            } else if piece.patterns.is_some() {
                glue.push(l([a("define"), pair.clone(), l([a(piece.fn_name)])]));
            } else {
                // Stateless, no outlets reached: evaluate for side effects.
                glue.push(l([a(piece.fn_name)]));
            }

            let Some(patterns) = piece.patterns else {
                continue;
            };
            let n_out = meta.outputs.get(&gid).copied().unwrap_or(0);
            prebound.insert(gid);
            if patterns.len() >= 2 {
                prebound_vars.push(ir::Var::Result { node: gid });
                push.push((gid, or_patterns(&patterns)?));
                extra_branches.insert(gid, patterns);
            } else {
                // Destructure all outputs from the plain result.
                match n_out {
                    0 => {}
                    1 => glue.push(l([a("define"), a(format!("node-{gid}-o0")), pair])),
                    _ => glue.push(l([
                        a("define-values"),
                        l((0..n_out).map(|o| a(format!("node-{gid}-o{o}")))),
                        pair,
                    ])),
                }
                for o in 0..n_out {
                    prebound_vars.push(ir::Var::Output {
                        node: gid,
                        output: o,
                    });
                }
                let conns = node::Conns::connected(n_out)
                    .map_err(|_| error::NodeConnsError::from(error::TooManyConns(n_out)))?;
                push.push((gid, conns));
            }
        }

        // This level's direct sources.
        for (id, srcs) in self.level_sources.get(path).map(|v| &v[..]).unwrap_or(&[]) {
            if id != ep {
                continue;
            }
            for src in srcs {
                let node_id = *src.path.last().unwrap();
                match src.kind {
                    EvalKind::Push => push.push((node_id, src.conns)),
                    EvalKind::Pull => pull.push((node_id, src.conns)),
                }
            }
        }

        if !any_child && push.is_empty() && pull.is_empty() {
            return Ok(None);
        }

        let cx = lower::Cx {
            meta,
            nested: meta_node.nested.keys().copied().collect(),
            extra_branches,
            prebound,
        };
        let out = lower::level_body(&cx, &lower::LevelSources::Eval { push, pull })?;
        #[cfg(debug_assertions)]
        ir::validate(&out.body, 0, &prebound_vars).expect("lowering produced invalid IR");
        lower::collect_confs(&out.body, self.confs.entry(path.to_vec()).or_default());
        let ecx = emit::Cx { path };
        glue.extend(emit::body_sexps(&ecx, &out.body));
        Ok(Some((glue, out.outlets)))
    }

    /// Build and emit the level fn evaluating `ep` at the nested level
    /// `path`, returning how the parent integrates it.
    fn ep_level(
        &mut self,
        ep: &EntrypointId,
        meta_node: &RoseTree<Meta>,
        flow_node: &RoseTree<(&Meta, Flow)>,
        path: &[node::Id],
    ) -> Result<Option<LevelPiece>, ModuleError> {
        use emit::{a, l};
        let Some((stmts, outlets)) = self.ep_level_stmts(ep, meta_node, flow_node, path)? else {
            return Ok(None);
        };
        let (_, flow) = &flow_node.elem;
        let reach = flow.outlet_reach.get(ep);
        let result = match reach {
            Some(r) => emit::level_result_sexp(&outlets, &r.patterns),
            None => emit::unit(),
        };
        let stateful = !meta_node.elem.stateful.is_empty();
        let fn_name = format!(
            "lvl-fn-{}-{}",
            ep.0.display_short(),
            codegen::path_string(path)
        );
        let mut body = Vec::with_capacity(stmts.len() + 2);
        let params: Vec<String> = if stateful {
            body.push(l([a("define"), a(crate::GRAPH_STATE), a("%lvl-state")]));
            vec!["%lvl-state".to_string()]
        } else {
            vec![]
        };
        body.extend(stmts);
        if stateful {
            body.push(l([a("list"), result, a(crate::GRAPH_STATE)]));
        } else {
            body.push(result);
        }
        self.fns.push(emit::fn_def(&fn_name, &params, body));
        Ok(Some(LevelPiece {
            fn_name,
            stateful,
            patterns: reach.map(|r| r.patterns.clone()),
        }))
    }

    /// Build and emit the graph fn for the nested level at `gpath`, for the
    /// variant invoked with the given active-input mask.
    fn graph_fn(&mut self, gpath: &[node::Id], imask: &node::Conns) -> Result<(), ModuleError> {
        use emit::{a, l};
        let sub = self.meta_tree.tree(gpath).expect("nested level exists");
        let meta = &sub.elem;
        let inlet_ids: Vec<node::Id> = meta.inlets.iter().copied().collect();
        let active = active_inlets_from_conns(&inlet_ids, imask);
        let cx = lower::Cx {
            meta,
            nested: sub.nested.keys().copied().collect(),
            extra_branches: std::collections::BTreeMap::new(),
            prebound: std::collections::BTreeSet::new(),
        };
        let out = lower::level_body(&cx, &lower::LevelSources::Inlets(active.clone()))?;
        #[cfg(debug_assertions)]
        {
            let inlet_vars: Vec<ir::Var> = active
                .iter()
                .map(|&i| ir::Var::Output { node: i, output: 0 })
                .collect();
            ir::validate(&out.body, 0, &inlet_vars).expect("lowering produced invalid IR");
        }
        lower::collect_confs(&out.body, self.confs.entry(gpath.to_vec()).or_default());

        // The node-contract patterns: the all-active analysis, matching the
        // order `Graph::branches` reports to the parent.
        let patterns = if meta.branches.is_empty() {
            vec![]
        } else {
            let outlet_ids: Vec<node::Id> = meta.outlets.iter().copied().collect();
            let all: std::collections::BTreeSet<node::Id> = meta.inlets.iter().copied().collect();
            let fg = inner_flow_graph_for(meta, &all)?;
            let arm_counts = meta.branches.iter().map(|(&i, v)| (i, v.len())).collect();
            branch_patterns_from_flow(&fg, &outlet_ids, &arm_counts)
                .map_err(error::NodeConnsError::from)?
        };
        let result = emit::level_result_sexp(&out.outlets, &patterns);
        let stateful = !meta.stateful.is_empty();
        let ecx = emit::Cx { path: gpath };
        let mut stmts = Vec::new();
        let mut params: Vec<String> = active.iter().map(|&i| format!("node-{i}-o0")).collect();
        if stateful {
            params.push("%lvl-state".to_string());
            stmts.push(l([a("define"), a(crate::GRAPH_STATE), a("%lvl-state")]));
        }
        stmts.extend(emit::body_sexps(&ecx, &out.body));
        if stateful {
            stmts.push(l([a("list"), result, a(crate::GRAPH_STATE)]));
        } else {
            stmts.push(result);
        }
        let fn_def = emit::fn_def(&emit::graph_fn_name(gpath, imask), &params, stmts);
        self.graph_fns.push((gpath.len(), fn_def));
        Ok(())
    }
}

/// Build the node-fn conf tree mirroring the meta tree, excluding
/// nested-graph node confs (those compile to graph fns).
fn v2_conf_tree(
    meta_node: &RoseTree<Meta>,
    confs: &std::collections::BTreeMap<Vec<node::Id>, std::collections::BTreeSet<NodeConf>>,
    path: &mut Vec<node::Id>,
) -> RoseTree<std::collections::BTreeSet<NodeConf>> {
    let elem = confs
        .get(path)
        .map(|set| {
            set.iter()
                .filter(|c| !meta_node.nested.contains_key(&c.id))
                .copied()
                .collect()
        })
        .unwrap_or_default();
    let nested = meta_node
        .nested
        .iter()
        .map(|(&id, sub)| {
            path.push(id);
            let t = v2_conf_tree(sub, confs, path);
            path.pop();
            (id, t)
        })
        .collect();
    RoseTree { elem, nested }
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
        let fg = inner_flow_graph_for(meta, &active_inlets)?;
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
