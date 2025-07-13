use crate::{
    Edge,
    node::{self, Node},
};
use steel::steel_vm::engine::Engine;

/// For types used to traverse nested graphs of [`Node`]s.
///
/// This is used for both node state registration and code generation.
pub trait Visitor {
    /// Called prior to traversing nested nodes.
    fn visit_pre(&mut self, _ctx: Ctx, _node: &dyn Node) {}
    /// Called following traversal of nested nodes.
    fn visit_post(&mut self, _ctx: Ctx, _node: &dyn Node) {}
}

/// The context provided for each node during the traversal.
#[derive(Clone, Copy, Debug)]
pub struct Ctx<'a> {
    /// The path at which this node is nested relative to the root.
    path: &'a [node::Id],
    /// A slice with an element for every input, `Some` if connected.
    inputs: &'a [(node::Id, Edge)],
}

/// A type used for registering all nodes in a [`Visitor`] traversal.
///
/// Can be used via:
///
/// - `gantz_core::node::register`
/// - `gantz_core::graph::register`
pub(crate) struct Register<'vm>(pub(crate) &'vm mut Engine);

impl<'a> Ctx<'a> {
    /// Create a `Ctx` instance. Exclusively for use by `Visitor`
    /// implementations.
    pub fn new(path: &'a [node::Id], inputs: &'a [(node::Id, Edge)]) -> Self {
        Self { path, inputs }
    }

    /// The path at which this node is nested relative to the root.
    pub fn path(&self) -> &[node::Id] {
        self.path
    }

    /// The ID associated with this node within its graph
    ///
    /// This is equivalent to the last element of the path.
    pub fn id(&self) -> node::Id {
        *self.path.last().expect("path cannot be empty")
    }

    /// A slice with an element for every input, `Some` if connected.
    pub fn inputs(&self) -> &[(node::Id, Edge)] {
        self.inputs
    }
}

/// The `Register` visitor just calls `register` for each node, prior to
/// traversing its nested nodes.
impl<'vm> Visitor for Register<'vm> {
    fn visit_pre(&mut self, ctx: Ctx, node: &dyn Node) {
        node.register(ctx.path(), self.0);
    }
}
