use crate::node::{self, Node};
use steel::steel_vm::engine::Engine;

/// For types used to traverse nested graphs of [`Node`]s.
pub trait Visitor {
    /// Called prior to traversing nested nodes.
    fn visit_pre(&mut self, _node: &dyn Node, _path: &[node::Id]) {}
    /// Called following traversal of nested nodes.
    fn visit_post(&mut self, _node: &dyn Node, _path: &[node::Id]) {}
}

/// A type used for registering all nodes in a [`Visitor`] traversal.
///
/// Can be used via:
///
/// - `gantz_core::node::register`
/// - `gantz_core::graph::register`
pub(crate) struct Register<'vm>(pub(crate) &'vm mut Engine);

/// The `Register` visitor just calls `register` for each node, prior to
/// traversing its nested nodes.
impl<'vm> Visitor for Register<'vm> {
    fn visit_pre(&mut self, node: &dyn Node, path: &[node::Id]) {
        node.register(path, self.0);
    }
}
