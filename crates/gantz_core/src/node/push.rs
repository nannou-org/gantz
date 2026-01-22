use super::{Deserialize, Serialize};
use crate::node::{self, Node};
use gantz_ca::CaHash;
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// A wrapper around a `Node` that enables push evaluation across all outputs.
///
/// The implementation of `Node` will match the inner node type `N`, but with a
/// unique implementation of [`Node::push_eval`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Push<Env, N> {
    env: core::marker::PhantomData<Env>,
    node: N,
    conf: node::EvalConf,
}

/// A trait implemented for all `Node` types allowing to enable push evaluation.
pub trait WithPushEval<Env>: Sized + Node<Env> {
    /// Consume `self` and return a `Node` that has push evaluation enabled.
    fn with_push_eval_conf(self, conf: node::EvalConf) -> Push<Env, Self>;
    /// Consume `self` and return a `Node` that has push evaluation enabled.
    fn with_push_eval(self) -> Push<Env, Self> {
        self.with_push_eval_conf(node::EvalConf::All)
    }
}

impl<Env, N> Push<Env, N>
where
    N: Node<Env>,
{
    /// Given some node, return a `Push` node enabling push evaluation across
    /// all outputs.
    pub fn all(node: N) -> Self {
        Push::new(node, node::EvalConf::All)
    }

    /// Given some node, return a `Push` node enabling push evaluation across
    /// some subset of the outputs.
    pub fn new(node: N, conf: node::EvalConf) -> Self {
        let env = core::marker::PhantomData;
        Push { env, node, conf }
    }
}

impl<Env, N> WithPushEval<Env> for N
where
    N: Node<Env>,
{
    fn with_push_eval_conf(self, conf: node::EvalConf) -> Push<Env, Self> {
        Push::new(self, conf)
    }
}

impl<Env, N> Node<Env> for Push<Env, N>
where
    N: Node<Env>,
{
    fn n_inputs(&self, env: &Env) -> usize {
        self.node.n_inputs(env)
    }

    fn n_outputs(&self, env: &Env) -> usize {
        self.node.n_outputs(env)
    }

    fn branches(&self, env: &Env) -> Vec<node::EvalConf> {
        self.node.branches(env)
    }

    fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
        self.node.expr(ctx)
    }

    fn push_eval(&self, _: &Env) -> Vec<node::EvalConf> {
        vec![self.conf.clone()]
    }

    fn pull_eval(&self, env: &Env) -> Vec<node::EvalConf> {
        self.node.pull_eval(env)
    }

    fn inlet(&self, env: &Env) -> bool {
        self.node.inlet(env)
    }

    fn outlet(&self, env: &Env) -> bool {
        self.node.outlet(env)
    }

    fn stateful(&self, env: &Env) -> bool {
        self.node.stateful(env)
    }

    fn register(&self, env: &Env, path: &[node::Id], vm: &mut Engine) {
        self.node.register(env, path, vm)
    }
}

impl<Env, N> CaHash for Push<Env, N>
where
    N: CaHash,
{
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        "Push".hash(hasher);
        self.conf.hash(hasher);
        self.node.hash(hasher);
    }
}
