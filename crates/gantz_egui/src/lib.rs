//! A suite of widgets, nodes and implementations for creating a GUI around
//! gantz using `egui`.

use petgraph::visit::{IntoNodeReferences, NodeRef};
use std::borrow::Cow;
use steel::{
    SteelErr, SteelVal,
    rvals::{FromSteelVal, IntoSteelVal},
    steel_vm::engine::Engine,
};

pub mod cycle;
pub mod export;
pub mod format;
mod impls;
pub mod keybind;
pub mod node;
pub mod ops;
pub mod reg;
pub mod response;
pub mod sync;
pub mod view;
pub mod widget;

// Re-export traits that make up the Registry supertrait.
pub use egui_graph::SocketKind;
pub use keybind::{Action, Keymap};
pub use node::{FnNodeNames, NameRegistry};
pub use reg::RegistryRef;
pub use response::{
    ContextMenuResponse, DynResponse, InspectorRowsResponse, InspectorUiResponse, NodeUiResponse,
    NodeViewResponse, ResponseData, Responses,
};
pub use view::{Camera, SceneView};
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

    /// Whether referencing the graph named `target` from the graph named
    /// `editing` would create a reference cycle (see [`cycle::would_cycle`]).
    ///
    /// Used by the command palette to hide node types that would form a cycle.
    /// The default is conservative (never a cycle); the standard [`RegistryRef`]
    /// implementation walks the registry.
    fn would_ref_cycle(&self, target: &str, editing: &str) -> bool {
        let _ = (target, editing);
        false
    }

    /// Get the demo graph name associated with the graph of the given name.
    fn demo_graph(&self, name: &str) -> Option<&str> {
        let _ = name;
        None
    }

    /// The [`SocketDoc`] for the given socket of the graph referenced by `ca`.
    ///
    /// Lets a referencing node (e.g. [`node::NamedRef`]) surface the referenced
    /// graph's inlet/outlet docs. The standard impl resolves the referenced
    /// graph and reads the relevant `Inlet`/`Outlet` marker's own doc, so docs
    /// live on the nodes rather than in side-metadata.
    fn socket_doc(
        &self,
        ca: &gantz_ca::ContentAddr,
        kind: SocketKind,
        ix: usize,
    ) -> Option<SocketDoc> {
        let _ = (ca, kind, ix);
        None
    }

    /// Display-ready documentation for the creatable node type named `name`.
    ///
    /// Combines the node's description with its derived input/output
    /// [`SocketDoc`]s. Shown beside the highlighted entry in the command palette
    /// and as hover documentation in the "Graphs" select widget. The standard
    /// [`RegistryRef`] impl introspects a builtin instance or resolves a named
    /// graph; the default returns just the name.
    fn command_info(&self, name: &str) -> CommandInfo {
        CommandInfo {
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// The stored description for the named graph `name`, if any.
    ///
    /// Used to seed the description editor in the graph config pane. The
    /// standard [`RegistryRef`] impl reads it from the content-addressed
    /// registry; the default has none.
    fn graph_description(&self, name: &str) -> Option<&str> {
        let _ = name;
        None
    }

    /// A concise description of the creatable node type `name`, for inline
    /// display in the command palette. Lighter than [`command_info`](Self::command_info)
    /// (it derives no input/output docs); the default has none.
    fn node_description(&self, name: &str) -> Option<Cow<'static, str>> {
        let _ = name;
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

/// Display-ready documentation for a creatable node type.
///
/// Built by [`Registry::command_info`] from a node's [`description`] and its
/// derived per-socket [`SocketDoc`]s, and rendered by [`node_info_ui`] in the
/// command palette and the "Graphs" select hover.
///
/// [`description`]: NodeUi::description
#[derive(Clone, Debug, Default)]
pub struct CommandInfo {
    /// The node type's name (the palette entry text).
    pub name: String,
    /// A concise description of what the node does, if any.
    pub description: Option<Cow<'static, str>>,
    /// One [`SocketDoc`] per input, in socket order.
    pub inputs: Vec<SocketDoc>,
    /// One [`SocketDoc`] per output, in socket order.
    pub outputs: Vec<SocketDoc>,
}

/// Render a [`CommandInfo`] as a name heading, description, and labelled
/// input/output lists.
///
/// Used both for the command palette's side panel and as the body of the
/// per-item / graph-select hover tooltips. Callers that render inside a tooltip
/// should set a max width first (see the tooltip-width note in `socket_hover`).
pub fn node_info_ui(info: &CommandInfo, ui: &mut egui::Ui) {
    if !info.name.is_empty() {
        ui.strong(&info.name);
    }
    if let Some(desc) = &info.description {
        ui.label(desc.as_ref());
    }
    socket_doc_list(ui, "Inputs", &info.inputs);
    socket_doc_list(ui, "Outputs", &info.outputs);
}

/// Render a labelled list of socket docs as `[ix] ty - description` rows.
fn socket_doc_list(ui: &mut egui::Ui, heading: &str, docs: &[SocketDoc]) {
    if docs.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.weak(heading);
    for (ix, doc) in docs.iter().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.weak(format!("{ix}"));
            if !doc.ty.is_empty() {
                ui.strong(doc.ty.as_ref());
            }
            if let Some(desc) = &doc.description {
                ui.label(format!("- {desc}"));
            }
        });
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
    pub view: &'a mut crate::SceneView,
    pub vm: &'a mut Engine,
}

/// A trait providing an egui `Ui` implementation for gantz nodes.
///
/// # Reporting changes
///
/// The graph the node lives in is content-addressed: its identity (and thus
/// the need to re-commit and recompile) is derived from a hash of every node's
/// CA-relevant state. To let the application detect edits without re-hashing
/// the whole graph every frame, each method returns a response with a
/// `changed` flag.
///
/// **A node MUST mark its response [`changed`](NodeUiResponse::mark_changed)
/// whenever it mutates state that contributes to its content address (its
/// `CaHash`), at the moment that state is actually written.** For
/// buffered/debounced edits (e.g. a text field flushed on focus loss) mark
/// `changed` at the *flush*, not on the keystroke - the node weight only
/// changes at the flush. Silent mutations (e.g. auto-syncing a reference to a
/// newer commit) must mark `changed` too, even though no widget was touched.
///
/// State that does NOT affect the content address must NOT mark `changed`:
/// `#[cahash(skip)]` fields, values written to VM runtime state via
/// [`NodeCtx::update_value`], node layout/position, and evaluation triggers
/// queued via [`push_eval`](NodeUiResponse::push_eval). A missed `changed`
/// leaves the committed graph stale (a correctness bug); a spurious `changed`
/// only costs a redundant hash, so when in doubt, mark it.
pub trait NodeUi {
    /// The name used to present the node within the inspector.
    fn name(&self, _registry: &dyn Registry) -> &str;

    /// Instantiate the `Ui` for the given node.
    ///
    /// The node's path into the state tree and the VM are provided to allow for
    /// access to the node's state. The egui_graph node context is provided to
    /// allow customizing the frame and other node display properties.
    ///
    /// Returns a [`NodeUiResponse`] wrapping the framed egui response; mark it
    /// [`changed`](NodeUiResponse::mark_changed) on CA-affecting edits and
    /// [`emit`](NodeUiResponse::emit) any payloads (see the trait docs).
    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse;

    /// Optionally add additional rows to the node's inspector UI.
    ///
    /// By default, only the node's path and its current state within the VM are
    /// shown. Adding to the given `body` by providing an implementation of this
    /// method will append extra rows. Mark the returned response
    /// [`changed`](InspectorRowsResponse::mark_changed) on CA-affecting edits.
    fn inspector_rows(
        &mut self,
        _ctx: &mut NodeCtx,
        _body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        InspectorRowsResponse::default()
    }

    /// Extra UI for the node to be presented within the node inspector
    /// following the default table.
    ///
    /// See [`NodeUi::inspector_rows`] for how to simply append rows to the
    /// table.
    fn inspector_ui(&mut self, _ctx: NodeCtx, _ui: &mut egui::Ui) -> InspectorUiResponse {
        InspectorUiResponse::default()
    }

    /// The node's UI when detached from the graph into its own pane via the
    /// "open view" action, for monitoring it in a fixed location.
    ///
    /// Unlike [`ui`](NodeUi::ui), this receives a plain [`egui::Ui`] filling the
    /// pane (no graph frame or sockets). The default renders the node's current
    /// VM state value (its debug repr), so every node is viewable with something
    /// useful; "viewer" nodes (e.g. [`Plot`](crate::node::Plot)) override it to
    /// render their full visualisation. Mark the returned response
    /// [`changed`](NodeViewResponse::mark_changed) on CA-affecting edits and
    /// [`emit`](NodeViewResponse::emit) any payloads.
    fn view_ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
        default_view_ui(&ctx, ui)
    }

    /// Add node-specific items to the node's right-click context menu.
    ///
    /// Called after the built-in items (copy, reset, delete, ...). Mark the
    /// returned response [`changed`](ContextMenuResponse::mark_changed) on
    /// CA-affecting edits and [`emit`](ContextMenuResponse::emit) any payloads
    /// for the application (or `Gantz::show`) to handle.
    fn context_menu(&mut self, _ctx: &mut NodeCtx, _ui: &mut egui::Ui) -> ContextMenuResponse {
        ContextMenuResponse::default()
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

    /// A concise, free-form description of what the node does.
    ///
    /// Shown alongside the node's inputs/outputs in the command palette and as
    /// hover documentation in the "Graphs" select widget. Builtins hardcode a
    /// short string; nodes that reference a named graph resolve the graph's
    /// stored description via [`Registry::command_info`].
    fn description(&self) -> Option<&'static str> {
        None
    }

    /// On-hover documentation for the socket of the given kind and index.
    ///
    /// Shown as a tooltip when the user hovers the socket. `Inlet`/`Outlet`
    /// read their own stored docs; nodes that reference a graph resolve the
    /// referenced graph's docs via [`Registry::socket_doc`].
    fn socket_doc(
        &self,
        _registry: &dyn Registry,
        _kind: SocketKind,
        _ix: usize,
    ) -> Option<SocketDoc> {
        None
    }

    /// Whether the inspector's default table includes a row showing the node's
    /// current VM state.
    ///
    /// Returns `true` by default. Override to `false` for nodes whose raw state
    /// is large or unwieldy (e.g. a long buffer) and better summarised in
    /// [`inspector_ui`](NodeUi::inspector_ui).
    fn show_state(&self) -> bool {
        true
    }
}

/// The default [`NodeUi::view_ui`] body: the node's current VM state value (its
/// debug repr), matching what [`Inspect`](crate::node::Inspect) shows in-graph.
/// No type label - the tab title already names the node. Used by any node that
/// doesn't override `view_ui` with a richer visualisation.
fn default_view_ui(ctx: &NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
    let mut resp = NodeViewResponse::default();
    let text = match ctx.extract_value() {
        Ok(Some(val)) => format!("{val:?}"),
        Ok(None) => "∅".to_string(),
        Err(_) => "ERR".to_string(),
    };
    let inner = egui::ScrollArea::both()
        .auto_shrink(false)
        .show(ui, |ui| ui.add(egui::Label::new(text).selectable(true)))
        .inner;
    resp.inner = Some(inner);
    resp
}

/// A wrapper around a node's path and the VM providing easy access to the
/// node's state.
///
/// Node UI methods report edits and emit payloads via their returned response
/// types (see [`NodeUi`]); `NodeCtx` itself only provides read/write access to
/// the node's surroundings (registry, path, VM state).
pub struct NodeCtx<'a> {
    registry: &'a dyn Registry,
    path: &'a [node::Id],
    inlets: &'a [node::Id],
    outlets: &'a [node::Id],
    vm: &'a mut Engine,
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

/// Copy the given nodes to the clipboard, then remove them from the graph.
#[derive(Clone, Debug)]
pub struct CutNodes(pub std::collections::HashSet<widget::graph_scene::NodeIndex>);

/// Duplicate the given nodes in place (copy, then paste at a small offset).
#[derive(Clone, Debug)]
pub struct DuplicateNodes(pub std::collections::HashSet<widget::graph_scene::NodeIndex>);

/// Create a new node of the given type in the emitting head's graph.
#[derive(Clone, Debug)]
pub struct CreateNode {
    /// The type name of the node to create.
    pub node_type: String,
    /// Where to place the new node, in graph coordinates. When `None`, the node
    /// is placed at the center of the current view.
    pub pos: Option<egui::Pos2>,
}

/// Create a new nested graph in the emitting head's graph.
///
/// Commits a fresh empty graph to the registry under the name `<parent>:<n>`
/// (where `<parent>` is the emitting head's name) and inserts a synced
/// [`node::NamedRef`] to it. Behaves like creating any
/// other node, but is registry-aware.
#[derive(Clone, Copy, Debug)]
pub struct CreateNestedGraph {
    /// Where to place the new node, in graph coordinates. When `None`, the node
    /// is placed at the center of the current view.
    pub pos: Option<egui::Pos2>,
}

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

/// Reset the top-level tile layout to its default arrangement.
///
/// Handled by `Gantz::show` itself - applications never see this payload.
#[derive(Clone, Copy, Debug)]
pub struct ResetTilesLayout;

/// Open (show) the logs pane.
///
/// Handled by `Gantz::show` itself - applications never see this payload.
#[derive(Clone, Copy, Debug)]
pub struct OpenLogs;

/// Open the given node's view ([`NodeUi::view_ui`]) as a tile in the Node Views
/// pane, for monitoring it in a fixed location. The node is identified by its
/// `path` within the emitting head's graph (the head is taken from the payload's
/// head tag).
///
/// Handled by `Gantz::show` itself - applications never see this payload.
#[derive(Clone, Debug)]
pub struct OpenNodeView {
    /// Path to the node within its head's graph (last element = node index).
    pub path: Vec<node::Id>,
    /// The node's type name ([`NodeUi::name`]), captured at emit time for the
    /// view tile's title (the registry is in scope there, not at the drain).
    pub ty_name: String,
}

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

impl<'a, N> NodeUi for &'a mut N
where
    N: ?Sized + NodeUi,
{
    fn name(&self, registry: &dyn Registry) -> &str {
        (**self).name(registry)
    }

    fn description(&self) -> Option<&'static str> {
        (**self).description()
    }

    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        (**self).ui(ctx, uictx)
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        (**self).inspector_rows(ctx, body)
    }

    fn inspector_ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> InspectorUiResponse {
        (**self).inspector_ui(ctx, ui)
    }

    fn view_ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
        (**self).view_ui(ctx, ui)
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

    fn socket_doc(
        &self,
        registry: &dyn Registry,
        kind: SocketKind,
        ix: usize,
    ) -> Option<SocketDoc> {
        (**self).socket_doc(registry, kind, ix)
    }

    fn context_menu(&mut self, ctx: &mut NodeCtx, ui: &mut egui::Ui) -> ContextMenuResponse {
        (**self).context_menu(ctx, ui)
    }

    fn show_state(&self) -> bool {
        (**self).show_state()
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

            fn description(&self) -> Option<&'static str> {
                (**self).description()
            }

            fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
                (**self).ui(ctx, uictx)
            }

            fn inspector_rows(&mut self, ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) -> InspectorRowsResponse {
                (**self).inspector_rows(ctx, body)
            }

            fn inspector_ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> InspectorUiResponse {
                (**self).inspector_ui(ctx, ui)
            }

            fn view_ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
                (**self).view_ui(ctx, ui)
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

            fn socket_doc(&self, registry: &dyn Registry, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
                (**self).socket_doc(registry, kind, ix)
            }

            fn context_menu(&mut self, ctx: &mut NodeCtx, ui: &mut egui::Ui) -> ContextMenuResponse {
                (**self).context_menu(ctx, ui)
            }

            fn show_state(&self) -> bool {
                (**self).show_state()
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
    ) -> Self {
        Self {
            registry,
            path,
            inlets,
            outlets,
            vm,
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
