#[doc(inline)]
use gantz_core::node;
use steel::{steel_vm::engine::Engine, SteelErr, SteelVal};

mod impls;

/// A trait providing an egui `Ui` implementation for gantz nodes.
pub trait NodeUi {
    /// Instantiate the `Ui` for the given node.
    ///
    /// The node's path into the state tree and the VM are provided to allow for
    /// access to the node's state.
    fn ui(&mut self, _ctx: NodeCtx, _ui: &mut egui::Ui) -> egui::Response;
}

/// A wrapper around a node's path and the VM providing easy access to the
/// node's state.
pub struct NodeCtx<'a> {
    path: &'a [node::Id],
    vm: &'a mut Engine,
}

impl<'a, N> NodeUi for &'a mut N
where
    N: ?Sized + NodeUi,
{
    fn ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        (**self).ui(ctx, ui)
    }
}

impl<'a> NodeCtx<'a> {
    pub fn new(path: &'a [node::Id], vm: &'a mut Engine) -> Self {
        Self { path, vm }
    }

    /// The node's full path into the state tree.
    pub fn path(&self) -> &[node::Id] {
        &self.path
    }

    /// Read-only access to the VM.
    pub fn vm(&self) -> &Engine {
        &*self.vm
    }

    /// Extract the node's state from the VM.
    pub fn extract(&self) -> Option<SteelVal> {
        // TODO: Add this once `extract_value` is changed to take a node path.
        // node::state::extract_value(self.vm, self.path)
        todo!()

    }

    /// Register the given value as the node's new state.
    pub fn register(&mut self, val: SteelVal) -> Result<(), SteelErr> {
        node::state::register_value(self.vm, self.path, val)
    }
}
