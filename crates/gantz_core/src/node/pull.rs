use super::{Deserialize, Serialize};
use crate::node::{self, Node};
use steel::steel_vm::engine::Engine;

/// A wrapper around a `Node` that enables pull evaluation across all inputs.
///
/// The implementation of `Node` will match the inner node type `N`, but with a
/// unique implementation of [`Node::pull_eval`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Pull<N> {
    node: N,
    conf: node::EvalConf,
}

/// A trait implemented for all `Node` types allowing to enable pull evaluation.
pub trait WithPullEval: Sized + Node {
    /// Consume `self` and return a `Node` that has push evaluation enabled.
    fn with_pull_eval_conf(self, conf: node::EvalConf) -> Pull<Self>;
    /// Consume `self` and return a `Node` that has pull evaluation enabled.
    fn with_pull_eval(self) -> Pull<Self> {
        self.with_pull_eval_conf(node::EvalConf::All)
    }
}

impl<N> Pull<N>
where
    N: Node,
{
    /// Given some node, return a `Pull` node enabling pull evaluation across
    /// all outputs.
    pub fn all(node: N) -> Self {
        Pull::new(node, node::EvalConf::All)
    }

    /// Given some node, return a `Pull` node enabling pull evaluation across
    /// some subset of the outputs.
    pub fn new(node: N, conf: node::EvalConf) -> Self {
        Pull { node, conf }
    }
}

impl<N> WithPullEval for N
where
    N: Node,
{
    /// Consume `self` and return an equivalent node with pull evaluation
    /// enabled.
    fn with_pull_eval_conf(self, conf: node::EvalConf) -> Pull<Self> {
        Pull::new(self, conf)
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

    fn expr(&self, ctx: node::ExprCtx) -> node::NodeExpr {
        self.node.expr(ctx)
    }

    fn push_eval(&self) -> Vec<node::EvalConf> {
        self.node.push_eval()
    }

    fn pull_eval(&self) -> Vec<node::EvalConf> {
        vec![self.conf.clone()]
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
