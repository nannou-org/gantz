//! A node that wraps another node as a first-class function value.

use crate::{
    node::{self, Node},
    visit,
};
use gantz_ca::CaHash;
use serde::{Deserialize, Serialize};

/// A node that emits a lambda function wrapping another node's expression.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.fn")]
pub struct Fn<N>(pub N);

impl<N> Fn<N> {
    /// Create a new Fn node that wraps the node at the given address.
    pub fn new(node: N) -> Self {
        Self(node)
    }
}

impl<N: Node> Node for Fn<N> {
    /// A single input for `Bang`-triggering.
    fn n_inputs(&self, _ctx: node::MetaCtx) -> usize {
        1
    }

    /// A single output that emits the function value (lambda).
    fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
        1
    }

    /// Produces an expression that, in response to a bang on its input, returns
    /// a lambda that wraps the inner node's expression.
    ///
    /// The returned function receives a number of arguments equal to the inner
    /// node's `Node::n_inputs` count.
    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        // Only emit a lambda in the case that some input is connected for
        // receiving bangs. Otherwise, we'll just emit an empty list.
        if ctx.inputs().get(0).and_then(|conn| conn.as_ref()).is_none() {
            return node::parse_expr("'()");
        }

        // Create a MetaCtx for querying node metadata.
        let get_node = ctx.get_node();
        let meta_ctx = node::MetaCtx::new(get_node);
        let node = &self.0;

        // Validate the node (must be stateless and non-branching).
        if node.stateful(meta_ctx) {
            return Err(node::ExprError::custom(
                "nodes used as functions must be stateless",
            ));
        }
        if !node.branches(meta_ctx).is_empty() {
            return Err(node::ExprError::custom(
                "nodes used as functions must not branch",
            ));
        }
        let n_outputs = node.n_outputs(meta_ctx);
        if n_outputs > 1 {
            return Err(node::ExprError::custom(
                "nodes used as functions must have at most one output",
            ));
        }

        // Generate the node's expression with a placeholder path.
        let n_inputs = node.n_inputs(meta_ctx);
        let params: Vec<_> = (0..n_inputs).map(|i| format!("arg{i}")).collect();
        let input_refs: Vec<Option<String>> = params.iter().map(|p| Some(p.clone())).collect();
        let outputs = node::Conns::connected(n_outputs).unwrap();
        let path = ctx.path();
        let ectx = node::ExprCtx::new(get_node, path, &input_refs, &outputs);
        let expr = node.expr(ectx)?;

        // Create the lambda that we'll return.
        let expr_str = expr.to_pretty(80);
        let params_str = params.join(" ");
        let lambda_expr = format!("(lambda ({}) {})", params_str, expr_str);
        node::parse_expr(&lambda_expr)
    }

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        self.0.required_addrs()
    }

    fn visit(&self, ctx: visit::Ctx<'_, '_>, visitor: &mut dyn node::Visitor) {
        self.0.visit(ctx, visitor);
    }
}
