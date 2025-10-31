//! A suite of widgets, nodes and implementations for creating a GUI around
//! gantz using `egui`.

use std::hash::{Hash, Hasher};
use steel::{
    SteelErr, SteelVal,
    rvals::{FromSteelVal, IntoSteelVal},
    steel_vm::engine::Engine,
};

mod impls;
pub mod node;
pub mod widget;

/// A trait providing an egui `Ui` implementation for gantz nodes.
pub trait NodeUi<Env> {
    /// The name used to present the node within the inspector.
    fn name(&self, _env: &Env) -> &str;

    /// Instantiate the `Ui` for the given node.
    ///
    /// The node's path into the state tree and the VM are provided to allow for
    /// access to the node's state.
    fn ui(&mut self, _ctx: NodeCtx<Env>, _ui: &mut egui::Ui) -> egui::Response;

    /// Optionally add additional rows to the node's inspector UI.
    ///
    /// By default, only the node's path and its current state within the VM are
    /// shown. Adding to the given `body` by providing an implementation of this
    /// method will append extra rows.
    fn inspector_rows(&mut self, _ctx: &NodeCtx<Env>, _body: &mut egui_extras::TableBody) {}

    /// Extra UI for the node to be presented within the node inspector
    /// following the default table.
    ///
    /// See [`NodeUi::inspector_rows`] for how to simply append rows to the
    /// table.
    fn inspector_ui(&mut self, _ctx: NodeCtx<Env>, _ui: &mut egui::Ui) -> Option<egui::Response> {
        None
    }

    /// The layout direction of the node's inputs to outputs.
    fn flow(&self, _env: &Env) -> egui::Direction {
        egui::Direction::TopDown
    }
}

/// A wrapper around a node's path and the VM providing easy access to the
/// node's state.
pub struct NodeCtx<'a, Env> {
    env: &'a Env,
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

/// Used to represent the content address
///
/// TODO: Replace this with [u8; 20] and use SHA1 or something.
pub type ContentAddr = u64;

impl<'a, Env, N> NodeUi<Env> for &'a mut N
where
    N: ?Sized + NodeUi<Env>,
{
    fn name(&self, env: &Env) -> &str {
        (**self).name(env)
    }

    fn ui(&mut self, ctx: NodeCtx<Env>, ui: &mut egui::Ui) -> egui::Response {
        (**self).ui(ctx, ui)
    }

    fn inspector_rows(&mut self, ctx: &NodeCtx<Env>, body: &mut egui_extras::TableBody) {
        (**self).inspector_rows(ctx, body)
    }

    fn inspector_ui(&mut self, ctx: NodeCtx<Env>, ui: &mut egui::Ui) -> Option<egui::Response> {
        (**self).inspector_ui(ctx, ui)
    }

    fn flow(&self, env: &Env) -> egui::Direction {
        (**self).flow(env)
    }
}

macro_rules! impl_node_ui_for_ptr {
    ($($Ty:ident)::*) => {
        impl<Env, T> NodeUi<Env> for $($Ty)::*<T>
        where
            T: ?Sized + NodeUi<Env>,
        {
            fn name(&self, env: &Env) -> &str {
                (**self).name(env)
            }

            fn ui(&mut self, ctx: NodeCtx<Env>, ui: &mut egui::Ui) -> egui::Response {
                (**self).ui(ctx, ui)
            }

            fn inspector_rows(&mut self, ctx: &NodeCtx<Env>, body: &mut egui_extras::TableBody) {
                (**self).inspector_rows(ctx, body)
            }

            fn inspector_ui(&mut self, ctx: NodeCtx<Env>, ui: &mut egui::Ui) -> Option<egui::Response> {
                (**self).inspector_ui(ctx, ui)
            }

            fn flow(&self, env: &Env) -> egui::Direction {
                (**self).flow(env)
            }
        }
    };
}

impl_node_ui_for_ptr!(Box);

impl<'a, Env> NodeCtx<'a, Env> {
    pub fn new(
        env: &'a Env,
        path: &'a [node::Id],
        vm: &'a mut Engine,
        cmds: &'a mut Vec<Cmd>,
    ) -> Self {
        Self {
            env,
            path,
            vm,
            cmds,
        }
    }

    /// Provide access to the node's input environment.
    pub fn env(&self) -> &Env {
        self.env
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

/// Produce the content address for a given graph.
pub fn graph_content_addr<N>(g: &gantz_core::node::graph::Graph<N>) -> ContentAddr
where
    N: Hash,
{
    // TODO: Use a more stable/reproducible hash method.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    gantz_core::graph::hash(g, &mut hasher);
    hasher.finish()
}

/// Format the given content address into a full-length hex string.
pub fn fmt_content_addr(ca: ContentAddr) -> String {
    format!("{ca:#018x}")
}

/// Format the given content address into a shorthand 8-char hex string.
pub fn fmt_content_addr_short(ca: ContentAddr) -> String {
    let mut s = format!("{ca:08x}");
    s.truncate(8);
    s
}
