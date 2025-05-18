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
}

/// A trait implemented for all `Node` types allowing to enable push evaluation.
pub trait WithPushEval: Sized + Node {
    /// Consume `self` and return a `Node` that has push evaluation enabled.
    fn with_push_eval(self) -> Push<Self>;
}

impl<N> Push<N>
where
    N: Node,
{
    /// Given some node, return a `Push` node enabling push evaluation.
    pub fn new(node: N) -> Self {
        Push { node }
    }
}

impl<N> WithPushEval for N
where
    N: Node,
{
    fn with_push_eval(self) -> Push<Self> {
        Push::new(self)
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
        Some(node::EvalFn)
    }

    fn pull_eval(&self) -> Option<node::EvalFn> {
        self.node.pull_eval()
    }

    fn stateful(&self) -> bool {
        self.node.stateful()
    }

    fn register(&self, path: &[node::Id], vm: &mut Engine) {
        self.node.register(path, vm)
    }
}
