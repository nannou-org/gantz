//! A suite of widgets, nodes and implementations for creating a GUI around
//! gantz using `egui`.

use petgraph::visit::{IntoNodeReferences, NodeRef};
use std::collections::HashMap;
use steel::{
    SteelErr, SteelVal,
    rvals::{FromSteelVal, IntoSteelVal},
    steel_vm::engine::Engine,
};

mod impls;
pub mod node;
pub mod widget;

// Re-export traits that make up the Registry supertrait.
pub use node::{FnNodeNames, NameRegistry};

/// Combined registry trait for UI operations.
///
/// Provides node lookup, name resolution, and Fn-compatible node listing.
/// This supertrait combines [`NameRegistry`] and [`FnNodeNames`] with an
/// additional method for looking up nodes by content address.
pub trait Registry: NameRegistry + FnNodeNames {
    /// Look up a node by content address.
    fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&dyn gantz_core::Node>;
}

/// Provides access to open head data for the Gantz widget.
///
/// This trait abstracts over different storage strategies (Bevy entities,
/// parallel Vecs, etc.) allowing the widget to access head data without
/// requiring a specific storage layout.
pub trait HeadAccess {
    /// The node type used in graphs.
    type Node;

    /// Get the list of all head identifiers.
    fn heads(&self) -> &[gantz_ca::Head];

    /// Get mutable access to a specific head's data via a callback.
    ///
    /// Returns `None` if the head is not found.
    fn with_head_mut<R>(
        &mut self,
        head: &gantz_ca::Head,
        f: impl FnOnce(HeadDataMut<'_, Self::Node>) -> R,
    ) -> Option<R>;

    /// Get the compiled module string for a head.
    fn compiled_module(&self, head: &gantz_ca::Head) -> Option<&str>;
}

/// Mutable access to a head's data, provided via [`HeadAccess::with_head_mut`].
pub struct HeadDataMut<'a, N> {
    pub graph: &'a mut gantz_core::node::graph::Graph<N>,
    pub views: &'a mut GraphViews,
    pub vm: &'a mut Engine,
}

/// View state (layout + camera) for a graph and all its nested subgraphs, keyed by path.
pub type GraphViews = HashMap<Vec<node::Id>, egui_graph::View>;

/// A trait providing an egui `Ui` implementation for gantz nodes.
pub trait NodeUi {
    /// The name used to present the node within the inspector.
    fn name(&self, _registry: &dyn Registry) -> &str;

    /// Instantiate the `Ui` for the given node.
    ///
    /// The node's path into the state tree and the VM are provided to allow for
    /// access to the node's state. The egui_graph node context is provided to
    /// allow customizing the frame and other node display properties.
    fn ui(
        &mut self,
        ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response>;

    /// Optionally add additional rows to the node's inspector UI.
    ///
    /// By default, only the node's path and its current state within the VM are
    /// shown. Adding to the given `body` by providing an implementation of this
    /// method will append extra rows.
    fn inspector_rows(&mut self, _ctx: &mut NodeCtx, _body: &mut egui_extras::TableBody) {}

    /// Extra UI for the node to be presented within the node inspector
    /// following the default table.
    ///
    /// See [`NodeUi::inspector_rows`] for how to simply append rows to the
    /// table.
    fn inspector_ui(&mut self, _ctx: NodeCtx, _ui: &mut egui::Ui) -> Option<egui::Response> {
        None
    }

    /// The layout direction of the node's inputs to outputs.
    fn flow(&self, _registry: &dyn Registry) -> egui::Direction {
        egui::Direction::TopDown
    }
}

/// A wrapper around a node's path and the VM providing easy access to the
/// node's state.
pub struct NodeCtx<'a> {
    registry: &'a dyn Registry,
    path: &'a [node::Id],
    inlets: &'a [node::Id],
    outlets: &'a [node::Id],
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
    OpenNamedNode(String, gantz_ca::ContentAddr),
    /// Fork a named node: create new name pointing to the given content address.
    ForkNamedNode {
        new_name: String,
        ca: gantz_ca::ContentAddr,
    },
    /// Insert an inspect node on the given edge at the given position.
    InspectEdge(InspectEdge),
    /// Create a new node of the given type at the current path.
    CreateNode(CreateNode),
}

/// A command to create a new node.
#[derive(Clone, Debug)]
pub struct CreateNode {
    /// The path within the graph hierarchy where the node should be created.
    pub path: Vec<node::Id>,
    /// The type name of the node to create.
    pub node_type: String,
}

/// A command to insert an Inspect node on an edge.
#[derive(Clone, Debug)]
pub struct InspectEdge {
    pub path: Vec<node::Id>,
    pub edge: petgraph::graph::EdgeIndex<usize>,
    pub pos: egui::Pos2,
}

impl<'a, N> NodeUi for &'a mut N
where
    N: ?Sized + NodeUi,
{
    fn name(&self, registry: &dyn Registry) -> &str {
        (**self).name(registry)
    }

    fn ui(
        &mut self,
        ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        (**self).ui(ctx, uictx)
    }

    fn inspector_rows(&mut self, ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
        (**self).inspector_rows(ctx, body)
    }

    fn inspector_ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> Option<egui::Response> {
        (**self).inspector_ui(ctx, ui)
    }

    fn flow(&self, registry: &dyn Registry) -> egui::Direction {
        (**self).flow(registry)
    }
}

macro_rules! impl_node_ui_for_ptr {
    ($($Ty:ident)::*) => {
        impl<T> NodeUi for $($Ty)::*<T>
        where
            T: ?Sized + NodeUi,
        {
            fn name(&self, registry: &dyn Registry) -> &str {
                (**self).name(registry)
            }

            fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> egui::InnerResponse<egui::Response> {
                (**self).ui(ctx, uictx)
            }

            fn inspector_rows(&mut self, ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
                (**self).inspector_rows(ctx, body)
            }

            fn inspector_ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> Option<egui::Response> {
                (**self).inspector_ui(ctx, ui)
            }

            fn flow(&self, registry: &dyn Registry) -> egui::Direction {
                (**self).flow(registry)
            }
        }
    };
}

impl_node_ui_for_ptr!(Box);

impl<'a> NodeCtx<'a> {
    pub fn new(
        registry: &'a dyn Registry,
        path: &'a [node::Id],
        inlets: &'a [node::Id],
        outlets: &'a [node::Id],
        vm: &'a mut Engine,
        cmds: &'a mut Vec<Cmd>,
    ) -> Self {
        Self {
            registry,
            path,
            inlets,
            outlets,
            vm,
            cmds,
        }
    }

    /// Provide access to the registry.
    pub fn registry(&self) -> &dyn Registry {
        self.registry
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

    /// The IDs of the inlets within the current graph.
    ///
    /// Primarily exposed so that `Inlet` nodes can present their index.
    pub fn inlets(&self) -> &[node::Id] {
        self.inlets
    }

    /// The IDs of the outlets within the current graph.
    ///
    /// Primarily exposed so that `Outlet` nodes can present their index.
    pub fn outlets(&self) -> &[node::Id] {
        self.outlets
    }
}

/// The IDs of the inlet and outlet nodes.
pub(crate) fn inlet_outlet_ids<N>(
    registry: &dyn Registry,
    g: &gantz_core::node::graph::Graph<N>,
) -> (Vec<node::Id>, Vec<node::Id>)
where
    N: gantz_core::Node,
{
    let get_node = |ca: &gantz_ca::ContentAddr| registry.node(ca);
    let ctx = gantz_core::node::MetaCtx::new(&get_node);
    let mut inlets = vec![];
    let mut outlets = vec![];
    for n_ref in g.node_references() {
        if n_ref.weight().inlet(ctx) {
            inlets.push(n_ref.id().index());
        }
        if n_ref.weight().outlet(ctx) {
            outlets.push(n_ref.id().index());
        }
    }
    (inlets, outlets)
}

fn system_time_from_web(t: web_time::SystemTime) -> Option<std::time::SystemTime> {
    let duration = t.duration_since(web_time::UNIX_EPOCH).ok()?;
    std::time::UNIX_EPOCH.checked_add(duration)
}

/// Check if the given head is the currently focused head.
///
/// `focused_head` represents an index into the given `heads` iterator.
pub fn head_is_focused<'a>(
    heads: impl IntoIterator<Item = &'a gantz_ca::Head>,
    focused_head: usize,
    head: &gantz_ca::Head,
) -> bool {
    heads
        .into_iter()
        .position(|h| h == head)
        .map(|ix| ix == focused_head)
        .unwrap_or(false)
}
