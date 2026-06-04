//! An implementation of a node acting as a nested graph.

use crate::{
    Edge, GRAPH_STATE, compile,
    node::{self, Node},
    visit,
};
use gantz_ca::CaHash;
use petgraph::{
    Directed,
    graph::{EdgeIndex, NodeIndex},
    visit::{
        Data, IntoEdgeReferences, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, NodeRef,
        Visitable,
    },
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    ops::{Deref, DerefMut},
};

/// The graph type used by the graph node to represent its nested graph.
pub type Graph<N> = petgraph::stable_graph::StableGraph<N, Edge, Directed, Index>;

/// The type used for indexing into the graph.
pub type Index = usize;
/// The type used to index into a graph's node's.
pub type NodeIx = NodeIndex<Index>;
/// The type used to index into a graph's edge's.
pub type EdgeIx = EdgeIndex<Index>;

/// A node that itself is implemented in terms of a graph of nodes.
///
/// While an implementation of [`Node`] is also provided for [`Graph`], the
/// `Graph` type is defined in the petgraph crate. As a result, we cannot ensure
/// it implements all of the upstream traits we require. By providing a
/// dedicated `GraphNode` type, we can also provide implementations for any
/// upstream traits we might need.
#[derive(Clone, Debug)]
pub struct GraphNode<N> {
    /// The nested graph.
    pub graph: Graph<N>,
}

/// An inlet to a nested graph.
///
/// Inlet values are provided via `define` bindings by the parent graph node.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, CaHash)]
#[cahash("gantz.inlet")]
pub struct Inlet;

/// An outlet from a nested graph.
///
/// Outlet values are passed through directly as the node's output.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, CaHash)]
#[cahash("gantz.outlet")]
pub struct Outlet;

impl<N> Default for GraphNode<N> {
    fn default() -> Self {
        let graph = Default::default();
        GraphNode { graph }
    }
}

impl<N> Hash for GraphNode<N>
where
    N: Hash,
{
    fn hash<H>(&self, hasher: &mut H)
    where
        H: Hasher,
    {
        crate::graph::hash(&self.graph, hasher);
    }
}

impl<N: Node> Node for Graph<N> {
    fn n_inputs(&self, ctx: node::MetaCtx) -> usize {
        self.node_references()
            .filter(|n_ref| n_ref.weight().inlet(ctx))
            .count()
    }

    fn n_outputs(&self, ctx: node::MetaCtx) -> usize {
        self.node_references()
            .filter(|n_ref| n_ref.weight().outlet(ctx))
            .count()
    }

    fn branches(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        // Any malformed-graph error surfaces authoritatively when `module()`
        // builds this same graph, so falling back to "not branching" here is
        // safe and keeps `branches()` consistent with `nested_expr`.
        graph_branches(ctx.get_node(), self)
            .unwrap_or_default()
            .into_iter()
            .map(node::EvalConf::Set)
            .collect()
    }

    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        nested_expr(ctx.get_node(), self, ctx.path(), ctx.inputs())
    }

    fn stateful(&self, ctx: node::MetaCtx) -> bool {
        self.node_references()
            .any(|n_ref| n_ref.weight().stateful(ctx))
    }

    fn register(&self, _ctx: node::RegCtx<'_, '_>) {
        // Graph state hashmaps are lazily initialized by `update_value` when
        // nested stateful nodes register their state.
    }

    fn visit(&self, ctx: visit::Ctx<'_, '_>, visitor: &mut dyn node::Visitor) {
        crate::graph::visit(ctx.get_node(), self, ctx.path(), visitor);
    }
}

impl<N: Node> Node for GraphNode<N> {
    fn n_inputs(&self, ctx: node::MetaCtx) -> usize {
        self.graph.n_inputs(ctx)
    }

    fn n_outputs(&self, ctx: node::MetaCtx) -> usize {
        self.graph.n_outputs(ctx)
    }

    fn branches(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        self.graph.branches(ctx)
    }

    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        self.graph.expr(ctx)
    }

    fn stateful(&self, ctx: node::MetaCtx) -> bool {
        self.graph.stateful(ctx)
    }

    fn register(&self, ctx: node::RegCtx<'_, '_>) {
        self.graph.register(ctx)
    }

    fn visit(&self, ctx: visit::Ctx<'_, '_>, visitor: &mut dyn node::Visitor) {
        self.graph.visit(ctx, visitor)
    }
}

impl<N: PartialEq> PartialEq for GraphNode<N> {
    fn eq(&self, other: &Self) -> bool {
        graph_partial_eq(self, other)
    }
}

impl<N: Eq> Eq for GraphNode<N> {}

// Manual implementation of `Deserialize` as it cannot be derived for a struct with associated
// types without unnecessary trait bounds on the struct itself.
impl<'de, N> Deserialize<'de> for GraphNode<N>
where
    N: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, SeqAccess, Visitor};

        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Graph,
        }

        struct GraphNodeVisitor<G>(std::marker::PhantomData<G>);

        impl<'de, N> Visitor<'de> for GraphNodeVisitor<Graph<N>>
        where
            N: Deserialize<'de>,
        {
            type Value = GraphNode<N>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct GraphNode")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<GraphNode<N>, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let graph = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                Ok(GraphNode { graph })
            }

            fn visit_map<V>(self, mut map: V) -> Result<GraphNode<N>, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut graph = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Graph => {
                            if graph.is_some() {
                                return Err(de::Error::duplicate_field("graph"));
                            }
                            graph = Some(map.next_value()?);
                        }
                    }
                }
                let graph = graph.ok_or_else(|| de::Error::missing_field("graph"))?;
                Ok(GraphNode { graph })
            }
        }

        const FIELDS: &[&str] = &["graph"];
        let visitor: GraphNodeVisitor<Graph<N>> = GraphNodeVisitor(std::marker::PhantomData);
        deserializer.deserialize_struct("GraphNode", FIELDS, visitor)
    }
}

// Manual implementation of `Serialize` as it cannot be derived for a struct with associated
// types without unnecessary trait bounds on the struct itself.
impl<N> Serialize for GraphNode<N>
where
    N: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("GraphNode", 3)?;
        state.serialize_field("graph", &self.graph)?;
        state.end()
    }
}

impl Node for Inlet {
    /// This method should never be called during compilation.
    ///
    /// Inlet nodes are special-cased to enable statelessness:
    /// - No node functions are generated for inlets (skipped in NodeFns visitor)
    /// - Inlet values are provided via direct `(define inlet-{ix} ...)` bindings in nested_expr
    /// - eval_stmt creates simple aliases to these bindings rather than calling node functions
    ///
    /// Returns `'()` as a safe fallback in case this is ever called outside normal compilation.
    fn expr(&self, _ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        node::parse_expr("'()")
    }

    fn n_inputs(&self, _ctx: node::MetaCtx) -> usize {
        0
    }

    fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
        1
    }

    fn inlet(&self, _ctx: node::MetaCtx) -> bool {
        true
    }
}

impl Node for Outlet {
    /// This method should never be called during compilation.
    ///
    /// Outlet nodes are special-cased to enable statelessness:
    /// - No node functions are generated for outlets (skipped in NodeFns visitor)
    /// - No evaluation statements are generated for outlets (skipped in eval_stmt)
    /// - Outlet values are read directly from source node output bindings by nested_expr
    ///
    /// Returns `'()` as a safe fallback in case this is ever called outside normal compilation.
    fn expr(&self, _ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        node::parse_expr("'()")
    }

    fn n_inputs(&self, _ctx: node::MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
        0
    }

    fn outlet(&self, _ctx: node::MetaCtx) -> bool {
        true
    }
}

impl<N> CaHash for GraphNode<N>
where
    N: CaHash,
{
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        gantz_ca::hash_graph(&self.graph, hasher);
    }
}

impl<N> Deref for GraphNode<N> {
    type Target = Graph<N>;
    fn deref(&self) -> &Self::Target {
        &self.graph
    }
}

impl<N> DerefMut for GraphNode<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.graph
    }
}

/// A `PartialEq` implementation for [`Graph`].
pub fn graph_partial_eq<N: PartialEq>(a: &Graph<N>, b: &Graph<N>) -> bool {
    a.node_references()
        .zip(b.node_references())
        .all(|(a, b)| a == b)
        && a.edge_references()
            .zip(b.edge_references())
            .all(|(a, b)| a == b)
}

/// Map a nested graph's branch arm counts (`id -> n_arms`) from its `Meta`.
fn branch_arm_counts(meta: &compile::Meta) -> BTreeMap<node::Id, usize> {
    meta.branches.iter().map(|(&id, v)| (id, v.len())).collect()
}

/// The distinct external branches of a nested graph, as output masks over its
/// outlets (ascending id order), or an empty `Vec` when it does not branch.
///
/// Each pattern is one set of outlets that may be simultaneously active under
/// some combination of inner branch outcomes (see [`compile::outlet_patterns`]).
/// Fewer than two distinct patterns means the graph always produces the same
/// outlets, i.e. it has no external branching.
fn branch_patterns_from_flow(
    fg: &compile::FlowGraph,
    outlet_ids: &[node::Id],
    branching: &BTreeMap<node::Id, usize>,
) -> Result<Vec<node::Conns>, node::ExprError> {
    let outlets: BTreeSet<node::Id> = outlet_ids.iter().copied().collect();
    let mut patterns: BTreeSet<node::Conns> = BTreeSet::new();
    for reached in compile::outlet_patterns(fg, &outlets, branching) {
        let mut conns = node::Conns::unconnected(outlet_ids.len()).map_err(node::ExprError::custom)?;
        for (i, id) in outlet_ids.iter().enumerate() {
            if reached.contains(id) {
                conns.set(i, true).map_err(node::ExprError::custom)?;
            }
        }
        patterns.insert(conns);
    }
    if patterns.len() < 2 {
        return Ok(vec![]);
    }
    Ok(patterns.into_iter().collect())
}

/// Compute the external branch masks for a nested graph (see [`Node::branches`]).
fn graph_branches<'a, G>(
    get_node: node::GetNode<'a>,
    g: G,
) -> Result<Vec<node::Conns>, node::ExprError>
where
    G: IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable + Data<EdgeWeight = Edge>,
    G::NodeWeight: Node,
    G::NodeId: Eq + Hash,
{
    let meta = compile::Meta::from_graph(get_node, g).map_err(node::ExprError::custom)?;
    // No inner branching => no external branching.
    if meta.branches.is_empty() {
        return Ok(vec![]);
    }
    let outlet_ids: Vec<node::Id> = meta.outlets.iter().copied().collect();
    let fg = inner_flow_graph(&meta)?;
    branch_patterns_from_flow(&fg, &outlet_ids, &branch_arm_counts(&meta))
}

/// Build the control flow graph for a nested graph: inlets push, outlets pull.
fn inner_flow_graph(meta: &compile::Meta) -> Result<compile::FlowGraph, node::ExprError> {
    let conn = || node::Conns::connected(1).unwrap();
    compile::flow_graph(
        meta,
        meta.inlets.iter().map(|&n| (n, conn())),
        meta.outlets.iter().map(|&n| (n, conn())),
    )
    .map_err(node::ExprError::custom)
}

/// The Steel expression that yields a nested graph's outlet values, shaped by
/// the given outlets: a single raw value for one outlet, a `(list ...)` for
/// several, or `'()` for none. Each value is read from its hoisted `outlet-{id}`
/// var.
fn outlet_values_expr(outlet_ids: &[node::Id]) -> String {
    match outlet_ids {
        [] => "'()".to_string(),
        [id] => format!("outlet-{id}"),
        ids => {
            let values: Vec<_> = ids.iter().map(|id| format!("outlet-{id}")).collect();
            format!("(list {})", values.join(" "))
        }
    }
}

/// The Steel expression selecting `(list branch-ix value)` for a branching
/// nested graph.
///
/// Each distinct outlet-activation pattern has a unique integer signature
/// (outlet `i` contributes `2^i` when active); the runtime signature is built
/// from the hoisted `outlet-active-{id}` flags and matched against each
/// pattern's signature with a nested `if` (the last pattern is the exhaustive
/// fallthrough). Only primitive Steel forms are used, since the VM runs the
/// base engine without the `cond`/`and` prelude macros.
fn branch_selector(patterns: &[node::Conns], outlet_ids: &[node::Id]) -> String {
    // The value to return for a pattern, shaped by its active outlet count.
    let value = |conns: &node::Conns| -> String {
        let active: Vec<node::Id> = outlet_ids
            .iter()
            .enumerate()
            .filter_map(|(i, &id)| conns.get(i).unwrap_or(false).then_some(id))
            .collect();
        outlet_values_expr(&active)
    };
    // The unique signature of a pattern's active outlets.
    let signature = |conns: &node::Conns| -> u128 {
        (0..outlet_ids.len())
            .filter(|&i| conns.get(i).unwrap_or(false))
            .map(|i| 1u128 << i)
            .sum()
    };

    // The runtime signature, summed from the active flags.
    let terms: Vec<String> = outlet_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| format!("(if outlet-active-{id} {} 0)", 1u128 << i))
        .collect();
    let sig_expr = match terms.as_slice() {
        [] => "0".to_string(),
        [t] => t.clone(),
        _ => format!("(+ {})", terms.join(" ")),
    };

    // Nested `if` over patterns; the last is the exhaustive fallthrough.
    let last = patterns.len() - 1;
    let mut expr = format!("(list {last} {})", value(&patterns[last]));
    for k in (0..last).rev() {
        expr = format!(
            "(if (= __branch-sig {}) (list {k} {}) {expr})",
            signature(&patterns[k]),
            value(&patterns[k]),
        );
    }
    format!("(let ((__branch-sig {sig_expr})) {expr})")
}

/// The implementation of the `GraphNode`'s `Node::expr` fn.
///
/// The nested graph is inlined as an expression. Inlet values are bound from the
/// node's inputs; the inner control flow runs with outlets writing their values
/// (and, when the graph branches externally, their "active" flags) to hoisted
/// vars. The result is the outlet values, or - when branching - a
/// `(list branch-ix value)` pair matching [`graph_branches`].
pub fn nested_expr<'a, G>(
    get_node: node::GetNode<'a>,
    g: G,
    path: &[node::Id],
    inputs: &[Option<String>],
) -> node::ExprResult
where
    G: IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable + Data<EdgeWeight = Edge>,
    G::NodeWeight: Node,
    G::NodeId: Eq + Hash,
{
    let meta = compile::Meta::from_graph(get_node, g).map_err(node::ExprError::custom)?;
    let inlet_ids: Vec<node::Id> = meta.inlets.iter().copied().collect();
    let outlet_ids: Vec<node::Id> = meta.outlets.iter().copied().collect();
    let fg = inner_flow_graph(&meta)?;
    let arm_counts = branch_arm_counts(&meta);

    // The external branch patterns (empty => not externally branching).
    let patterns = branch_patterns_from_flow(&fg, &outlet_ids, &arm_counts)?;
    let branching = !patterns.is_empty();

    // Bind each inlet from the corresponding node input (input i -> inlet i).
    let mut bindings: Vec<String> = inlet_ids
        .iter()
        .enumerate()
        .map(|(i, &inlet_id)| {
            let input = inputs
                .get(i)
                .and_then(Clone::clone)
                .unwrap_or_else(|| "'()".to_string());
            format!("(define inlet-{inlet_id} {input})")
        })
        .collect();

    // Declare the hoisted outlet value vars (and active flags when branching)
    // that the flow statements `set!` wherever an outlet is reached.
    for &id in &outlet_ids {
        bindings.push(format!("(define outlet-{id} '())"));
        if branching {
            bindings.push(format!("(define outlet-active-{id} #f)"));
        }
    }

    let stmts = compile::entry_fn_body(
        path,
        &meta.graph,
        &meta.stateful,
        &arm_counts,
        &meta.inlets,
        &meta.outlets,
        &fg,
        &BTreeSet::new(),
        branching,
    )
    .map_err(node::ExprError::custom)?;

    let body = bindings
        .into_iter()
        .chain(stmts.iter().map(|stmt| format!("{stmt}")))
        .collect::<Vec<_>>()
        .join(" ");

    // The node's output: a `(list branch-ix value)` when branching, else the
    // outlet values directly.
    let tail = if branching {
        branch_selector(&patterns, &outlet_ids)
    } else {
        outlet_values_expr(&outlet_ids)
    };

    // Wrap in `(let () ...)` so the inner statements form a *body*: unlike a
    // bare `(begin ...)` used as an expression, a body permits `define`s
    // interleaved with the branch `if` expressions of multiple components.
    // Include state handling only if the graph has stateful nodes.
    let expr_str = if meta.stateful.is_empty() {
        format!("(let () {body} {tail})")
    } else {
        format!("(let () (define {GRAPH_STATE} state) {body} (set! state {GRAPH_STATE}) {tail})")
    };
    node::parse_expr(&expr_str)
}
