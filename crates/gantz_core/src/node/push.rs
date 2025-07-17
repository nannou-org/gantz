use super::{Deserialize, Serialize};
use crate::node::{self, Node};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// A wrapper around a `Node` that enables push evaluation across all outputs.
///
/// The implementation of `Node` will match the inner node type `N`, but with a
/// unique implementation of [`Node::push_eval`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Push<N> {
    node: N,
    set: node::PushEval,
}

/// A trait implemented for all `Node` types allowing to enable push evaluation.
pub trait WithPushEval: Sized + Node {
    /// Consume `self` and return a `Node` that has push evaluation enabled.
    fn with_push_eval_set(self, set: node::PushEval) -> Push<Self>;
    /// Consume `self` and return a `Node` that has push evaluation enabled.
    fn with_push_eval(self) -> Push<Self> {
        self.with_push_eval_set(node::PushEval::All)
    }
}

impl<N> Push<N>
where
    N: Node,
{
    /// Given some node, return a `Push` node enabling push evaluation across
    /// all outputs.
    pub fn all(node: N) -> Self {
        Push::set(node, node::PushEval::All)
    }

    /// Given some node, return a `Push` node enabling push evaluation across
    /// some subset of the outputs.
    pub fn set(node: N, set: node::PushEval) -> Self {
        Push { node, set }
    }
}

impl<N> WithPushEval for N
where
    N: Node,
{
    fn with_push_eval_set(self, set: node::EvalSet) -> Push<Self> {
        Push::set(self, set)
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

    fn expr(&self, ctx: node::ExprCtx) -> ExprKind {
        self.node.expr(ctx)
    }

    fn push_eval(&self) -> Vec<node::PushEval> {
        vec![self.set.clone()]
    }

    fn pull_eval(&self) -> Vec<node::PullEval> {
        self.node.pull_eval()
    }

    fn inlet(&self) -> bool {
        self.node.inlet()
    }

    fn outlet(&self) -> bool {
        self.node.outlet()
    }

    fn stateful(&self) -> bool {
        self.node.stateful()
    }

    fn register(&self, path: &[node::Id], vm: &mut Engine) {
        self.node.register(path, vm)
    }
}
