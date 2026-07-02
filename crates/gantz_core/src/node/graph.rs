//! [`Node`] implementations for nested graphs.
//!
//! A nested graph is a [`Graph`] referenced by a [`Ref`](crate::node::Ref):
//! the `Graph<N>: Node` impl here compiles it (a call to its graph fn, see
//! [`graph_call_expr`]), while [`Inlet`]/[`Outlet`] mark its input/output
//! interface.

use crate::{
    Edge, compile,
    node::{self, Node},
    visit,
};
use gantz_ca::CaHash;
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
/// A plain (non-stable) `petgraph::Graph`: node indices stay contiguous (`0..n`)
/// because `remove_node` swap-removes (the former-last node adopts the removed
/// index). Callers that key persistent data by node index must migrate the
/// swapped node on removal - see `gantz_core::node::state::move_value`.
pub type Graph<N> = petgraph::graph::Graph<N, Edge, Directed, Index>;

/// The type used for indexing into the graph.
pub type Index = usize;
/// The type used to index into a graph's node's.
pub type NodeIx = NodeIndex<Index>;
/// The type used to index into a graph's edge's.
pub type EdgeIx = EdgeIndex<Index>;

/// An inlet to a nested graph.
///
/// Inlet values are provided via `define` bindings by the parent graph node.
///
/// `ty` and `description` are optional, GUI-facing documentation for the inlet
/// (a short "type" label and a longer note). They are plain data stored with
/// the node; the GUI layer interprets and presents them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, CaHash, NodeTag)]
#[cahash("gantz.inlet")]
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
/// See [`Inlet`] regarding `ty`/`description`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, CaHash, NodeTag)]
#[cahash("gantz.outlet")]
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
    /// active-input variant, so a graph node compiles like any other node: a
    /// node fn whose body is the call (see [`graph_call_expr`]).
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

/// Compute the external branch masks for a nested graph (see [`Node::branches`]):
/// the distinct outlet-activation patterns of the all-inlets-active analysis.
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
/// active-input variant (`graph-fn-{path}-i{mask}`, see [`compile::module`]).
///
/// The graph fn yields all outlet values (raw for one outlet, a `(list ...)`
/// for several), or a `(list branch-ix value)` pair matching
/// `graph_branches` when the interior branches externally. Branching pairs
/// pass through untouched (the value is already shaped per arm, mirroring
/// the `Branch` node contract); otherwise, when only a subset of several
/// outlets is consumed (`outputs`), the active subset is selected so the
/// expression honours the node-fn result contract. A stateful interior
/// threads the node's `state` (the nested level's state hashmap) through
/// the call.
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
