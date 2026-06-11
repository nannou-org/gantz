//! Items related to generating steel code from a gantz graph, primarily the
//! [`module`] fn.
//!
//! ## Pipeline
//!
//! 1. **Meta** (`meta`): a visitor walk collects one [`Meta`] per graph level
//!    (adjacency, arities, branch masks, stateful/inlet/outlet/delay
//!    sets) into a rose tree.
//! 2. **Lowering** (`lower`): each level lowers once per use - per
//!    active-inlet variant for graph fns, per entrypoint for level bodies -
//!    to an IR (`ir`) of node-call steps, branch dispatches and join points
//!    (local fns whose parameters carry reconverging values; values consumed
//!    outside a branch construct ride its exports). `ir::validate` checks
//!    the IR's scoping/arity invariants on every lowering (on by default,
//!    see [`Config::validate_ir`]); a violation is a compiler bug and
//!    surfaces as [`ModuleError::InvalidIr`].
//! 3. **Outlet-activation analysis** (`analysis`): the distinct sets of a
//!    level's outlets that can fire together, computed by abstract
//!    interpretation of the lowered IR itself (forking per branch arm), so
//!    the patterns cannot drift from the emitted code. Backs
//!    `Graph::branches` and push-through-outlet propagation.
//! 4. **Emission** (`emit`): a mechanical IR -> Steel walk restricted to
//!    primitive base-engine forms, plus the per-variant node fns
//!    (`codegen::node_fn`). Definition order is semantic (Steel resolves
//!    free identifiers at definition): deepest levels first, a level's node
//!    fns before its graph fns, entry fns last.
//!
//! Nested graphs compile *call-based*: one `graph-fn-{path}-i{mask}` per
//! active-inlet variant, called like a node fn. An entrypoint sourced inside
//! a nested graph becomes a per-level fn threading the level's state and
//! returning the outlet result - a `(list branch-ix vals)` pair when the
//! push reaches the outlets through branching - which the parent level
//! continues from as a pre-bound source.
//!
//! Cycles are legal when they pass through a [`node::Delay`]: ordering and
//! reachability never propagate through a delay, whose value crosses
//! *between* evaluations (read at the top of a level body, written where its
//! input is produced).

use crate::{
    Edge,
    node::{self, Node},
};
pub(crate) use analysis::level_branch_patterns;
#[doc(inline)]
pub use entrypoint::{
    Entrypoint, EntrypointId, EvalKind, EvalSource, pull_source, push_pull_entrypoints, push_source,
};
#[doc(inline)]
pub use error::ModuleError;
pub(crate) use lower::{NodeConf, NodeConns};
use meta::MetaTree;
#[doc(inline)]
pub use names::{Name, entry_fn_name};
pub(crate) use names::graph_fn_name;
#[doc(inline)]
pub use source_map::SourceMap;
#[doc(inline)]
pub use meta::{EdgeKind, Meta, MetaGraph};
use petgraph::visit::{
    Data, Dfs, EdgeRef, GraphBase, GraphRef, IntoEdgesDirected, IntoNeighbors, IntoNodeReferences,
    NodeIndexable, Topo, Visitable, Walker,
};
pub(crate) use rosetree::RoseTree;
use std::{collections::HashSet, hash::Hash};
use steel::parser::ast::ExprKind;

mod analysis;
mod codegen;
mod emit;
pub mod entrypoint;
pub mod error;
mod ir;
mod lower;
mod meta;
pub mod names;
mod rosetree;
pub mod source_map;

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

/// Options controlling [`module`] compilation.
///
/// The defaults are what regular builds want; the toggles exist for
/// optimisation ([`validate_ir`][Self::validate_ir]) and codegen debugging
/// ([`emit_all_node_fns`][Self::emit_all_node_fns]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    /// Check the scoping/arity invariants of every lowered IR body, surfacing
    /// a violation as [`ModuleError::InvalidIr`]. A violation is a bug in the
    /// compiler itself, never in the compiled graph.
    ///
    /// On by default. Disable as an optimisation, at the cost of an internal
    /// compiler error surfacing further downstream (e.g. as a confusing Steel
    /// evaluation error) instead of at lowering with a precise diagnosis.
    pub validate_ir: bool,
    /// Emit a node fn for every node - its all-connected variant - rather
    /// than only the variants called by some lowered evaluation.
    ///
    /// Off by default: node fns are normally emitted on demand, so a node
    /// that no entrypoint's evaluation calls produces no code at all. Enable
    /// to inspect any node's generated code (e.g. in the app's module view)
    /// before anything calls it. The extra definitions are never called and
    /// do not affect evaluation.
    pub emit_all_node_fns: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            validate_ir: true,
            emit_all_node_fns: false,
        }
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

/// Given a root gantz graph, generate the full module with all the necessary
/// functions for executing it: a function per node variant (input/output
/// configuration), a graph fn per nested level variant, and a function per
/// entrypoint's evaluation.
///
/// Lowers through the join-point IR pipeline (`lower` -> `ir` -> `emit`),
/// with the toggles in [`Config`] applied along the way. Nested graphs
/// compile *call-based*: each level becomes one `graph-fn-{path}-i{mask}`
/// per active-inlet variant, which parents call like a node fn. An
/// entrypoint sourced inside a nested graph becomes a per-level fn whose
/// result - `(list value state')`, with `value` a `(list branch-ix vals)`
/// pair when the push reaches the outlets through branching - the parent
/// level continues from (push-through-outlet as an ordinary value return).
pub fn module<'a, G>(
    get_node: node::GetNode<'a>,
    g: G,
    entrypoints: &[Entrypoint],
    config: &Config,
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

    let level_sources = group_sources_by_level(entrypoints);

    let mut builder = ModuleBuilder {
        meta_tree: &meta_tree,
        level_sources: &level_sources,
        config,
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
        if let Some((glue, out)) = builder.ep_level_stmts(ep_id, &meta_tree, &[])? {
            stmts.extend(glue);
            stmts.extend(emit::body_sexps(&emit::Cx { path: &[] }, &out.body));
        }
        stmts.push(emit::l([
            emit::a("set!"),
            emit::a(crate::ROOT_STATE),
            emit::a(crate::GRAPH_STATE),
        ]));
        builder
            .fns
            .push(emit::fn_def(&names::entry_fn_name(ep_id), &[], stmts));
    }

    // Every nested level compiles its all-active variant unconditionally:
    // wrapper nodes without graph-call semantics (e.g. `Fn`) inline an
    // expression that calls it, and its interior node fns must exist either
    // way (mirroring the all-connected interiors of the flow pipeline).
    let mut emitted: std::collections::BTreeSet<(Vec<node::Id>, node::Conns)> =
        std::collections::BTreeSet::new();
    let mut levels = Vec::new();
    collect_levels(&meta_tree, &mut Vec::new(), &mut levels);
    for (gpath, n_inlets) in levels {
        let imask = node::Conns::connected(n_inlets).map_err(|_| ModuleError::NodeConns {
            path: gpath.clone(),
            error: error::TooManyConns(n_inlets).into(),
        })?;
        if emitted.insert((gpath.clone(), imask)) {
            builder.graph_fn(&gpath, &imask)?;
        }
    }

    // With `emit_all_node_fns`, every node contributes its all-connected
    // variant so its node fn is emitted even when nothing reachable from an
    // entrypoint calls it. Before the fixpoint below so any graph fns these
    // variants call are generated too.
    if config.emit_all_node_fns {
        all_connected_confs(&meta_tree, &mut Vec::new(), &mut builder.confs)?;
    }

    // Generate a graph fn per (nested level, active-inlet variant) reachable
    // from the lowered bodies, to a fixpoint (graph fns may call deeper
    // variants).
    loop {
        let mut queue = Vec::new();
        for (path, confs) in &builder.confs {
            let sub = builder.meta_tree.tree(path).expect("conf path exists");
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
                builder.graph_fn(&gpath, &imask)?;
            }
        }
    }

    // A node fn per non-graph variant. The conf tree mirrors the full meta
    // tree so the generating visitor finds every level; graph-node confs
    // are excluded (they are graph fns, not node fns).
    let confs_tree = node_confs_tree(&meta_tree, &builder.confs, &mut Vec::new());
    let node_fns = codegen::node_fns(get_node, g, &confs_tree)?;

    // Definition order (Steel resolves free identifiers at definition):
    // deepest levels first; within a depth, a level's interior node fns
    // precede the graph fns of the levels at that depth. A wrapper node fn
    // (e.g. `Fn`) at depth D may call a graph fn at depth D+1; a graph fn
    // calls its interior node fns and deeper graph fns; level and entry fns
    // come last.
    let mut fns: Vec<(usize, usize, ExprKind)> = node_fns
        .into_iter()
        .map(|(depth, f)| (depth, 0, f))
        .collect();
    fns.extend(
        builder
            .graph_fns
            .into_iter()
            .map(|(depth, f)| (depth, 1, f)),
    );
    fns.sort_by(|(d1, k1, _), (d2, k2, _)| d2.cmp(d1).then(k1.cmp(k2)));
    Ok(fns
        .into_iter()
        .map(|(_, _, f)| f)
        .chain(builder.fns)
        .collect())
}

/// Insert the all-connected variant conf of every node at every level of the
/// meta tree (the [`Config::emit_all_node_fns`] set). Inlets and outlets
/// resolve as bindings and delays are intrinsics - none have node fns - so
/// they are skipped.
fn all_connected_confs(
    tree: &RoseTree<Meta>,
    path: &mut Vec<node::Id>,
    confs: &mut std::collections::BTreeMap<Vec<node::Id>, std::collections::BTreeSet<NodeConf>>,
) -> Result<(), ModuleError> {
    let conn = |n: usize, path: &[node::Id]| {
        node::Conns::connected(n).map_err(|_| ModuleError::NodeConns {
            path: path.to_vec(),
            error: error::TooManyConns(n).into(),
        })
    };
    let meta = &tree.elem;
    let level_confs = confs.entry(path.clone()).or_default();
    for n in meta.graph.nodes() {
        if meta.inlets.contains(&n) || meta.outlets.contains(&n) || meta.delays.contains(&n) {
            continue;
        }
        let inputs = conn(meta.inputs.get(&n).copied().unwrap_or(0), path)?;
        let outputs = conn(meta.outputs.get(&n).copied().unwrap_or(0), path)?;
        let conns = NodeConns { inputs, outputs };
        level_confs.insert(NodeConf { id: n, conns });
    }
    for (&id, sub) in &tree.nested {
        path.push(id);
        all_connected_confs(sub, path, confs)?;
        path.pop();
    }
    Ok(())
}

/// Collect every nested level path in the meta tree with its inlet count.
fn collect_levels(
    tree: &RoseTree<Meta>,
    path: &mut Vec<node::Id>,
    out: &mut Vec<(Vec<node::Id>, usize)>,
) {
    for (&id, sub) in &tree.nested {
        path.push(id);
        out.push((path.clone(), sub.elem.inlets.len()));
        collect_levels(sub, path, out);
        path.pop();
    }
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

/// The working state of one [`module`] invocation: accumulated node variant
/// confs and emitted fns, with the per-entrypoint level recursion and the
/// per-variant graph fn generation as methods.
struct ModuleBuilder<'a> {
    meta_tree: &'a RoseTree<Meta>,
    level_sources:
        &'a std::collections::BTreeMap<Vec<node::Id>, Vec<(EntrypointId, Vec<&'a EvalSource>)>>,
    config: &'a Config,
    /// Node variants called by the lowered bodies, per level path.
    confs: std::collections::BTreeMap<Vec<node::Id>, std::collections::BTreeSet<NodeConf>>,
    /// Emitted graph fns with their level depth. A graph fn calls only
    /// deeper graph fns, so they are defined deepest-first, before all level
    /// and entry fns (Steel resolves free identifiers at definition).
    graph_fns: Vec<(usize, ExprKind)>,
    /// Emitted level and entry fns, child levels before their parents.
    fns: Vec<ExprKind>,
}

impl ModuleBuilder<'_> {
    /// Lower the evaluation of `ep` at the level `path`: the glue statements
    /// calling child level fns (with state threading and result binding),
    /// and this level's own lowered body. Returns `None` when neither this
    /// level nor any descendant has sources for `ep`.
    fn ep_level_stmts(
        &mut self,
        ep: &EntrypointId,
        meta_node: &RoseTree<Meta>,
        path: &[node::Id],
    ) -> Result<Option<(Vec<emit::Sexp>, lower::LevelOut)>, ModuleError> {
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
            let mut child_path = path.to_vec();
            child_path.push(gid);
            let Some(piece) = self.ep_level(ep, sub_meta, &child_path)? else {
                continue;
            };
            any_child = true;
            let key = a(format!("'{gid}"));
            let pair = a(names::pair_name(gid));
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
                let pattern_union =
                    or_patterns(&patterns).map_err(|error| ModuleError::NodeConns {
                        path: path.to_vec(),
                        error,
                    })?;
                push.push((gid, pattern_union));
                extra_branches.insert(gid, patterns);
            } else {
                // Destructure all outputs from the plain result.
                let out_name =
                    |o: usize| names::var_name(&ir::Var::Output { node: gid, output: o });
                match n_out {
                    0 => {}
                    1 => glue.push(l([a("define"), a(out_name(0)), pair])),
                    _ => {
                        for o in 0..n_out {
                            glue.push(l([
                                a("define"),
                                a(out_name(o)),
                                l([a("list-ref"), pair.clone(), a(o.to_string())]),
                            ]));
                        }
                    }
                }
                for o in 0..n_out {
                    prebound_vars.push(ir::Var::Output {
                        node: gid,
                        output: o,
                    });
                }
                let conns =
                    node::Conns::connected(n_out).map_err(|_| ModuleError::NodeConns {
                        path: path.to_vec(),
                        error: error::TooManyConns(n_out).into(),
                    })?;
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
            extra_branches,
            prebound,
        };
        let out = lower::level_body(&cx, &lower::LevelSources::Eval { push, pull }).map_err(
            |error| ModuleError::Lower {
                path: path.to_vec(),
                error,
            },
        )?;
        if self.config.validate_ir {
            ir::validate(&out.body, 0, &prebound_vars).map_err(|e| ModuleError::InvalidIr {
                path: path.to_vec(),
                detail: e.to_string(),
            })?;
        }
        lower::collect_confs(&out.body, self.confs.entry(path.to_vec()).or_default());
        Ok(Some((glue, out)))
    }

    /// Build and emit the level fn evaluating `ep` at the nested level
    /// `path`, returning how the parent integrates it.
    fn ep_level(
        &mut self,
        ep: &EntrypointId,
        meta_node: &RoseTree<Meta>,
        path: &[node::Id],
    ) -> Result<Option<LevelPiece>, ModuleError> {
        use emit::{a, l};
        let Some((glue, out)) = self.ep_level_stmts(ep, meta_node, path)? else {
            return Ok(None);
        };

        // Whether (and how) this level's evaluation produces its outlets,
        // analysed over the lowered body itself.
        let produced = out.outlets.iter().any(|o| o.atom.is_some());
        let patterns = match produced {
            true => Some(analysis::outlet_patterns(&out.body, &out.outlets).map_err(
                |error| ModuleError::NodeConns {
                    path: path.to_vec(),
                    error: error.into(),
                },
            )?),
            false => None,
        };
        let result = match &patterns {
            Some(patterns) => emit::level_result_sexp(&out.outlets, patterns),
            None => emit::unit(),
        };

        let stateful = !meta_node.elem.stateful.is_empty();
        let fn_name = names::lvl_fn_name(ep, path);
        let mut body = Vec::new();
        let params: Vec<String> = if stateful {
            body.push(l([a("define"), a(crate::GRAPH_STATE), a("%lvl-state")]));
            vec!["%lvl-state".to_string()]
        } else {
            vec![]
        };
        body.extend(glue);
        body.extend(emit::body_sexps(&emit::Cx { path }, &out.body));
        if stateful {
            body.push(l([a("list"), result, a(crate::GRAPH_STATE)]));
        } else {
            body.push(result);
        }
        self.fns.push(emit::fn_def(&fn_name, &params, body));
        Ok(Some(LevelPiece {
            fn_name,
            stateful,
            patterns,
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
            extra_branches: std::collections::BTreeMap::new(),
            prebound: std::collections::BTreeSet::new(),
        };
        let out = lower::level_body(&cx, &lower::LevelSources::Inlets(active.clone())).map_err(
            |error| ModuleError::Lower {
                path: gpath.to_vec(),
                error,
            },
        )?;
        if self.config.validate_ir {
            let inlet_vars: Vec<ir::Var> = active
                .iter()
                .map(|&i| ir::Var::Output { node: i, output: 0 })
                .collect();
            ir::validate(&out.body, 0, &inlet_vars).map_err(|e| ModuleError::InvalidIr {
                path: gpath.to_vec(),
                detail: e.to_string(),
            })?;
        }
        lower::collect_confs(&out.body, self.confs.entry(gpath.to_vec()).or_default());

        // The node-contract patterns: the all-active analysis, matching the
        // order `Graph::branches` reports to the parent.
        let patterns =
            analysis::level_branch_patterns(meta).map_err(|error| ModuleError::Lower {
                path: gpath.to_vec(),
                error,
            })?;
        let result = emit::level_result_sexp(&out.outlets, &patterns);
        let stateful = !meta.stateful.is_empty();
        let ecx = emit::Cx { path: gpath };
        let mut stmts = Vec::new();
        let mut params: Vec<String> = active
            .iter()
            .map(|&i| names::var_name(&ir::Var::Output { node: i, output: 0 }))
            .collect();
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
        let fn_def = emit::fn_def(&names::graph_fn_name(gpath, imask), &params, stmts);
        self.graph_fns.push((gpath.len(), fn_def));
        Ok(())
    }
}

/// Build the node-fn conf tree mirroring the meta tree, excluding
/// nested-graph node confs (those compile to graph fns).
fn node_confs_tree(
    meta_node: &RoseTree<Meta>,
    confs: &std::collections::BTreeMap<Vec<node::Id>, std::collections::BTreeSet<NodeConf>>,
    path: &mut Vec<node::Id>,
) -> RoseTree<std::collections::BTreeSet<NodeConf>> {
    let elem = confs.get(path).cloned().unwrap_or_default();
    let nested = meta_node
        .nested
        .iter()
        .map(|(&id, sub)| {
            path.push(id);
            let t = node_confs_tree(sub, confs, path);
            path.pop();
            (id, t)
        })
        .collect();
    RoseTree { elem, nested }
}

/// The inlet ids active for a parent input-conns mask (bit `i` <=> `inlet_ids[i]`,
/// the "input i -> inlet i" contract shared with `node::graph::graph_call_expr`).
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
