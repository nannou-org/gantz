//! A suite of widgets, nodes and implementations for creating a GUI around
//! gantz using `egui`.

use petgraph::visit::{IntoNodeReferences, NodeRef};
use std::borrow::Cow;
use std::collections::HashMap;
use steel::{
    SteelErr, SteelVal,
    rvals::{FromSteelVal, IntoSteelVal},
    steel_vm::engine::Engine,
};

pub mod export;
pub mod format;
mod impls;
pub mod node;
pub mod ops;
pub mod reg;
pub mod response;
pub mod sync;
pub mod widget;

// Re-export traits that make up the Registry supertrait.
pub use node::{FnNodeNames, NameRegistry};
pub use reg::RegistryRef;
pub use response::{DynResponse, ResponseData, Responses};
pub use widget::gantz::NodeTypeRegistry;
pub use widget::graph_select::GraphRegistry;

/// Combined registry trait for UI operations.
///
/// Brings together the different lookup capabilities required by the various
/// gantz_egui widgets. Each supertrait is defined alongside the widget that
/// requires it:
///
/// - [`NameRegistry`] (`node/named_ref.rs`) — resolves named references to
///   content addresses. Required by [`node::NamedRef`] to check whether a
///   referenced graph still exists and to display up-to-date status.
///
/// - [`FnNodeNames`] (`node/fn_named_ref.rs`) — lists names eligible for
///   use in `Fn`-style node references (stateless, branchless, single-output).
///   Required by [`node::FnNamedRef`]'s UI dropdown.
///
/// - [`NodeTypeRegistry`] (`widget/gantz.rs`) — enumerates all creatable node
///   types. Required by the command palette for node creation.
///
/// - [`GraphRegistry`] (`widget/graph_select.rs`) — provides access to commits
///   and branch names. Required by the graph selector and history view.
///
/// The `node` method provides direct node lookup by content address, used
/// throughout for resolving graph references during compilation, evaluation,
/// and UI rendering.
///
/// See [`reg::RegistryRef`] for the standard implementation combining a
/// [`gantz_ca::Registry`] with [`gantz_core::Builtins`].
pub trait Registry: NameRegistry + FnNodeNames + NodeTypeRegistry + GraphRegistry {
    /// Look up a node by content address.
    fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&dyn gantz_core::Node>;

    /// Get the demo graph name for a node at the given content address.
    fn demo_graph(&self, ca: &gantz_ca::ContentAddr) -> Option<&str> {
        let _ = ca;
        None
    }

    /// The inlet/outlet documentation associated with the graph at the given
    /// content address, if any.
    ///
    /// This is GUI-side metadata (authored in the inspector, stored separately
    /// from the graph's content like demos and views) rather than part of the
    /// node itself. It enables a referencing node (e.g. [`node::NamedRef`]) to
    /// surface the referenced graph's socket docs - see [`SocketDoc`].
    fn interface_docs(&self, ca: &gantz_ca::ContentAddr) -> Option<&InterfaceDocs> {
        let _ = ca;
        None
    }
}

/// On-hover documentation for a single node inlet or outlet.
///
/// `ty` is a short, free-form label for the expected/produced "type" (e.g.
/// `"number"`, `"function"`, `"bang"`, `"any"`). gantz values are dynamic Steel
/// values, so this is a human hint rather than a checked type. `description` is
/// an optional concise note for extra context.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SocketDoc {
    pub ty: Cow<'static, str>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<Cow<'static, str>>,
}

/// Per-graph inlet/outlet documentation, keyed by socket ordinal.
///
/// Stored as GUI side-metadata keyed by the graph's commit (mirroring `demos`
/// and views) so the core `Inlet`/`Outlet` nodes - and thus content addresses -
/// stay untouched.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct InterfaceDocs {
    #[serde(default)]
    pub inlets: HashMap<usize, SocketDoc>,
    #[serde(default)]
    pub outlets: HashMap<usize, SocketDoc>,
}

impl SocketDoc {
    /// A doc with just a type label and no description.
    pub fn ty(ty: impl Into<Cow<'static, str>>) -> Self {
        SocketDoc {
            ty: ty.into(),
            description: None,
        }
    }

    /// Attach a concise description.
    pub fn with_description(mut self, description: impl Into<Cow<'static, str>>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Whether this doc carries no content (empty type and no description).
    pub fn is_empty(&self) -> bool {
        self.ty.is_empty() && self.description.is_none()
    }
}

/// Resolve a head to its current commit address via the registry.
///
/// A `Commit` head is its own address; a `Branch` head resolves through the
/// registry's name map. Used to key GUI side-metadata (e.g. [`InterfaceDocs`])
/// by the head's current commit.
pub fn head_commit_addr(
    registry: &dyn Registry,
    head: &gantz_ca::Head,
) -> Option<gantz_ca::CommitAddr> {
    match head {
        gantz_ca::Head::Commit(ca) => Some(*ca),
        gantz_ca::Head::Branch(name) => registry.names().get(name).copied(),
    }
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

    /// The head's latest module artifact (source text + source map), for
    /// display and for resolving node/error spans into the module source.
    fn module(&self, _head: &gantz_ca::Head) -> Option<&gantz_core::vm::Compiled> {
        None
    }

    /// The rendered error chain from the head's latest compile, when it
    /// failed. May coexist with [`module`][Self::module] (a generated module
    /// that steel rejected).
    fn compile_error(&self, _head: &gantz_ca::Head) -> Option<&str> {
        None
    }

    /// Diagnostics from the head's latest compile and entrypoint
    /// evaluations.
    fn diagnostics(&self, _head: &gantz_ca::Head) -> &[gantz_core::Diagnostic] {
        &[]
    }
}

/// Mutable access to a head's data, provided via [`HeadAccess::with_head_mut`].
pub struct HeadDataMut<'a, N> {
    pub graph: &'a mut gantz_core::node::graph::Graph<N>,
    /// View state (node layout + camera) for this head's graph. Nested graphs
    /// are separate named heads with their own view, so one view per head
    /// suffices (no path-keyed map).
    pub view: &'a mut egui_graph::View,
    pub vm: &'a mut Engine,
}

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
    ) -> egui_graph::FramedResponse<egui::Response>;

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

    /// Look up the demo graph name associated with this node, if any.
    fn demo_graph<'a>(&self, _registry: &'a dyn Registry) -> Option<&'a str> {
        None
    }

    /// The head this node navigates to when entered, if any.
    ///
    /// Returned for nodes that reference a named graph (e.g.
    /// [`NamedRef`](crate::node::NamedRef)): double-clicking enters it in place,
    /// and the scene offers an "open in new tab" context-menu action.
    fn nav_head(&self, _registry: &dyn Registry) -> Option<gantz_ca::Head> {
        None
    }

    /// On-hover documentation for the input socket at the given index.
    ///
    /// Shown as a tooltip when the user hovers the inlet. Wrapper nodes should
    /// delegate to their inner node; nodes that reference a graph resolve the
    /// referenced graph's docs via [`Registry::interface_docs`].
    fn input_doc(&self, _registry: &dyn Registry, _ix: usize) -> Option<SocketDoc> {
        None
    }

    /// On-hover documentation for the output socket at the given index.
    ///
    /// See [`NodeUi::input_doc`].
    fn output_doc(&self, _registry: &dyn Registry, _ix: usize) -> Option<SocketDoc> {
        None
    }
}

/// A wrapper around a node's path and the VM providing easy access to the
/// node's state.
pub struct NodeCtx<'a> {
    registry: &'a dyn Registry,
    path: &'a [node::Id],
    inlets: &'a [node::Id],
    outlets: &'a [node::Id],
    /// Inlet/outlet docs for the graph currently being shown, if any. Lets
    /// `Inlet`/`Outlet` inspector UIs read and pre-fill the doc being edited.
    interface_docs: Option<&'a InterfaceDocs>,
    vm: &'a mut Engine,
    responses: &'a mut Vec<DynResponse>,
}

/// How to position pasted nodes.
#[derive(Clone, Debug)]
pub enum PastePos {
    /// Offset each node's original position by this amount.
    Offset(egui::Vec2),
    /// Center the pasted nodes at this graph-space position.
    GraphPos(egui::Pos2),
}

/// Resolve a [`PastePos`] to a concrete offset vector for use with
/// [`export::paste`].
pub fn resolve_paste_offset(pos: &PastePos, copied_positions: &egui_graph::Layout) -> egui::Vec2 {
    match pos {
        PastePos::Offset(v) => *v,
        PastePos::GraphPos(target) => {
            if copied_positions.is_empty() {
                target.to_vec2()
            } else {
                let center = copied_positions
                    .values()
                    .fold(egui::Vec2::ZERO, |acc, p| acc + p.to_vec2())
                    / copied_positions.len() as f32;
                target.to_vec2() - center
            }
        }
    }
}

// ----------------------------------------------------------------------------
// Response payloads
//
// Typed payloads emitted from within the widget tree via the dynamic
// [`response::Responses`] channel and returned from `Gantz::show`. With the
// exception of [`OpenCommandPalette`] (which `Gantz::show` handles itself),
// applications drain and handle these after the GUI pass.
// Unhandled payloads should be reported via [`response::Responses::type_names`].
// ----------------------------------------------------------------------------

/// Branch a named node: create a new name with its own commit for the given
/// content address, and replace the node with a reference to it.
#[derive(Clone, Debug)]
pub struct BranchNode {
    pub new_name: String,
    pub ca: gantz_ca::ContentAddr,
    /// Path from root to the NamedRef node (last element = node index).
    pub path: Vec<node::Id>,
}

/// Copy the given nodes to the clipboard.
#[derive(Clone, Debug)]
pub struct CopyNodes(pub std::collections::HashSet<widget::graph_scene::NodeIndex>);

/// Create a new node of the given type in the emitting head's graph.
#[derive(Clone, Debug)]
pub struct CreateNode {
    /// The type name of the node to create.
    pub node_type: String,
}

/// Create a new nested graph in the emitting head's graph.
///
/// Commits a fresh empty graph to the registry under the name `<parent>:<n>`
/// (where `<parent>` is the emitting head's name) and inserts a synced
/// [`node::NamedRef`] to it. Behaves like creating any
/// other node, but is registry-aware.
#[derive(Clone, Copy, Debug)]
pub struct CreateNestedGraph;

/// Evaluate an entrypoint (push or pull).
#[derive(Clone, Debug)]
pub struct EvalEntry(pub gantz_core::compile::Entrypoint);

/// Export all named graphs (with transitive deps + views) to a single
/// `.gantz` file. Emitted without an associated head.
#[derive(Clone, Copy, Debug)]
pub struct ExportAllNamed;

/// Export the emitting head (graph + transitive deps + views) to a `.gantz`
/// file.
#[derive(Clone, Copy, Debug)]
pub struct ExportHead;

/// Insert an inspect node on the given edge at the given position.
#[derive(Clone, Debug)]
pub struct InspectEdge {
    pub edge: petgraph::graph::EdgeIndex<usize>,
    pub pos: egui::Pos2,
}

/// Open the command palette for node creation.
///
/// Handled by `Gantz::show` itself - applications never see this payload.
#[derive(Clone, Copy, Debug)]
pub struct OpenCommandPalette;

/// Open a head (named or commit) as a new tab.
#[derive(Clone, Debug)]
pub struct OpenHead(pub gantz_ca::Head);

/// Navigate the *focused* tab to a head in place (replacing it), rather than
/// opening a new tab. Used for entering a nested graph and for breadcrumb
/// navigation between `parent:child` levels.
#[derive(Clone, Debug)]
pub struct ReplaceHead(pub gantz_ca::Head);

/// Paste clipboard contents at the given position.
///
/// `text` is `Some` when the integration layer provides clipboard text
/// directly (e.g. via `egui::Event::Paste` in eframe). When `None`, the
/// handler is expected to read the system clipboard itself.
#[derive(Clone, Debug)]
pub struct Paste {
    pub text: Option<String>,
    pub pos: PastePos,
}

/// Redo a previously undone edit (move head forward).
#[derive(Clone, Copy, Debug)]
pub struct Redo;

/// Undo the last graph edit (move head to parent commit).
#[derive(Clone, Copy, Debug)]
pub struct Undo;

/// Whether a [`SetInterfaceDoc`] targets an inlet or an outlet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketDocKind {
    Inlet,
    Outlet,
}

/// Set (or clear) the documentation for an inlet or outlet of the emitting
/// head's graph.
///
/// `doc == None` (or an empty doc) clears the entry. Applied to the GUI-side
/// [`InterfaceDocs`] keyed by the head's current commit.
#[derive(Clone, Debug)]
pub struct SetInterfaceDoc {
    pub kind: SocketDocKind,
    /// The inlet/outlet ordinal within the graph.
    pub ix: usize,
    pub doc: Option<SocketDoc>,
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
    ) -> egui_graph::FramedResponse<egui::Response> {
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

    fn demo_graph<'b>(&self, registry: &'b dyn Registry) -> Option<&'b str> {
        (**self).demo_graph(registry)
    }

    fn nav_head(&self, registry: &dyn Registry) -> Option<gantz_ca::Head> {
        (**self).nav_head(registry)
    }

    fn input_doc(&self, registry: &dyn Registry, ix: usize) -> Option<SocketDoc> {
        (**self).input_doc(registry, ix)
    }

    fn output_doc(&self, registry: &dyn Registry, ix: usize) -> Option<SocketDoc> {
        (**self).output_doc(registry, ix)
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

            fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> egui_graph::FramedResponse<egui::Response> {
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

            fn demo_graph<'a>(&self, registry: &'a dyn Registry) -> Option<&'a str> {
                (**self).demo_graph(registry)
            }

            fn nav_head(&self, registry: &dyn Registry) -> Option<gantz_ca::Head> {
                (**self).nav_head(registry)
            }

            fn input_doc(&self, registry: &dyn Registry, ix: usize) -> Option<SocketDoc> {
                (**self).input_doc(registry, ix)
            }

            fn output_doc(&self, registry: &dyn Registry, ix: usize) -> Option<SocketDoc> {
                (**self).output_doc(registry, ix)
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
        interface_docs: Option<&'a InterfaceDocs>,
        vm: &'a mut Engine,
        responses: &'a mut Vec<DynResponse>,
    ) -> Self {
        Self {
            registry,
            path,
            inlets,
            outlets,
            interface_docs,
            vm,
            responses,
        }
    }

    /// Emit a response payload for the application to handle after the GUI
    /// pass.
    ///
    /// Payloads may be any of the builtin types (e.g. [`OpenHead`],
    /// [`EvalEntry`]) or custom types defined alongside the node, allowing
    /// independently-declared nodes to communicate with application-specific
    /// handlers.
    pub fn response<T: ResponseData>(&mut self, data: T) {
        self.responses.push(DynResponse::new(data));
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
    pub fn push_eval(&mut self, n_outputs: u8) {
        let ep = gantz_core::compile::entrypoint::push(self.path.to_vec(), n_outputs);
        self.response(EvalEntry(ep));
    }

    /// Queue a call to the generated pull evaluation function for this node.
    ///
    /// This will only be successful if the underlying node's
    /// [`gantz_core::Node::pull_eval`] fn returned `Some` last time the graph
    /// was compiled.
    pub fn pull_eval(&mut self, n_inputs: u8) {
        let ep = gantz_core::compile::entrypoint::pull(self.path.to_vec(), n_inputs);
        self.response(EvalEntry(ep));
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

    /// The inlet/outlet docs for the graph currently being shown, if any.
    ///
    /// Exposed so that `Inlet`/`Outlet` inspector UIs can read and pre-fill the
    /// doc being edited.
    pub fn interface_docs(&self) -> Option<&InterfaceDocs> {
        self.interface_docs
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
