//! A node that wraps another node as a first-class function value.

use crate::{
    node::{self, Node},
    visit,
};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// A node that emits a lambda function wrapping another node's expression.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Fn<N>(pub N);

/// The wire tag for [`Fn<Self>`], for node types that appear fn-wrapped in a
/// node set.
///
/// `Fn<N>` is foreign to `N`'s crate, so the orphan rule forbids implementing
/// [`NodeTag`] for it there directly. This trait lets the wrapped type
/// declare the wrapper's tag at its own definition site instead. The blanket
/// impl below lifts it.
pub trait FnNodeTag {
    /// The `"type"` tag identifying `Fn<Self>` on the wire.
    const FN_TAG: &'static str;
}

impl<N: FnNodeTag> NodeTag for Fn<N> {
    const TAG: &'static str = N::FN_TAG;
}

impl<N> Fn<N> {
    /// Create a new `Fn` node that wraps the given node.
    pub fn new(node: N) -> Self {
        Self(node)
    }
}

impl<N: Node> Node for Fn<N> {
    /// A bang input, then one input for each input of the inner node.
    fn n_inputs(&self, ctx: node::MetaCtx) -> usize {
        1 + self.0.n_inputs(ctx)
    }

    /// A single output that emits the lambda function value.
    fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
        1
    }

    /// Produces an expression that returns a lambda that wraps the inner
    /// node's expression.
    ///
    /// A connected input of the inner node binds its value when the lambda is
    /// made. The lambda takes the inner node's other inputs as its arguments,
    /// in order.
    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        // Only emit a lambda when an input is connected. Otherwise emit an
        // empty list.
        if ctx.inputs().iter().all(Option::is_none) {
            return node::parse_expr("'()");
        }

        let get_node = ctx.get_node();
        let meta_ctx = node::MetaCtx::new(get_node);
        let node = &self.0;

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

        // Each inner input reads a binding of a connected input of this node,
        // or an argument of the lambda.
        let n_inputs = node.n_inputs(meta_ctx);
        let mut binds = vec![];
        let mut params = vec![];
        let mut input_refs = vec![];
        for i in 0..n_inputs {
            let name = match ctx.inputs().get(1 + i).and_then(Option::as_ref) {
                Some(value) => {
                    let name = format!("cap{i}");
                    binds.push(format!("({name} {value})"));
                    name
                }
                None => {
                    let name = format!("arg{i}");
                    params.push(name.clone());
                    name
                }
            };
            input_refs.push(Some(name));
        }
        let outputs = node::Conns::connected(n_outputs).unwrap();
        let path = ctx.path();
        let ectx = node::ExprCtx::new(get_node, path, &input_refs, &outputs);
        let expr = node.expr(ectx)?;

        let expr_str = expr.to_pretty(80);
        let lambda_expr = format!("(lambda ({}) {})", params.join(" "), expr_str);
        if binds.is_empty() {
            node::parse_expr(&lambda_expr)
        } else {
            node::parse_expr(&format!("(let ({}) {lambda_expr})", binds.join(" ")))
        }
    }

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        self.0.required_addrs()
    }

    fn required_blobs(&self) -> Vec<(gantz_ca::SectionId, gantz_ca::ContentAddr)> {
        self.0.required_blobs()
    }

    /// Forwarded from the inner node. The emitted lambda inlines the inner
    /// node's expression, so its module bindings are load-bearing.
    fn required_modules(&self, ctx: node::MetaCtx) -> Vec<String> {
        self.0.required_modules(ctx)
    }

    fn visit(&self, ctx: visit::Ctx<'_, '_>, visitor: &mut dyn node::Visitor) {
        self.0.visit(ctx, visitor);
    }
}
