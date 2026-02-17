use super::{Deserialize, Serialize};
use crate::node::{self, Node};
use gantz_ca::CaHash;

/// A wrapper around a `Node` that enables pull evaluation across all inputs.
///
/// The implementation of `Node` will match the inner node type `N`, but with a
/// unique implementation of [`Node::pull_eval`].
#[derive(Clone, Debug, Deserialize, Serialize, CaHash)]
#[cahash("gantz.pull")]
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

impl<N: Node> Pull<N> {
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

impl<N: Node> WithPullEval for N {
    /// Consume `self` and return an equivalent node with pull evaluation
    /// enabled.
    fn with_pull_eval_conf(self, conf: node::EvalConf) -> Pull<Self> {
        Pull::new(self, conf)
    }
}

impl<N: Node> Node for Pull<N> {
    fn n_inputs(&self, ctx: node::MetaCtx) -> usize {
        self.node.n_inputs(ctx)
    }

    fn n_outputs(&self, ctx: node::MetaCtx) -> usize {
        self.node.n_outputs(ctx)
    }

    fn branches(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        self.node.branches(ctx)
    }

    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        self.node.expr(ctx)
    }

    fn push_eval(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        self.node.push_eval(ctx)
    }

    fn pull_eval(&self, _ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        vec![self.conf.clone()]
    }

    fn inlet(&self, ctx: node::MetaCtx) -> bool {
        self.node.inlet(ctx)
    }

    fn outlet(&self, ctx: node::MetaCtx) -> bool {
        self.node.outlet(ctx)
    }

    fn stateful(&self, ctx: node::MetaCtx) -> bool {
        self.node.stateful(ctx)
    }

    fn register(&self, ctx: node::RegCtx<'_, '_>) {
        self.node.register(ctx)
    }

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        self.node.required_addrs()
    }
}
