//! A node that wraps another node as a first-class function value.

use crate::{node::{self, Node}, visit};
use serde::{Deserialize, Serialize};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// A node that emits a lambda function wrapping another node's expression.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Fn<N>(pub N);

impl<N> Fn<N> {
    /// Create a new Fn node that wraps the node at the given address.
    pub fn new(node: N) -> Self {
        Self(node)
    }
}

impl<Env, N> Node<Env> for Fn<N>
where
    N: Node<Env>,
{
    /// A single input for `Bang`-triggering.
    fn n_inputs(&self, _env: &Env) -> usize {
        1
    }

    /// A single output that emits the function value (lambda).
    fn n_outputs(&self, _env: &Env) -> usize {
        1
    }

    /// Produces an expression that, in response to a bang on its input, returns
    /// a lambda that wraps the inner node's expression.
    ///
    /// The returned function receives a number of arguments equal to the inner
    /// node's `Node::n_inputs` count.
    fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
        // Only emit a lambda in the case that some input is connected for
        // receiving bangs. Otherwise, we'll just emit an empty list.
        if ctx.inputs().get(0).and_then(|conn| conn.as_ref()).is_none() {
            return Engine::emit_ast("'()").unwrap().into_iter().next().unwrap();
        }

        // Lookup the node for the given address.
        let env = ctx.env();
        let node = &self.0;

        // Validate the node (must be stateless and non-branching).
        if node.stateful(env) {
            panic!("nodes used as functions must be stateless");
        }
        if !node.branches(env).is_empty() {
            panic!("nodes used as functions must not branch");
        }
        let n_outputs = node.n_outputs(env);
        if n_outputs != 1 {
            panic!("nodes used as functions must have a single output");
        }

        // Generate the node's expression with a placeholder path.
        let n_inputs = node.n_inputs(&env);
        let params: Vec<_> = (0..n_inputs).map(|i| format!("arg{i}")).collect();
        let input_refs: Vec<Option<String>> = params.iter().map(|p| Some(p.clone())).collect();
        let outputs = node::Conns::connected(n_outputs).unwrap();
        let path = ctx.path();
        let ectx = node::ExprCtx::new(env, path, &input_refs, &outputs);
        let expr = node.expr(ectx);

        // Create the lambda that we'll return.
        let expr_str = expr.to_pretty(80);
        let params_str = params.join(" ");
        let lambda_expr = format!("(lambda ({}) {})", params_str, expr_str);
        Engine::emit_ast(&lambda_expr)
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
    }

    fn visit(&self, ctx: visit::Ctx<Env>, visitor: &mut dyn node::Visitor<Env>) {
        self.0.visit(ctx, visitor);
    }
}

impl<N: gantz_ca::CaHash> gantz_ca::CaHash for Fn<N> {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        "gantz_core::node::Fn".hash(hasher);
        self.0.hash(hasher);
    }
}
