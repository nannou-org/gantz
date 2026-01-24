use super::{Deserialize, Serialize};
use crate::node::{self, Node};
use gantz_ca::CaHash;
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// A wrapper around a `Node` that enables pull evaluation across all inputs.
///
/// The implementation of `Node` will match the inner node type `N`, but with a
/// unique implementation of [`Node::pull_eval`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Pull<Env, N> {
    env: core::marker::PhantomData<Env>,
    node: N,
    conf: node::EvalConf,
}

/// A trait implemented for all `Node` types allowing to enable pull evaluation.
pub trait WithPullEval<Env>: Sized + Node<Env> {
    /// Consume `self` and return a `Node` that has push evaluation enabled.
    fn with_pull_eval_conf(self, conf: node::EvalConf) -> Pull<Env, Self>;
    /// Consume `self` and return a `Node` that has pull evaluation enabled.
    fn with_pull_eval(self) -> Pull<Env, Self> {
        self.with_pull_eval_conf(node::EvalConf::All)
    }
}

impl<Env, N> Pull<Env, N>
where
    N: Node<Env>,
{
    /// Given some node, return a `Pull` node enabling pull evaluation across
    /// all outputs.
    pub fn all(node: N) -> Self {
        Pull::new(node, node::EvalConf::All)
    }

    /// Given some node, return a `Pull` node enabling pull evaluation across
    /// some subset of the outputs.
    pub fn new(node: N, conf: node::EvalConf) -> Self {
        let env = core::marker::PhantomData;
        Pull { env, node, conf }
    }
}

impl<Env, N> WithPullEval<Env> for N
where
    N: Node<Env>,
{
    /// Consume `self` and return an equivalent node with pull evaluation
    /// enabled.
    fn with_pull_eval_conf(self, conf: node::EvalConf) -> Pull<Env, Self> {
        Pull::new(self, conf)
    }
}

impl<Env, N> Node<Env> for Pull<Env, N>
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

    fn push_eval(&self, env: &Env) -> Vec<node::EvalConf> {
        self.node.push_eval(env)
    }

    fn pull_eval(&self, _env: &Env) -> Vec<node::EvalConf> {
        vec![self.conf.clone()]
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

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        self.node.required_addrs()
    }
}

impl<Env, N> CaHash for Pull<Env, N>
where
    N: CaHash,
{
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        "Pull".hash(hasher);
        self.conf.hash(hasher);
        self.node.hash(hasher);
    }
}
