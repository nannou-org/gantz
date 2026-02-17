//! A node that looks up its own implementation via content address.

use crate::{
    node::{self, Node},
    visit,
};
use gantz_ca::CaHash;
use serde::{Deserialize, Serialize};

/// A node that refers to another node in the environment by content address.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.ref")]
pub struct Ref(gantz_ca::ContentAddr);

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

impl Node for Ref {
    fn n_inputs(&self, ctx: node::MetaCtx) -> usize {
        ctx.node(&self.0).map(|n| n.n_inputs(ctx)).unwrap_or(0)
    }

    fn n_outputs(&self, ctx: node::MetaCtx) -> usize {
        ctx.node(&self.0).map(|n| n.n_outputs(ctx)).unwrap_or(0)
    }

    fn branches(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        ctx.node(&self.0)
            .map(|n| n.branches(ctx))
            .unwrap_or_default()
    }

    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        match ctx.node(&self.0) {
            Some(n) => n.expr(ctx),
            None => Err(node::ExprError::custom(format!(
                "node not found for address {:?}",
                self.0
            ))),
        }
    }

    fn push_eval(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        ctx.node(&self.0)
            .map(|n| n.push_eval(ctx))
            .unwrap_or_default()
    }

    fn pull_eval(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        ctx.node(&self.0)
            .map(|n| n.pull_eval(ctx))
            .unwrap_or_default()
    }

    fn stateful(&self, ctx: node::MetaCtx) -> bool {
        ctx.node(&self.0).map(|n| n.stateful(ctx)).unwrap_or(false)
    }

    fn register(&self, ctx: node::RegCtx<'_, '_>) {
        // Check if node exists first, then decompose context to pass to nested register.
        if ctx.node(&self.0).is_some() {
            let (get_node, path, vm) = ctx.into_parts();
            // Safe to unwrap since we checked above.
            let n = (get_node)(&self.0).unwrap();
            n.register(node::RegCtx::new(get_node, path, vm));
        }
    }

    fn inlet(&self, ctx: node::MetaCtx) -> bool {
        ctx.node(&self.0).map(|n| n.inlet(ctx)).unwrap_or(false)
    }

    fn outlet(&self, ctx: node::MetaCtx) -> bool {
        ctx.node(&self.0).map(|n| n.outlet(ctx)).unwrap_or(false)
    }

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        vec![self.0]
    }

    fn visit(&self, ctx: visit::Ctx<'_, '_>, visitor: &mut dyn node::Visitor) {
        if let Some(n) = ctx.node(&self.0) {
            n.visit(ctx, visitor);
        }
    }
}
