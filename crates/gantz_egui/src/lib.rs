#[doc(inline)]
use gantz_core::node;
use steel::{
    SteelErr, SteelVal,
    rvals::{FromSteelVal, IntoSteelVal},
    steel_vm::engine::Engine,
};

mod impls;
pub mod widget;

/// A trait providing an egui `Ui` implementation for gantz nodes.
pub trait NodeUi {
    /// The name used to present the node within the inspector.
    fn name(&self) -> &str;

    /// Instantiate the `Ui` for the given node.
    ///
    /// The node's path into the state tree and the VM are provided to allow for
    /// access to the node's state.
    fn ui(&mut self, _ctx: NodeCtx, _ui: &mut egui::Ui) -> egui::Response;

    /// UI for the node to be presented within the node inspector.
    ///
    /// By default, this presents the node's path and its current state within
    /// the VM.
    fn inspector_ui(&mut self, _ctx: NodeCtx, _ui: &mut egui::Ui) -> Option<egui::Response> {
        None
    }

    /// The layout direction of the node's inputs to outputs.
    fn flow(&self) -> egui::Direction {
        egui::Direction::TopDown
    }
}

/// A wrapper around a node's path and the VM providing easy access to the
/// node's state.
pub struct NodeCtx<'a> {
    path: &'a [node::Id],
    vm: &'a mut Engine,
    cmds: &'a mut Vec<Cmd>,
}

/// Commands that can be emitted by nodes that are processed after the GUI pass
/// is complete.
#[derive(Debug)]
pub enum Cmd {
    PushEval(Vec<node::Id>),
    PullEval(Vec<node::Id>),
    OpenGraph(Vec<node::Id>),
}

impl<'a, N> NodeUi for &'a mut N
where
    N: ?Sized + NodeUi,
{
    fn name(&self) -> &str {
        (**self).name()
    }

    fn ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        (**self).ui(ctx, ui)
    }

    fn inspector_ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> Option<egui::Response> {
        (**self).inspector_ui(ctx, ui)
    }
}

macro_rules! impl_node_ui_for_ptr {
    ($($Ty:ident)::*) => {
        impl<T> NodeUi for $($Ty)::*<T>
        where
            T: ?Sized + NodeUi,
        {
            fn name(&self) -> &str {
                (**self).name()
            }

            fn ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
                (**self).ui(ctx, ui)
            }

            fn inspector_ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> Option<egui::Response> {
                (**self).inspector_ui(ctx, ui)
            }
        }
    };
}

impl_node_ui_for_ptr!(Box);

impl<'a> NodeCtx<'a> {
    pub fn new(path: &'a [node::Id], vm: &'a mut Engine, cmds: &'a mut Vec<Cmd>) -> Self {
        Self { path, vm, cmds }
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
    pub fn extract_value(&self) -> Result<Option<SteelVal>, SteelErr> {
        node::state::extract_value(self.vm, self.path)
    }

    /// Extract and unwrap the node's unique state from the VM.
    pub fn extract<T: FromSteelVal>(&self) -> Result<Option<T>, SteelErr> {
        node::state::extract(self.vm, self.path)
    }

    /// Register the given value as the node's new state.
    pub fn update_value(&mut self, val: SteelVal) -> Result<(), SteelErr> {
        node::state::update_value(self.vm, self.path, val)
    }

    /// Register the given value as the node's new state.
    pub fn update<T: IntoSteelVal>(&mut self, val: T) -> Result<(), SteelErr> {
        node::state::update(self.vm, self.path, val)
    }

    /// Queue a call to the generated push evaluation function for this node.
    ///
    /// This will only be successful if the underlying node's
    /// [`gantz_core::Node::push_eval`] fn returned `Some` last time the graph
    /// was compiled.
    pub fn push_eval(&mut self) {
        self.cmds.push(Cmd::PushEval(self.path.to_vec()));
    }

    /// Queue a call to the generated pull evaluation function for this node.
    ///
    /// This will only be successful if the underlying node's
    /// [`gantz_core::Node::pull_eval`] fn returned `Some` last time the graph
    /// was compiled.
    pub fn pull_eval(&mut self) {
        self.cmds.push(Cmd::PullEval(self.path.to_vec()));
    }
}
