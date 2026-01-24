//! A node that looks up its own implementation via content address.

use crate::{
    node::{self, Node},
    visit,
};
use serde::{Deserialize, Serialize};
use steel::{parser::ast::ExprKind, steel_vm::engine::Engine};

/// A node that refers to another node in the environment by content address.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Ref(gantz_ca::ContentAddr);

/// A registry of `Node`s, used by the [`Ref`] node to lookup a node by it's
/// content address.
pub trait NodeRegistry {
    /// The node type.
    type Node: ?Sized;
    /// Returns a node for the given node address.
    fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&Self::Node>;
}

impl Ref {
    /// Create a new [`Ref`] node that references the node at the given address.
    pub fn new(addr: gantz_ca::ContentAddr) -> Self {
        Self(addr)
    }

    /// The content address of the referenced node.
    pub fn content_addr(&self) -> gantz_ca::ContentAddr {
        self.0
    }
}

impl gantz_ca::CaHash for Ref {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        hasher.update("gantz_core::node::Ref".as_bytes());
        hasher.update(&self.0.0);
    }
}

impl<Env> Node<Env> for Ref
where
    Env: NodeRegistry,
    Env::Node: node::Node<Env>,
{
    fn n_inputs(&self, env: &Env) -> usize {
        env.node(&self.0).map(|n| n.n_inputs(env)).unwrap_or(0)
    }

    fn n_outputs(&self, env: &Env) -> usize {
        env.node(&self.0).map(|n| n.n_outputs(env)).unwrap_or(0)
    }

    fn branches(&self, env: &Env) -> Vec<node::EvalConf> {
        env.node(&self.0)
            .map(|n| n.branches(env))
            .unwrap_or_default()
    }

    fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
        let ctx2 = ctx.clone();
        ctx.env()
            .node(&self.0)
            .map(|n| n.expr(ctx2))
            .unwrap_or_default()
    }

    fn push_eval(&self, env: &Env) -> Vec<node::EvalConf> {
        env.node(&self.0)
            .map(|n| n.push_eval(env))
            .unwrap_or_default()
    }

    fn pull_eval(&self, env: &Env) -> Vec<node::EvalConf> {
        env.node(&self.0)
            .map(|n| n.pull_eval(env))
            .unwrap_or_default()
    }

    fn stateful(&self, env: &Env) -> bool {
        env.node(&self.0).map(|n| n.stateful(env)).unwrap_or(false)
    }

    fn register(&self, env: &Env, path: &[node::Id], vm: &mut Engine) {
        if let Some(n) = env.node(&self.0) {
            n.register(env, path, vm);
        }
    }

    fn inlet(&self, env: &Env) -> bool {
        env.node(&self.0).map(|n| n.inlet(env)).unwrap_or(false)
    }

    fn outlet(&self, env: &Env) -> bool {
        env.node(&self.0).map(|n| n.outlet(env)).unwrap_or(false)
    }

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        vec![self.0]
    }

    fn visit(&self, ctx: visit::Ctx<Env>, visitor: &mut dyn node::Visitor<Env>) {
        if let Some(n) = ctx.env().node(&self.0) {
            n.visit(ctx, visitor);
        }
    }
}
