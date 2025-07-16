use super::{Deserialize, Serialize};
use crate::node::{self, Node};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// A wrapper around a `Node` that enables pull evaluation across all inputs.
///
/// The implementation of `Node` will match the inner node type `N`, but with a
/// unique implementation of [`Node::pull_eval`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Pull<N> {
    node: N,
}

/// A trait implemented for all `Node` types allowing to enable pull evaluation.
pub trait WithPullEval: Sized + Node {
    /// Consume `self` and return a `Node` that has pull evaluation enabled.
    fn with_pull_eval(self) -> Pull<Self>;
}

impl<N> Pull<N>
where
    N: Node,
{
    /// Given some node, return a `Pull` node enabling pull evaluation.
    pub fn new(node: N) -> Self {
        Pull { node }
    }
}

impl<N> WithPullEval for N
where
    N: Node,
{
    /// Consume `self` and return an equivalent node with pull evaluation
    /// enabled.
    fn with_pull_eval(self) -> Pull<Self> {
        Pull::new(self)
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

    fn expr(&self, ctx: node::ExprCtx) -> ExprKind {
        self.node.expr(ctx)
    }

    fn push_eval(&self) -> Vec<node::PushEval> {
        self.node.push_eval()
    }

    fn pull_eval(&self) -> Vec<node::PullEval> {
        vec![node::PullEval::All]
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
