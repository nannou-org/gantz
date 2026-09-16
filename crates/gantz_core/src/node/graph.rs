//! [`Node`] implementations for nested graphs.
//!
//! A nested graph is a [`Graph`] referenced by a [`Ref`](crate::node::Ref).
//! The `Graph<N>: Node` impl here compiles it as a call to its graph fn. See
//! [`graph_call_expr`]. [`Inlet`] and [`Outlet`] mark its input and output
//! interface.

use crate::{
    Edge, compile,
    node::{self, Node},
    visit,
};
use gantz_nodetag::NodeTag;
use petgraph::{
    Directed,
    graph::{EdgeIndex, NodeIndex},
    visit::{Data, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, NodeRef, Visitable},
};
use serde::{Deserialize, Serialize};
use std::hash::Hash;

/// The graph type used to represent a nested graph.
///
/// A plain, non-stable `petgraph::Graph`. Node indices stay contiguous in
/// `0..n` because `remove_node` swap-removes. The former-last node adopts the
/// removed index. Callers that key persistent data by node index must migrate
/// the swapped node on removal. See `gantz_core::node::state::move_value`.
pub type Graph<N> = petgraph::graph::Graph<N, Edge, Directed, Index>;

/// The type used for indexing into the graph.
pub type Index = usize;
/// The type used to index a graph's nodes.
pub type NodeIx = NodeIndex<Index>;
/// The type used to index a graph's edges.
pub type EdgeIx = EdgeIndex<Index>;

/// An inlet to a nested graph.
///
/// Inlet values are provided via `define` bindings by the parent graph node.
///
/// `ty` and `description` are optional GUI-facing documentation for the
/// inlet. `ty` is a short type label and `description` a longer note. They
/// are plain data stored with the node. The GUI layer interprets and presents
/// them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, NodeTag)]
pub struct Inlet {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ty: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// An outlet from a nested graph.
///
/// Outlet values are passed through directly as the node's output.
///
/// See [`Inlet`] regarding `ty` and `description`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, NodeTag)]
pub struct Outlet {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ty: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
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

    /// The expression calls the graph fn compiled for this graph's
    /// active-input variant, so a graph node compiles like any other node.
    /// Its node fn body is the call. See [`graph_call_expr`].
    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        graph_call_expr(
            ctx.get_node(),
            self,
            ctx.path(),
            ctx.inputs(),
            ctx.outputs(),
        )
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

impl Node for Inlet {
    /// Never called during compilation. The compiler special-cases inlets.
    /// No node fn is generated and inlet values resolve as bindings.
    ///
    /// Returns `'()` as a safe fallback.
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
    /// Never called during compilation. The compiler special-cases outlets.
    /// No node fn is generated and outlet values are read from their source
    /// bindings.
    ///
    /// Returns `'()` as a safe fallback.
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

/// Compute the external branch masks for a nested graph. See
/// [`Node::branches`]. These are the distinct outlet-activation patterns of
/// the all-inlets-active analysis.
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
    compile::level_branch_patterns(&meta).map_err(node::ExprError::custom)
}

/// The expression calling the graph fn compiled for this graph's
/// active-input variant. The fn is `graph-fn-{path}-i{mask}`. See
/// [`compile::module`].
///
/// The graph fn yields all outlet values. That is raw for one outlet and a
/// `(list ...)` for several. When the interior branches externally it yields
/// a `(list branch-ix value)` pair matching `graph_branches`. Branching pairs
/// pass through untouched, since the value is already shaped per arm like
/// the `Branch` node contract. Otherwise, when `outputs` selects only a
/// subset of several outlets, the active subset is selected so the
/// expression honours the node-fn result contract. A stateful interior
/// threads the node's `state` through the call. That is the nested level's
/// state hashmap.
pub fn graph_call_expr<'a, G>(
    get_node: node::GetNode<'a>,
    g: G,
    path: &[node::Id],
    inputs: &[Option<String>],
    outputs: &node::Conns,
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
    let call = if stateful {
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

    let n_outlets = g
        .node_references()
        .filter(|n_ref| n_ref.weight().outlet(meta_ctx))
        .count();
    let active: Vec<usize> = outputs
        .iter()
        .enumerate()
        .filter_map(|(o, b)| b.then_some(o))
        .collect();
    let branching = !graph_branches(get_node, g)?.is_empty();
    if branching || n_outlets <= 1 || active.is_empty() || active.len() == n_outlets {
        return node::parse_expr(&call);
    }

    // Select the consumed subset from the full outlet list.
    let selection = match active.as_slice() {
        [k] => format!("(list-ref %gantz-out {k})"),
        ks => {
            let refs: Vec<String> = ks
                .iter()
                .map(|k| format!("(list-ref %gantz-out {k})"))
                .collect();
            format!("(list {})", refs.join(" "))
        }
    };
    node::parse_expr(&format!("(let ((%gantz-out {call})) {selection})"))
}
