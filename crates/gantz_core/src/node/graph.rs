//! An implementation of a node acting as a nested graph.

use crate::{
    Edge, compile,
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
        // safe and keeps `branches()` consistent with the graph fn selector.
        graph_branches(ctx.get_node(), self)
            .unwrap_or_default()
            .into_iter()
            .map(node::EvalConf::Set)
            .collect()
    }

    /// The compiler bypasses this (graph nodes lower to graph-fn calls
    /// directly), but wrapping nodes such as [`node::Fn`] inline it: the
    /// expression calls the graph fn for the active-input variant.
    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        graph_call_expr(ctx.get_node(), self, ctx.path(), ctx.inputs())
    }

    fn graph(&self, _ctx: node::MetaCtx) -> bool {
        true
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

    fn graph(&self, ctx: node::MetaCtx) -> bool {
        self.graph.graph(ctx)
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
    compile::branch_patterns_from_flow(&fg, &outlet_ids, &branch_arm_counts(&meta))
        .map_err(node::ExprError::custom)
}

/// Build the all-inlets-active interior control flow graph (inlets push,
/// outlets pull), used by [`graph_branches`] - which must report the
/// union/over-approximation of external branch patterns. See
/// [`compile::inner_flow_graph_for`] for the active-set aware variant.
fn inner_flow_graph(meta: &compile::Meta) -> Result<compile::FlowGraph, node::ExprError> {
    let all: BTreeSet<node::Id> = meta.inlets.iter().copied().collect();
    compile::inner_flow_graph_for(meta, &all).map_err(node::ExprError::custom)
}

/// The expression calling the graph fn compiled for this graph's
/// active-input variant (`graph-fn-{path}-i{mask}`, see [`compile::module`]).
///
/// The result is the outlet values (raw for one outlet, a `(list ...)` for
/// several), or a `(list branch-ix value)` pair matching `graph_branches`
/// when the interior branches externally. A stateful interior threads the
/// node's `state` (the nested level's state hashmap) through the call.
pub fn graph_call_expr<'a, G>(
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
    let imask = node::Conns::try_from_iter(inputs.iter().map(Option::is_some))
        .map_err(node::ExprError::custom)?;
    let fn_name = compile::graph_fn_name(path, &imask);
    let mut args: Vec<String> = inputs.iter().flatten().cloned().collect();
    let meta_ctx = node::MetaCtx::new(get_node);
    let stateful = g
        .node_references()
        .any(|n_ref| n_ref.weight().stateful(meta_ctx));
    let expr_str = if stateful {
        args.push("state".to_string());
        format!(
            "(let ((%gantz-r ({fn_name} {})))
               (set! state (list-ref %gantz-r 1))
               (list-ref %gantz-r 0))",
            args.join(" "),
        )
    } else {
        format!("({fn_name} {})", args.join(" "))
    };
    node::parse_expr(&expr_str)
}
