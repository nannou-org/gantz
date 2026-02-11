use super::{Deserialize, Serialize};
use crate::node::{self, Node};
use gantz_ca::CaHash;

/// A wrapper around a `Node` that enables push evaluation across all outputs.
///
/// The implementation of `Node` will match the inner node type `N`, but with a
/// unique implementation of [`Node::push_eval`].
#[derive(Clone, Debug, Deserialize, Serialize, CaHash)]
#[cahash("gantz.push")]
pub struct Push<N> {
    node: N,
    conf: node::EvalConf,
}

/// A trait implemented for all `Node` types allowing to enable push evaluation.
pub trait WithPushEval: Sized + Node {
    /// Consume `self` and return a `Node` that has push evaluation enabled.
    fn with_push_eval_conf(self, conf: node::EvalConf) -> Push<Self>;
    /// Consume `self` and return a `Node` that has push evaluation enabled.
    fn with_push_eval(self) -> Push<Self> {
        self.with_push_eval_conf(node::EvalConf::All)
    }
}

impl<N: Node> Push<N> {
    /// Given some node, return a `Push` node enabling push evaluation across
    /// all outputs.
    pub fn all(node: N) -> Self {
        Push::new(node, node::EvalConf::All)
    }

    /// Given some node, return a `Push` node enabling push evaluation across
    /// some subset of the outputs.
    pub fn new(node: N, conf: node::EvalConf) -> Self {
        Push { node, conf }
    }
}

impl<N: Node> WithPushEval for N {
    fn with_push_eval_conf(self, conf: node::EvalConf) -> Push<Self> {
        Push::new(self, conf)
    }
}

impl<N: Node> Node for Push<N> {
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

    fn push_eval(&self, _ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        vec![self.conf.clone()]
    }

    fn pull_eval(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        self.node.pull_eval(ctx)
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
