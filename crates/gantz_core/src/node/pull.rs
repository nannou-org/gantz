use super::{Deserialize, Serialize};
use crate::node::{self, Node};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// A wrapper around a `Node` that enables pull evaluation.
///
/// The implementation of `Node` will match the inner node type `N`, but with a
/// unique implementation of [`Node::pull_eval`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Pull<N> {
    node: N,
    pull_eval: node::EvalFn,
}

/// A trait implemented for all `Node` types allowing to enable pull evaluation.
pub trait WithPullEval: Sized + Node {
    /// Consume `self` and return a `Node` that has pull evaluation enabled.
    fn with_pull_eval(self, pull_eval: node::EvalFn) -> Pull<Self>;

    /// Enable pull evaluation by generating a function with the given name.
    fn with_pull_eval_name(self, name: impl Into<String>) -> Pull<Self> {
        let eval_fn = node::EvalFn { name: name.into() };
        self.with_pull_eval(eval_fn)
    }
}

impl<N> Pull<N>
where
    N: Node,
{
    /// Given some node, return a `Pull` node enabling pull evaluation.
    pub fn new(node: N, pull_eval: node::EvalFn) -> Self {
        Pull { node, pull_eval }
    }
}

impl<N> WithPullEval for N
where
    N: Node,
{
    /// Consume `self` and return an equivalent node with pull evaluation enabled.
    fn with_pull_eval(self, pull_eval: node::EvalFn) -> Pull<Self> {
        Pull::new(self, pull_eval)
    }
}

impl<N> Node for Pull<N>
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
        self.node.push_eval()
    }

    fn pull_eval(&self) -> Option<node::EvalFn> {
        Some(self.pull_eval.clone())
    }

    fn register_state(&self, path: &[node::Id], vm: &mut Engine) {
        self.node.register_state(path, vm)
    }
}
