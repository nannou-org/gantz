use super::{Deserialize, Serialize};
use crate::node::{self, Node};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// A wrapper around a `Node` that enables push evaluation.
///
/// The implementation of `Node` will match the inner node type `N`, but with a
/// unique implementation of `Node::push_eval`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Push<N> {
    node: N,
    push_eval: node::EvalFn,
}

/// A trait implemented for all `Node` types allowing to enable push evaluation.
pub trait WithPushEval: Sized + Node {
    /// Consume `self` and return a `Node` that has push evaluation enabled.
    fn with_push_eval(self, push_eval: node::EvalFn) -> Push<Self>;

    /// Enable push evaluation by generating a function with the given name.
    fn with_push_eval_name(self, name: impl Into<String>) -> Push<Self> {
        let eval_fn = node::EvalFn { name: name.into() };
        self.with_push_eval(eval_fn)
    }
}

impl<N> Push<N>
where
    N: Node,
{
    /// Given some node, return a `Push` node enabling push evaluation.
    pub fn new(node: N, push_eval: node::EvalFn) -> Self {
        Push { node, push_eval }
    }
}

impl<N> WithPushEval for N
where
    N: Node,
{
    fn with_push_eval(self, push_eval: node::EvalFn) -> Push<Self> {
        Push::new(self, push_eval)
    }
}

impl<N> Node for Push<N>
where
    N: Node,
{
    fn n_inputs(&self) -> usize {
        self.node.n_inputs()
    }

    fn n_outputs(&self) -> usize {
        self.node.n_outputs()
    }

    fn expr(&self, inputs: &[Option<ExprKind>]) -> ExprKind {
        self.node.expr(inputs)
    }

    fn push_eval(&self) -> Option<node::EvalFn> {
        Some(self.push_eval.clone())
    }

    fn pull_eval(&self) -> Option<node::EvalFn> {
        self.node.pull_eval()
    }

    fn register_state(&self, path: &[node::Id], vm: &mut Engine) {
        self.node.register_state(path, vm)
    }
}
