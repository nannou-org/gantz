//! Items related to traversing nested graphs of gantz nodes.

use crate::{
    Edge,
    node::{self, Node},
};
use std::collections::HashSet;
use steel::steel_vm::engine::Engine;

/// For types used to traverse nested graphs of [`Node`]s.
///
/// This is used for both node state registration and code generation.
pub trait Visitor {
    /// Called prior to traversing nested nodes.
    fn visit_pre(&mut self, _ctx: Ctx<'_, '_>, _node: &dyn Node) {}
    /// Called following traversal of nested nodes.
    fn visit_post(&mut self, _ctx: Ctx<'_, '_>, _node: &dyn Node) {}
}

/// The context provided for each node during the traversal.
#[derive(Clone, Copy)]
pub struct Ctx<'env, 'data> {
    /// Function for looking up nodes by content address.
    get_node: node::GetNode<'env>,
    /// The path at which this node is nested relative to the root.
    path: &'data [node::Id],
    /// A slice with an element for every input, `Some` if connected.
    inputs: &'data [(node::Id, Edge)],
}

/// A type used for registering all nodes in a [`Visitor`] traversal.
///
/// Can be used via:
///
/// - `gantz_core::node::register`
/// - `gantz_core::graph::register`
pub(crate) struct Register<'vm>(pub(crate) &'vm mut Engine);

/// Visitor that collects all required content addresses from nodes.
pub(crate) struct RequiredAddrs<'a> {
    /// The set of collected addresses.
    pub addrs: &'a mut HashSet<gantz_ca::ContentAddr>,
}

impl<'env, 'data> Ctx<'env, 'data> {
    /// Create a `Ctx` instance. Exclusively for use by `Visitor`
    /// implementations.
    pub fn new(
        get_node: node::GetNode<'env>,
        path: &'data [node::Id],
        inputs: &'data [(node::Id, Edge)],
    ) -> Self {
        Self {
            get_node,
            path,
            inputs,
        }
    }

    /// Look up a node by content address.
    pub fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&'env dyn Node> {
        (self.get_node)(ca)
    }

    /// The path at which this node is nested relative to the root.
    pub fn path(&self) -> &'data [node::Id] {
        self.path
    }

    /// The ID associated with this node within its graph
    ///
    /// This is equivalent to the last element of the path.
    pub fn id(&self) -> node::Id {
        *self.path.last().expect("path cannot be empty")
    }

    /// A slice with an element for every input, `Some` if connected.
    pub fn inputs(&self) -> &'data [(node::Id, Edge)] {
        self.inputs
    }

    /// Access to the node lookup function.
    pub fn get_node(&self) -> node::GetNode<'env> {
        self.get_node
    }
}

/// The `Register` visitor just calls `register` for each node, prior to
/// traversing its nested nodes.
impl Visitor for Register<'_> {
    fn visit_pre(&mut self, ctx: Ctx<'_, '_>, node: &dyn Node) {
        let reg_ctx = node::RegCtx::new(ctx.get_node(), ctx.path(), self.0);
        node.register(reg_ctx);
    }
}

impl Visitor for RequiredAddrs<'_> {
    fn visit_pre(&mut self, _ctx: Ctx<'_, '_>, node: &dyn Node) {
        self.addrs.extend(node.required_addrs());
    }
}
