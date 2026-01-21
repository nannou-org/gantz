//! A node that wraps another node as a first-class function value.

use crate::node;
use serde::{Deserialize, Serialize};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// A node that emits a lambda function wrapping another named node.
///
/// This node can wrap both primitive nodes and registry graphs, emitting them
/// as callable lambda functions rather than evaluating them directly.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Fn {
    /// The address of a registered node.
    pub addr: gantz_ca::ContentAddr,
}

/// A registry of `Node`s, used by the [`Fn`] node to lookup a node by it's CA.
pub trait NodeRegistry {
    /// The node type.
    type Node;
    /// Returns a node for the given node address.
    fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&Self::Node>;
}

impl Fn {
    /// Create a new Fn node that wraps the node at the given address.
    pub fn new(addr: gantz_ca::ContentAddr) -> Self {
        Self { addr }
    }
}

impl Default for Fn {
    fn default() -> Self {
        // Default to the identity function - a pure, stateless function
        let addr = gantz_ca::content_addr(&node::Identity);
        Self { addr }
    }
}

impl<Env> node::Node<Env> for Fn
where
    Env: NodeRegistry,
    Env::Node: node::Node<Env>,
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
    /// a lambda that wraps the inner node's expression and receives a number of
    /// arguments equal to the inner node's `Node::n_inputs` count.
    fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
        // Only emit a lambda in the case that some input is connected for
        // receiving bangs. Otherwise, we'll just emit an empty list.
        if ctx.inputs().get(0).and_then(|conn| conn.as_ref()).is_none() {
            return Engine::emit_ast("'()").unwrap().into_iter().next().unwrap();
        }

        // Lookup the node for the given address.
        let env = ctx.env();
        let node = env
            .node(&self.addr)
            .unwrap_or_else(|| panic!("no node found for address '{}'", self.addr));

        // Validate the node (must be stateless and non-branching).
        if node.stateful() {
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
}

impl gantz_ca::CaHash for Fn {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        "gantz_core::node::Fn".hash(hasher);
        self.addr.hash(hasher);
    }
}
