use crate::{
    Action, CopyNodes, CreateNestedGraph, CreateNode, CutNodes, DuplicateNodes, Env,
    ExportAllNamed, ExportHead, ExportStyle, HeadAccess, ImportStyle, Keymap, NodeCtx, NodeUi,
    OpenLogs, OpenNodePalette, OpenNodeView, Paste, Redo, ReplaceHead, ResetTilesLayout, Undo,
    export,
    node::NodeCodec,
    response::{DynResponse, Responses},
    style::SeparatorConfig,
    widget::{self, GraphScene, GraphSceneState, graph_scene},
};
use gantz_core::node;
use petgraph::visit::IntoNodeIdentifiers;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use steel::steel_vm::engine::Engine;

/// A file dropped onto a gantz pane.
#[derive(Debug)]
pub struct FileDrop {
    pub bytes: Vec<u8>,
    pub target: FileDropTarget,
}

/// Which pane received the file drop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDropTarget {
    /// The Graphs pane. Merge the registry and views only.
    Graphs,
    /// The GraphScene pane. Merge, then open the root graph if unique.
    GraphScene,
}

/// The reserved node-type name that creates a new nested graph.
///
/// Selecting this entry in the node palette emits a
/// [`CreateNestedGraph`] rather than a [`CreateNode`], so a nested graph is
/// created like any other node but routed through the registry-aware op.
pub const NESTED_GRAPH_TYPE: &str = "graph";

/// The top-level gantz widget.
pub struct Gantz<'a> {
    env: &'a Env<'a>,
    /// The value-level codec through which working-graph nodes reify for
    /// their UI passes and erase back on change.
    codec: &'a NodeCodec,
    base_names: &'a crate::reg::Names,
    log_source: Option<LogSource>,
    perf_vm: Option<&'a mut widget::PerfCapture>,
    perf_gui: Option<&'a mut widget::PerfCapture>,
    base_immutable: bool,
    compile_config: Option<gantz_core::compile::Config>,
    validate_change_tracking: Option<bool>,
    settings_tabs: &'a mut [&'a mut dyn widget::SettingsTab],
    ext_panes: &'a mut [&'a mut dyn widget::ExtPane],
    ref_ext_uis: &'a [&'a dyn crate::node::RefExtUi],
    edge_styles: &'a [&'a dyn widget::EdgeStyle],
    base_sources: Option<BaseSourcesCtx<'a>>,
    pane_window_mode: PaneWindowMode,
    collab: Option<&'a crate::collab::CollabUiState>,
    /// A host-provided clipboard reader for widget paste affordances, since
    /// egui alone cannot read the clipboard. `None` hides them.
    clipboard: Option<&'a dyn Fn() -> Option<String>>,
    audio_heads: Option<&'a HashSet<gantz_ca::Head>>,
}

/// Base-source authoring context for the graph config pane's "source"
/// dropdown. See [`widget::GraphConfig::base_sources`]. Supplied only by
/// base-authoring hosts like `update-base`, where the per-source write-back
/// makes an association change durable.
#[derive(Clone, Copy)]
pub struct BaseSourcesCtx<'a> {
    /// The available base source names, in load order.
    pub sources: &'a [&'a str],
    /// Each base name's owning source.
    pub name_sources: &'a HashMap<String, &'static str>,
    /// The source an unattributed name is written to. Session-created names
    /// are unattributed.
    pub default_source: &'a str,
}

/// Selects who draws the windows that popped-out panes live in.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PaneWindowMode {
    /// The widget draws each windowed pane as a floating `egui::Window`. Used on
    /// web, in the eframe demo, and as the native fallback. The default.
    #[default]
    EguiWindow,
    /// The host owns real OS windows and renders each windowed pane itself via
    /// [`Gantz::render_windowed_pane`]. The widget only reports the windowed
    /// set via [`GantzResponse::windowed_panes`].
    HostNative,
}

enum LogSource {
    Logger(widget::log_view::Logger),
    #[cfg(feature = "tracing")]
    TraceCapture(
        widget::trace_view::TraceCapture,
        tracing::level_filters::LevelFilter,
    ),
}

/// All state for the widget.
#[derive(serde::Deserialize, serde::Serialize)]
pub struct GantzState {
    /// State for each open head.
    #[serde(serialize_with = "gantz_ca::serde_sorted::serialize_map")]
    pub open_heads: OpenHeadStates,
    pub view_toggles: ViewToggles,
    #[serde(default, alias = "command_palette")]
    pub node_palette: widget::NodePalette,
    /// Global auto-layout parameters. These are the non-flow `egui_graph`
    /// layout params. Flow stays per-head in [`OpenHeadState::layout_flow`].
    #[serde(default)]
    pub layout_config: LayoutConfig,
    /// Global interactive-scene parameters for the dot grid, drag snapping
    /// and snap-align. They mirror the per-frame `egui_graph::Graph` builder
    /// options and apply to every open head.
    #[serde(default)]
    pub scene_config: SceneConfig,
    /// The egui theme preference and per-theme style overrides, applied to
    /// every gantz context. See [`crate::style`]. Edited in the Settings
    /// Style subtab.
    #[serde(default)]
    pub style: crate::StyleConfig,
    /// The command keyboard shortcuts. The single source of truth for editor
    /// command bindings. See [`crate::keybind`]. Edited in the Settings
    /// Keybinds subtab.
    #[serde(default)]
    pub keymap: Keymap,
    /// User-editable collaboration configuration, edited in the Settings
    /// Collab subtab.
    #[serde(default)]
    pub collab: crate::collab::CollabConfig,
    /// How graph merges resolve conflicts. Edited via the merge row's "⛭"
    /// menu in the Graph Config pane.
    #[serde(default)]
    pub merge_resolutions: gantz_ca::merge::Resolutions,
    /// Per-head redo stacks for undo and redo.
    #[serde(default, serialize_with = "gantz_ca::serde_sorted::serialize_map")]
    pub redo_stacks: HashMap<gantz_ca::Head, Vec<gantz_ca::CommitAddr>>,
    /// Per-head stepping state for session undo and redo by revert commit.
    /// See [`crate::ops::session_undo`]. Lives and migrates beside
    /// [`Self::redo_stacks`].
    #[serde(default, serialize_with = "gantz_ca::serde_sorted::serialize_map")]
    pub undo_cursors: HashMap<gantz_ca::Head, crate::ops::RevertCursor>,
    /// The sidebar width in pixels. It is fixed, not proportional, so it
    /// survives window resizes. Dragging the divider updates it.
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    /// The bottom tray's pixel height, maintained across window resizes.
    #[serde(default = "default_tray_height")]
    pub tray_height: f32,
    /// Last-seen size of each pane popped out into its own OS window, keyed by
    /// [`pane_key`], so a native host can restore it across sessions. Only the
    /// native window backend populates this. Web `egui::Window`s persist
    /// their own geometry in egui memory.
    #[serde(default)]
    pub windowed_geometry: HashMap<String, PaneWindowGeometry>,
}

/// The persisted geometry of a pane's pop-out window, in logical pixels.
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
pub struct PaneWindowGeometry {
    pub width: f32,
    pub height: f32,
}

fn default_sidebar_width() -> f32 {
    270.0
}

fn default_tray_height() -> f32 {
    300.0
}

pub type OpenHeadStates = HashMap<gantz_ca::Head, OpenHeadState>;

/// State associated with a single open graph.
#[derive(serde::Deserialize, serde::Serialize)]
pub struct OpenHeadState {
    /// State associated with the `GraphScene` widget.
    pub scene: GraphSceneState,
    /// The per-head flow direction used when auto-layout is invoked.
    #[serde(default = "default_layout_flow")]
    pub layout_flow: egui::Direction,
    /// Whether the head's audio output is muted.
    #[serde(default)]
    pub muted: bool,
}

fn default_layout_flow() -> egui::Direction {
    GantzState::DEFAULT_DIRECTION
}

impl Default for OpenHeadState {
    fn default() -> Self {
        Self {
            scene: GraphSceneState::default(),
            layout_flow: GantzState::DEFAULT_DIRECTION,
            muted: false,
        }
    }
}

/// Global auto-layout parameters, mirroring the non-flow fields of
/// [`egui_graph::LayoutParams`]. Flow stays per-head. See
/// [`OpenHeadState::layout_flow`]. These apply to every head's auto-layout.
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct LayoutConfig {
    /// The gap between adjacent layers along the flow direction.
    #[serde(default = "default_layer_gap")]
    pub layer_gap: f32,
    /// The gap between adjacent nodes within a layer.
    #[serde(default = "default_node_gap")]
    pub node_gap: f32,
    /// The gap between disconnected components of the graph.
    #[serde(default = "default_component_gap")]
    pub component_gap: f32,
    /// Whether the layout accounts for the socket each edge connects to.
    #[serde(default = "default_socket_aware")]
    pub socket_aware: bool,
}

fn default_layer_gap() -> f32 {
    egui_graph::LayoutParams::DEFAULT_LAYER_GAP
}

fn default_node_gap() -> f32 {
    egui_graph::LayoutParams::DEFAULT_NODE_GAP
}

fn default_component_gap() -> f32 {
    egui_graph::LayoutParams::DEFAULT_COMPONENT_GAP
}

fn default_socket_aware() -> bool {
    true
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            layer_gap: default_layer_gap(),
            node_gap: default_node_gap(),
            component_gap: default_component_gap(),
            socket_aware: default_socket_aware(),
        }
    }
}

impl LayoutConfig {
    /// Build [`egui_graph::LayoutParams`] from these globals plus a per-head
    /// `flow` direction.
    pub fn to_params(&self, flow: egui::Direction) -> egui_graph::LayoutParams {
        egui_graph::LayoutParams::new(flow)
            .layer_gap(self.layer_gap)
            .node_gap(self.node_gap)
            .component_gap(self.component_gap)
            .socket_aware(self.socket_aware)
    }
}

/// Global interactive-scene configuration for the dot grid, drag snapping and
/// snap-align. It mirrors the per-frame options on [`egui_graph::Graph`] and
/// applies to every open head, like [`LayoutConfig`].
#[derive(Clone, Copy, Default, serde::Deserialize, serde::Serialize)]
pub struct SceneConfig {
    #[serde(default)]
    pub grid: GridConfig,
    #[serde(default)]
    pub snap: SnapConfig,
    #[serde(default)]
    pub align: AlignConfig,
}

/// The dot grid drawn behind the graph. See [`egui_graph::Graph::dot_grid`].
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct GridConfig {
    /// Whether the dot grid is drawn.
    #[serde(default = "default_grid_show")]
    pub show: bool,
    /// The base spacing of the dot grid, in graph-space units.
    #[serde(default = "default_grid_step")]
    pub step: f32,
}

fn default_grid_show() -> bool {
    true
}

fn default_grid_step() -> f32 {
    20.0
}

impl Default for GridConfig {
    fn default() -> Self {
        Self {
            show: default_grid_show(),
            step: default_grid_step(),
        }
    }
}

/// How a dragged node's position is snapped.
#[derive(Clone, Copy, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum SnapMode {
    /// Snap to the nearest unit point, which is effectively free.
    #[default]
    Point,
    /// Snap to a relative fraction of the dot grid. See
    /// [`SnapConfig::grid_ratio`].
    Grid,
}

/// Drag snapping configuration. See [`egui_graph::Graph::snap`].
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct SnapConfig {
    #[serde(default)]
    pub mode: SnapMode,
    /// In [`SnapMode::Grid`], the snap step relative to the grid step, so
    /// `snap_step = grid.step * grid_ratio`. `1.0` snaps to the full grid,
    /// `0.5` to half and `0.25` to quarter.
    #[serde(default = "default_grid_ratio")]
    pub grid_ratio: f32,
}

fn default_grid_ratio() -> f32 {
    1.0
}

impl Default for SnapConfig {
    fn default() -> Self {
        Self {
            mode: SnapMode::default(),
            grid_ratio: default_grid_ratio(),
        }
    }
}

/// Drag-time snap-align configuration. See [`egui_graph::Graph::align`].
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct AlignConfig {
    /// Whether a dragged node snap-aligns to its neighbours.
    #[serde(default = "default_align_enabled")]
    pub enabled: bool,
    /// Align to neighbours' left/right/top/bottom edges.
    #[serde(default = "default_align_edges")]
    pub edges: bool,
    /// Align to neighbours' horizontal/vertical centres.
    #[serde(default = "default_align_centers")]
    pub centers: bool,
}

fn default_align_enabled() -> bool {
    true
}

fn default_align_edges() -> bool {
    true
}

fn default_align_centers() -> bool {
    false
}

impl Default for AlignConfig {
    fn default() -> Self {
        Self {
            enabled: default_align_enabled(),
            edges: default_align_edges(),
            centers: default_align_centers(),
        }
    }
}

impl SceneConfig {
    /// Apply the grid, snap and align options onto an [`egui_graph::Graph`]
    /// builder. The snap step derives from the mode. [`SnapMode::Point`]
    /// snaps to unit points and [`SnapMode::Grid`] to a fraction of the grid.
    pub fn apply(self, graph: egui_graph::Graph) -> egui_graph::Graph {
        let snap_step = match self.snap.mode {
            SnapMode::Point => 1.0,
            SnapMode::Grid => self.grid.step * self.snap.grid_ratio,
        };
        graph
            // gantz owns zoom persistence. The camera stores centre and zoom,
            // and the scene rect is rebuilt from it each frame against the
            // live viewport. Use `MaintainView` so `egui_graph` does not
            // rescale the rect on resize as well and double-adjust the zoom.
            .resize_behavior(egui_graph::ResizeBehavior::MaintainView)
            .dot_grid(self.grid.show)
            .dot_grid_step(self.grid.step)
            .snap(Some(egui_graph::Snap::Round))
            .snap_step(snap_step)
            .align(self.align.enabled)
            .align_targets(egui_graph::AlignTargets {
                edges: self.align.edges,
                centers: self.align.centers,
            })
    }
}

/// A pane within the outer tree.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Pane {
    /// An application-supplied pane, identified by its provider's stable key.
    /// See [`ExtPane::key`][widget::ExtPane::key]. Renders a placeholder
    /// while no provider supplies the key.
    Ext(String),
    GraphConfig,
    /// Contains the inner graph tree with all open graph tabs.
    GraphScene,
    Graphs,
    GuiPerf,
    /// The focused head's GUI, rendered live through the
    /// [`ui_tree`][crate::ui_tree] interpreter from its `gui` marker tree.
    GuiPreview,
    /// The focused head's stored `gui` marker tree as scheme text, with its
    /// decode warnings.
    GuiTree,
    History,
    Logs,
    NodeInspector,
    /// A node detached from a graph via the "open view" action, rendered via
    /// [`NodeUi::view_ui`] for monitoring. Unlike the singleton variants it
    /// carries data, so any number can be opened and freely placed anywhere
    /// in the top-level tree.
    NodeView(NodeViewPane),
    /// Globally relevant configuration grouped into subtabs.
    Settings,
    Steel,
    VmPerf,
}

/// A pane within the inner graph tree. Contains the head that this pane
/// displays.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
struct GraphPane(gantz_ca::Head);

/// The payload of a [`Pane::NodeView`]. One detached node view, identified by
/// its `head` and `path` within that head's graph. `ty_name` is the node's
/// type name from [`NodeUi::name`], cached at open time so the tab title is
/// stable while the head is closed. The node type never changes. Only its
/// index does, and `migrate_node_view_paths` migrates that.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct NodeViewPane {
    head: gantz_ca::Head,
    path: Vec<node::Id>,
    ty_name: String,
}

/// The egui ID used to store the inner graph tree.
const GRAPH_TREE_ID: &str = "gantz-graph-tiles-tree";

/// The `egui_tiles::Behavior::resize_stroke` shared by both tile trees.
fn separator_stroke(
    cfg: SeparatorConfig,
    style: &egui::Style,
    resize_state: egui_tiles::ResizeState,
) -> egui::Stroke {
    let color = match resize_state {
        egui_tiles::ResizeState::Idle => cfg.color,
        egui_tiles::ResizeState::Hovering | egui_tiles::ResizeState::Dragging => {
            cfg.hover.unwrap_or(cfg.color)
        }
    };
    egui::Stroke::new(cfg.width, color.resolve(&style.visuals))
}

/// Load a value persisted in egui memory as a RON `String`.
///
/// Values are stored as a RON `String` rather than typed. A `String`'s
/// `TypeId` is stable across recompiles, whereas the `TypeId` of a type like
/// `Tree<Pane>` changes whenever this crate is rebuilt. Storing the typed
/// value would leave a stale copy behind in egui's per-type persisted map on
/// every dev build. egui never evicts entries of a type the running build no
/// longer uses. The persisted memory would bloat without bound.
fn load_ron<T: serde::de::DeserializeOwned>(ctx: &egui::Context, id: egui::Id) -> Option<T> {
    let ron = ctx.memory_mut(|m| m.data.get_persisted::<String>(id))?;
    ron::from_str(&ron).ok()
}

/// Persist a value to egui memory as a RON `String`. See [`load_ron`].
fn store_ron<T: serde::Serialize>(ctx: &egui::Context, id: egui::Id, value: &T) {
    if let Ok(ron) = ron::to_string(value) {
        ctx.memory_mut(|m| m.data.insert_persisted(id, ron));
    }
}

/// Load a tile tree from egui's persisted memory. See [`load_ron`].
fn load_tree<P: serde::de::DeserializeOwned>(
    ctx: &egui::Context,
    id: egui::Id,
) -> Option<egui_tiles::Tree<P>> {
    load_ron(ctx, id)
}

/// Persist a tile tree to egui memory as a RON `String`. See [`load_ron`].
fn store_tree<P: serde::Serialize>(ctx: &egui::Context, id: egui::Id, tree: &egui_tiles::Tree<P>) {
    store_ron(ctx, id, tree)
}

/// egui temp-memory flag id for a pending "clear egui memory" request.
fn clear_egui_memory_id() -> egui::Id {
    egui::Id::new("gantz-clear-egui-memory-request")
}

/// Request that egui's persisted memory be cleared at the start of the next
/// `Gantz::show`.
///
/// A recovery tool for when egui's persisted memory accumulates stale state.
/// It discards the UI memory but leaves the graph registry and other app
/// storage untouched. Deferring to the next frame keeps the clear
/// deterministic. It runs before any persisted UI state is loaded, rather
/// than racing this frame's widgets.
pub fn request_clear_egui_memory(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(clear_egui_memory_id(), true));
}

/// Update the head stored in a graph pane when a commit CA changes.
///
/// This should be called after `commit_graph_to_head` modifies a head's commit CA.
/// It updates the persisted graph tree to reflect the new head value.
pub fn update_graph_pane_head(
    ctx: &egui::Context,
    old_head: &gantz_ca::Head,
    new_head: &gantz_ca::Head,
) {
    if old_head == new_head {
        return;
    }
    let graph_tree_id = egui::Id::new(GRAPH_TREE_ID);
    let Some(mut tree) = load_tree::<GraphPane>(ctx, graph_tree_id) else {
        return;
    };
    let mut changed = false;
    for (_, tile) in tree.tiles.iter_mut() {
        if let egui_tiles::Tile::Pane(GraphPane(head)) = tile {
            if head == old_head {
                *head = new_head.clone();
                changed = true;
                break;
            }
        }
    }
    if changed {
        store_tree(ctx, graph_tree_id, &tree);
    }
}

/// Migrate the [`Pane::NodeView`] panes for `head` after a node removal
/// reindexed its graph. This mirrors the state, layout and selection
/// migration in [`crate::ops::remove_nodes`]. A view whose node was removed
/// is dropped. A view whose node was swapped to a new index has its path
/// rewritten. This keeps a detached view pointing at the same node across
/// deletions, with no staleness guard. Operates on the top-level `tree`,
/// where node views live as tiles.
fn migrate_node_view_paths(
    tree: &mut egui_tiles::Tree<Pane>,
    head: &gantz_ca::Head,
    reindex: &crate::ops::Reindex,
) {
    if reindex.is_empty() {
        return;
    }
    let mut to_remove = Vec::new();
    for (id, tile) in tree.tiles.iter_mut() {
        let egui_tiles::Tile::Pane(Pane::NodeView(pane)) = tile else {
            continue;
        };
        if pane.head != *head {
            continue;
        }
        // Root-level node views have a single-element path.
        let [ix] = pane.path[..] else { continue };
        match reindex.apply_to_index(ix) {
            Some(new_ix) => pane.path = vec![new_ix],
            None => to_remove.push(*id),
        }
    }
    for id in to_remove {
        tree.tiles.remove(id);
    }
}

/// The data a single pane needs to render, independent of where the pane lives.
///
/// Extracted from [`TreeBehaviour`] so the same [`render_pane`] can draw a pane
/// as a tile in the tree, as a floating `egui::Window`, or into a native
/// host's own OS window.
struct PaneCtx<'a, 's, Access>
where
    Access: HeadAccess,
{
    gantz: &'a mut Gantz<'s>,
    state: &'a mut GantzState,
    access: &'a mut Access,
    focused_head: usize,
    base_names: &'a crate::reg::Names,
    response: &'a mut GantzResponse,
}

/// The context passed to the `egui_tiles::Tree` widget.
struct TreeBehaviour<'a, 's, Access>
where
    Access: HeadAccess,
{
    gantz: &'a mut Gantz<'s>,
    state: &'a mut GantzState,
    access: &'a mut Access,
    focused_head: usize,
    base_names: &'a crate::reg::Names,
    gantz_response: &'a mut GantzResponse,
    /// Panes detached into windows. The tab context menu pushes to this when a
    /// pane is popped out.
    windowed: &'a mut Vec<Pane>,
}

/// Response from the top-level gantz widget.
///
/// Whole-widget outcomes such as focus, tab management, file drops and config
/// are plain fields. Operations emitted from deeper within the widget tree
/// arrive as dynamic payloads in [`responses`][Self::responses] for the
/// application to drain and handle.
#[derive(Debug)]
pub struct GantzResponse {
    /// The focused head index. User interaction may have changed it.
    pub focused_head: usize,
    pub graph_select: Option<widget::graph_select::GraphSelectResponse>,
    /// Heads that were closed via the tab close button.
    pub closed_heads: Vec<gantz_ca::Head>,
    /// A new branch created from a tab double-click, as the original head and
    /// the new branch name.
    pub new_branch: Option<(gantz_ca::Head, String)>,
    /// Files dropped onto gantz panes.
    pub file_drops: Vec<FileDrop>,
    /// The demo graph association changed, as the head and the new demo name
    /// or `None`.
    pub demo_changed: Option<(gantz_ca::Head, Option<String>)>,
    /// A named graph's description was edited, as the head and the new
    /// description. An empty string clears the description.
    pub description_changed: Option<(gantz_ca::Head, String)>,
    /// A base graph should be reset to its original state.
    pub reset_base_graph: Option<gantz_ca::Head>,
    /// All `demo-*` base graphs should be reset to their original state.
    pub reset_all_demos: bool,
    /// The global compile config was changed via the Graph Config pane.
    pub compile_config: Option<gantz_core::compile::Config>,
    /// The change-tracking validation toggle was changed. Holds the new value.
    pub validate_change_tracking: Option<bool>,
    /// The graph's base source association was changed via the graph config
    /// pane. See [`Gantz::base_sources`].
    pub base_source_changed: Option<(gantz_ca::Head, String)>,
    /// Heads whose graph had a CA-affecting edit this frame from a node UI,
    /// an inspector edit or a structural scene edit. Lets the application
    /// commit and recompile only the changed heads instead of re-hashing
    /// every open graph each frame. May contain duplicates. Treat membership
    /// as a set.
    pub changed_heads: Vec<gantz_ca::Head>,
    /// Dynamic payloads emitted from within the widget tree, tagged with the
    /// emitting head. See [`crate::response`] for the handling contract.
    pub responses: Responses,
    /// Panes currently popped out into windows, reported every frame.
    /// Populated in both [`PaneWindowMode`]s. Under
    /// [`PaneWindowMode::HostNative`] a host diffs this to create, title and
    /// destroy its OS windows and renders each via
    /// [`Gantz::render_windowed_pane`].
    pub windowed_panes: Vec<WindowedPane>,
    /// Per-head node index remappings from this frame's deletions, collected
    /// during traversal and applied to the top-level tree's [`Pane::NodeView`]
    /// paths by `Gantz::show` after layout. Internal scratch that is drained
    /// before the response is returned, so applications can ignore it.
    pub(crate) node_view_reindexes: Vec<(gantz_ca::Head, crate::ops::Reindex)>,
}

/// A pane currently popped out into a window, reported via
/// [`GantzResponse::windowed_panes`].
#[derive(Clone, Debug)]
pub struct WindowedPane {
    /// The pane's identity and payload. Pass it back to
    /// [`Gantz::render_windowed_pane`] to draw it into the host's window.
    pub pane: Pane,
    /// The pane's display title, the same text as its tab.
    pub title: String,
}

/// State for editing a tab name via double-click.
#[derive(Clone, Default)]
struct TabEditState {
    /// The tile currently being edited, if any.
    editing_tile_id: Option<egui_tiles::TileId>,
    /// The text being edited.
    edit_text: String,
    /// Whether to request focus on the next frame.
    request_focus: bool,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ViewToggles {
    /// Whether the sidebar is open. The hamburger toggles it.
    pub sidebar_open: bool,
    pub graphs: bool,
    pub history: bool,
    pub settings: bool,
    pub logs: bool,
    pub node_inspector: bool,
    pub perf_gui: bool,
    pub perf_vm: bool,
    pub steel: bool,
    pub gui_preview: bool,
    pub gui_tree: bool,
    pub graph_config: bool,
    /// Per-[`Pane::Ext`] visibility, keyed by the provider key. A missing
    /// entry means hidden, matching the tray panes' default. Keys of
    /// long-gone providers linger harmlessly.
    pub ext: BTreeMap<String, bool>,
}

impl Default for ViewToggles {
    fn default() -> Self {
        // The sidebar starts closed so a fresh launch shows only the graph
        // scene, but its content panes default to visible so that opening the
        // sidebar reveals the full arrangement. The Logs/Steel tray stays
        // hidden until toggled.
        Self {
            sidebar_open: false,
            graphs: true,
            history: true,
            settings: true,
            logs: false,
            node_inspector: true,
            perf_gui: false,
            perf_vm: false,
            steel: false,
            gui_preview: false,
            gui_tree: false,
            graph_config: true,
            ext: BTreeMap::new(),
        }
    }
}

struct NodeTyCmd<'a> {
    env: &'a Env<'a>,
    name: &'a str,
}

impl GantzResponse {
    /// An empty response for the given focused head.
    fn new(focused_head: usize) -> Self {
        GantzResponse {
            focused_head,
            graph_select: None,
            closed_heads: Vec::new(),
            new_branch: None,
            file_drops: Vec::new(),
            demo_changed: None,
            description_changed: None,
            reset_base_graph: None,
            reset_all_demos: false,
            compile_config: None,
            validate_change_tracking: None,
            base_source_changed: None,
            changed_heads: Vec::new(),
            responses: Responses::default(),
            windowed_panes: Vec::new(),
            node_view_reindexes: Vec::new(),
        }
    }

    /// Indicates the new graph button was clicked.
    pub fn new_graph(&self) -> bool {
        self.graph_select
            .as_ref()
            .map(|g| g.new_graph)
            .unwrap_or(false)
    }

    /// Replace the focused head with this one.
    pub fn graph_replaced(&self) -> Option<&gantz_ca::Head> {
        self.graph_select.as_ref().and_then(|g| g.replaced.as_ref())
    }

    /// Open this head as a new tab, or focus it if already open.
    pub fn graph_opened(&self) -> Option<&gantz_ca::Head> {
        self.graph_select.as_ref().and_then(|g| g.opened.as_ref())
    }

    /// Close this head.
    pub fn graph_closed(&self) -> Option<&gantz_ca::Head> {
        self.graph_select.as_ref().and_then(|g| g.closed.as_ref())
    }

    /// The given graph name was removed.
    pub fn graph_name_removed(&self) -> Option<gantz_ca::Name> {
        self.graph_select
            .as_ref()
            .and_then(|g| g.name_removed.clone())
    }

    /// A new branch created from a tab double-click, as the original head and
    /// the new branch name.
    pub fn new_branch(&self) -> Option<&(gantz_ca::Head, String)> {
        self.new_branch.as_ref()
    }

    /// Indicates the import button was clicked.
    pub fn import(&self) -> bool {
        self.graph_select
            .as_ref()
            .map(|g| g.import)
            .unwrap_or(false)
    }
}

impl<'a> Gantz<'a> {
    /// Instantiate the full top-level gantz widget.
    pub fn new(env: &'a Env<'a>, base_names: &'a crate::reg::Names) -> Self {
        Self {
            env,
            codec: env.codec,
            base_names,
            log_source: None,
            perf_vm: None,
            perf_gui: None,
            base_immutable: true,
            compile_config: None,
            validate_change_tracking: None,
            settings_tabs: &mut [],
            ext_panes: &mut [],
            ref_ext_uis: &[],
            edge_styles: &[],
            base_sources: None,
            pane_window_mode: PaneWindowMode::default(),
            collab: None,
            clipboard: None,
            audio_heads: None,
        }
    }

    /// Provide the open heads whose graph produces audio. Their tabs show a
    /// speaker that toggles [`OpenHeadState::muted`].
    pub fn audio_heads(mut self, heads: &'a HashSet<gantz_ca::Head>) -> Self {
        self.audio_heads = Some(heads);
        self
    }

    /// Provide the collaborative-session display state so the Graph Config
    /// pane shows the collab row for shared or shareable graphs.
    pub fn collab(mut self, collab: &'a crate::collab::CollabUiState) -> Self {
        self.collab = Some(collab);
        self
    }

    /// Provide a clipboard reader for widget paste affordances, for example
    /// the join popup's right-click paste. Ctrl+V works through egui's own
    /// event path regardless.
    pub fn clipboard(mut self, clipboard: &'a dyn Fn() -> Option<String>) -> Self {
        self.clipboard = Some(clipboard);
        self
    }

    /// Choose whether the widget draws popped-out panes as `egui::Window`s,
    /// the default, or leaves them to the host as native OS windows.
    pub fn pane_window_mode(mut self, mode: PaneWindowMode) -> Self {
        self.pane_window_mode = mode;
        self
    }

    /// Provide the current compile config so the Graph Config pane shows
    /// the compile toggles. The config is global. It applies to all open
    /// heads, and a change is reported via [`GantzResponse::compile_config`].
    pub fn compile_config(mut self, config: gantz_core::compile::Config) -> Self {
        self.compile_config = Some(config);
        self
    }

    /// Provide the current change-tracking validation state so the Settings >
    /// Global pane shows its toggle. A change is reported via
    /// [`GantzResponse::validate_change_tracking`].
    ///
    /// When not provided, validation defaults on in debug builds and off in
    /// release. The node instance cache would otherwise mask a missed
    /// `changed` flag.
    pub fn validate_change_tracking(mut self, enabled: bool) -> Self {
        self.validate_change_tracking = Some(enabled);
        self
    }

    /// Provide extension settings subtabs. See
    /// [`SettingsTab`][widget::SettingsTab]. One subtab appears per entry,
    /// and any payloads a tab emits are reported via
    /// [`GantzResponse::responses`].
    pub fn settings_tabs(mut self, tabs: &'a mut [&'a mut dyn widget::SettingsTab]) -> Self {
        self.settings_tabs = tabs;
        self
    }

    /// Provide extension top-level panes. See [`ExtPane`][widget::ExtPane].
    /// One tray tile appears per entry, and any payloads a pane emits are
    /// reported via [`GantzResponse::responses`] tagged with the focused head.
    pub fn ext_panes(mut self, panes: &'a mut [&'a mut dyn widget::ExtPane]) -> Self {
        self.ext_panes = panes;
        self
    }

    /// Provide domain extensions for the `NamedRef` node inspector. See
    /// [`RefExtUi`][crate::node::RefExtUi]. Each applicable extension's rows
    /// are appended after the ref's own inspector rows.
    pub fn ref_ext_uis(mut self, uis: &'a [&'a dyn crate::node::RefExtUi]) -> Self {
        self.ref_ext_uis = uis;
        self
    }

    /// Provide domain edge stylers for the graph scenes. See
    /// [`EdgeStyle`][widget::EdgeStyle]. The first styler returning `Some`
    /// styles each edge. Unclaimed edges keep the default styling.
    pub fn edge_styles(mut self, styles: &'a [&'a dyn widget::EdgeStyle]) -> Self {
        self.edge_styles = styles;
        self
    }

    /// Provide base-source authoring context so the graph config pane shows
    /// a "source" dropdown selecting which base file a graph belongs to. A
    /// change is reported via [`GantzResponse::base_source_changed`].
    pub fn base_sources(mut self, ctx: BaseSourcesCtx<'a>) -> Self {
        self.base_sources = Some(ctx);
        self
    }

    /// Enable the logging window with a basic env logger.
    pub fn logger(mut self, logger: widget::log_view::Logger) -> Self {
        self.log_source = Some(LogSource::Logger(logger));
        self
    }

    /// Enable the logging window for tracking tracing.
    #[cfg(feature = "tracing")]
    pub fn trace_capture(
        mut self,
        trace_capture: widget::trace_view::TraceCapture,
        level: tracing::level_filters::LevelFilter,
    ) -> Self {
        self.log_source = Some(LogSource::TraceCapture(trace_capture, level));
        self
    }

    /// Set the performance capture sources for VM and GUI timing.
    pub fn perf_captures(
        mut self,
        perf_vm: &'a mut widget::PerfCapture,
        perf_gui: &'a mut widget::PerfCapture,
    ) -> Self {
        self.perf_vm = Some(perf_vm);
        self.perf_gui = Some(perf_gui);
        self
    }

    /// Whether base node graphs should be immutable.
    ///
    /// When `true`, the default, graphs for heads whose branch name appears
    /// in `base_names` are shown in immutable mode. Navigation and selection
    /// work, but structural edits are disabled.
    ///
    /// Set to `false` for developer tools like `update-base` that need
    /// to edit base nodes.
    pub fn base_immutable(mut self, base_immutable: bool) -> Self {
        self.base_immutable = base_immutable;
        self
    }

    /// Present the gantz UI.
    ///
    /// The `access` parameter provides access to all open heads and their data.
    /// The `focused_head` is the index of the currently focused head.
    ///
    /// Returns a response containing the possibly updated focused head index.
    pub fn show<'s, Access>(
        mut self,
        state: &'s mut GantzState,
        focused_head: usize,
        access: &'s mut Access,
        ui: &'s mut egui::Ui,
    ) -> GantzResponse
    where
        's: 'a,
        Access: HeadAccess,
    {
        // Honour a pending "clear egui memory" request before loading any
        // persisted UI state this frame.
        if ui
            .ctx()
            .data(|d| d.get_temp::<bool>(clear_egui_memory_id()))
            .unwrap_or(false)
        {
            ui.ctx().memory_mut(|m| m.data.clear());
        }

        crate::style::apply(ui.ctx(), &state.style);

        // The persisted outer tree. The version suffix invalidates any tree
        // persisted before the latest default-layout change, forcing a
        // rebuild via `create_tree`.
        let tree_id = egui::Id::new("gantz-tiles-tree-storage-v5");

        let mut tree: egui_tiles::Tree<Pane> =
            load_tree(ui.ctx(), tree_id).unwrap_or_else(create_tree);

        // The set of panes popped out into windows, persisted alongside the tree.
        let mut windowed: Vec<Pane> = load_ron(ui.ctx(), windowed_panes_id()).unwrap_or_default();

        // Ensure every supplied extension pane has a tile.
        let ext_keys: Vec<&str> = self.ext_panes.iter().map(|p| p.key()).collect();
        sync_ext_panes(&mut tree, &ext_keys);

        // Ensure the GUI Preview and GUI Tree panes have tiles. Trees
        // persisted before they existed lack them.
        sync_singleton_pane(&mut tree, Pane::GuiPreview);
        sync_singleton_pane(&mut tree, Pane::GuiTree);

        // Check the `view_toggles` match the pane visibility.
        set_tile_visibility(&mut tree, &state.view_toggles);

        // Simplify the tree, and ensure tabs are where they should be.
        simplify_tree(&mut tree, ui.ctx());

        // Maintain a fixed sidebar width and tray height across window resizes
        // by imposing the stored pixel sizes on the share splits before layout.
        // `available_rect_before_wrap` matches the root rect `Tree::ui` uses.
        let widget_area = ui.available_rect_before_wrap();
        impose_fixed_sizes(&mut tree, state, widget_area);

        let mut response = GantzResponse::new(focused_head);

        let base_names = self.base_names;
        let mut behaviour = TreeBehaviour {
            gantz: &mut self,
            state: &mut *state,
            access: &mut *access,
            focused_head,
            base_names,
            gantz_response: &mut response,
            windowed: &mut windowed,
        };
        tree.ui(&mut behaviour, ui);

        response.focused_head = behaviour.focused_head;

        // Capture the sidebar width and tray height from the laid-out tree,
        // which reflects any manual divider drag, to re-impose next frame.
        capture_fixed_sizes(&tree, state, widget_area);

        // Detect .gantz file drops globally rather than per pane, since the
        // pointer position may be unavailable during OS file drags on some
        // platforms.
        response.file_drops = collect_gantz_file_drops(ui.ctx());

        // Apply payloads that only affect the widget's own state, so
        // applications never see them. These are node palette toggling and
        // resetting the tile layout to its default arrangement.
        for _ in response.responses.take::<OpenNodePalette>() {
            state.node_palette.toggle();
        }
        for _ in response.responses.take::<ResetTilesLayout>() {
            tree = create_tree();
            // Restore the default sidebar width and tray height too.
            state.sidebar_width = default_sidebar_width();
            state.tray_height = default_tray_height();
            // Restore default pane visibility, but keep the sidebar's open
            // state since the user just acted from within it.
            let sidebar_open = state.view_toggles.sidebar_open;
            state.view_toggles = ViewToggles::default();
            state.view_toggles.sidebar_open = sidebar_open;
        }
        for _ in response.responses.take::<OpenLogs>() {
            state.view_toggles.logs = true;
        }
        // Migrate node-view tiles past this frame's node deletions. They were
        // collected during traversal, since the live top-level tree was not
        // reachable then.
        for (head, reindex) in &response.node_view_reindexes {
            migrate_node_view_paths(&mut tree, head, reindex);
            migrate_windowed_node_views(&mut windowed, head, reindex);
        }
        // Add or focus a node-view tile in the top-level tree. The node's head
        // is the payload's head tag.
        for (head, view) in response.responses.take::<OpenNodeView>() {
            let Some(head) = head else { continue };
            // Skip if this view is already popped out into a window.
            let windowed_already = windowed.iter().any(
                |p| matches!(p, Pane::NodeView(np) if np.head == head && np.path == view.path),
            );
            if windowed_already {
                continue;
            }
            add_node_view_pane(&mut tree, head, view.path, view.ty_name);
        }

        // Reconcile the windowed set. A singleton whose toggle was turned back
        // on re-docks, so drop it here. Node views stay windowed until their
        // window is closed.
        windowed
            .retain(|p| matches!(p, Pane::NodeView(_)) || !pane_is_visible(&state.view_toggles, p));

        // Redock and close intents queued by a native host between frames.
        // Its OS-window buttons call `redock_windowed_pane` and
        // `close_windowed_pane`.
        let mut redock: Vec<Pane> = drain_pending(ui.ctx(), pending_redock_id());
        let close: Vec<Pane> = drain_pending(ui.ctx(), pending_close_id());

        // In the egui-window backend, draw each windowed pane as a floating
        // `egui::Window`. Its title-bar close returns the pane to the tile
        // tree. Under `HostNative` the host draws them as OS windows instead.
        if self.pane_window_mode == PaneWindowMode::EguiWindow {
            for pane in &mut windowed {
                let title = pane_title(&self, &*access, focused_head, pane);
                let mut open = true;
                egui::Window::new(title)
                    .id(egui::Id::new(("gantz-windowed-pane", pane_key(pane))))
                    .open(&mut open)
                    .show(ui.ctx(), |ui| {
                        // Namespace child ids so a windowed pane never collides
                        // with its briefly still-visible docked instance.
                        ui.push_id(("gantz-windowed", pane_key(pane)), |ui| {
                            let mut cx = PaneCtx {
                                gantz: &mut self,
                                state: &mut *state,
                                access: &mut *access,
                                focused_head,
                                base_names,
                                response: &mut response,
                            };
                            render_pane(&mut cx, ui, pane);
                        });
                    });
                if !open {
                    redock.push(pane.clone());
                }
            }
        }

        // Apply redocks, which return to the tree, and closes, which destroy
        // node views. Singletons have no destroy, so re-dock them.
        for pane in redock {
            let key = pane_key(&pane);
            windowed.retain(|p| pane_key(p) != key);
            redock_pane(&mut state.view_toggles, &mut tree, pane);
        }
        for pane in close {
            let key = pane_key(&pane);
            windowed.retain(|p| pane_key(p) != key);
            if !matches!(pane, Pane::NodeView(_)) {
                redock_pane(&mut state.view_toggles, &mut tree, pane);
            }
        }

        // Report the final windowed set so a native host can match its OS windows.
        response.windowed_panes = windowed
            .iter()
            .map(|pane| WindowedPane {
                pane: pane.clone(),
                title: pane_title(&self, &*access, focused_head, pane),
            })
            .collect();

        store_tree(ui.ctx(), tree_id, &tree);
        store_ron(ui.ctx(), windowed_panes_id(), &windowed);

        response
    }

    /// Render a single popped-out pane into `ui`, typically a native host's
    /// OS-window egui context under [`PaneWindowMode::HostNative`].
    ///
    /// Mirrors [`Self::show`]'s per-pane rendering. The returned
    /// [`GantzResponse`] carries this pane's `changed_heads` and payloads.
    /// Apply it exactly like `show`'s response.
    pub fn render_windowed_pane<'s, Access>(
        mut self,
        state: &'s mut GantzState,
        focused_head: usize,
        access: &'s mut Access,
        pane: &mut Pane,
        ui: &mut egui::Ui,
    ) -> GantzResponse
    where
        's: 'a,
        Access: HeadAccess,
    {
        crate::style::apply(ui.ctx(), &state.style);
        let mut response = GantzResponse::new(focused_head);
        let base_names = self.base_names;
        let mut cx = PaneCtx {
            gantz: &mut self,
            state,
            access,
            focused_head,
            base_names,
            response: &mut response,
        };
        render_pane(&mut cx, ui, pane);
        response
    }
}

impl GantzState {
    pub const DEFAULT_DIRECTION: egui::Direction = egui::Direction::TopDown;

    /// Shorthand for initialising graph state with no initial layout, so the
    /// first pass determines the layout automatically.
    pub fn new() -> Self {
        Self::from_open_heads(Default::default())
    }

    pub fn from_open_heads(open_heads: OpenHeadStates) -> Self {
        Self {
            open_heads,
            node_palette: widget::NodePalette::default(),
            view_toggles: ViewToggles::default(),
            layout_config: LayoutConfig::default(),
            scene_config: SceneConfig::default(),
            style: Default::default(),
            keymap: Keymap::default(),
            collab: Default::default(),
            merge_resolutions: Default::default(),
            redo_stacks: HashMap::new(),
            undo_cursors: HashMap::new(),
            sidebar_width: default_sidebar_width(),
            tray_height: default_tray_height(),
            windowed_geometry: HashMap::new(),
        }
    }

    /// Migrate GUI state when a head's identity changes.
    ///
    /// Moves the `open_heads` entry from the old to the new key. When
    /// `clear_redo` is true, removes redo stacks for both keys, since a new
    /// edit commit invalidates them. Otherwise migrates the redo stack to the
    /// new key.
    pub fn migrate_head(&mut self, old: &gantz_ca::Head, new: &gantz_ca::Head, clear_redo: bool) {
        if let Some(state) = self.open_heads.remove(old) {
            self.open_heads.insert(new.clone(), state);
        }
        migrate_or_clear(&mut self.redo_stacks, old, new, clear_redo);
        migrate_or_clear(&mut self.undo_cursors, old, new, clear_redo);
    }
}

/// Migrate one per-head map entry across a head-identity change. When
/// `clear`, a new edit invalidates it, so it is cleared for both keys.
/// Otherwise it moves to the new key.
fn migrate_or_clear<V>(
    map: &mut HashMap<gantz_ca::Head, V>,
    old: &gantz_ca::Head,
    new: &gantz_ca::Head,
    clear: bool,
) {
    if clear {
        map.remove(old);
        map.remove(new);
    } else if let Some(v) = map.remove(old) {
        map.insert(new.clone(), v);
    }
}

impl<'a, 's, Access> egui_tiles::Behavior<Pane> for TreeBehaviour<'a, 's, Access>
where
    Access: HeadAccess,
{
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        pane_title(self.gantz, self.access, self.focused_head, pane).into()
    }

    fn on_tab_button(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
        button_response: egui::Response,
    ) -> egui::Response {
        // Right-click a tab for pane actions. Hideable panes offer hide. Any
        // pane but the graph scene offers pop out into a window.
        let pane = match tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(pane))
                if pane_is_hideable(pane) || pane_is_poppable(pane) =>
            {
                pane.clone()
            }
            _ => return button_response,
        };
        button_response.context_menu(|ui| {
            if pane_is_hideable(&pane) && ui.button("hide").clicked() {
                set_pane_visible(&mut self.state.view_toggles, &pane, false);
                ui.close();
            }
            if pane_is_poppable(&pane) && ui.button("pop out to window").clicked() {
                detach_pane(
                    &mut self.state.view_toggles,
                    self.windowed,
                    tiles,
                    tile_id,
                    &pane,
                );
                ui.close();
            }
        });
        button_response
    }

    fn is_tab_closable(
        &self,
        tiles: &egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        // The tray panes and detached node views get a close button. The rest
        // are toggled via the Panes settings or the tab right-click menu.
        matches!(
            tiles.get_pane(&tile_id),
            Some(Pane::Logs | Pane::Steel | Pane::GuiPreview | Pane::GuiTree | Pane::NodeView(_))
        )
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        match tiles.get_pane(&tile_id) {
            // Node views are user-created tiles. Closing removes them entirely.
            Some(Pane::NodeView(_)) => true,
            // Hide other panes via their toggle so they can be reopened, rather
            // than letting egui_tiles remove the tile from the tree.
            Some(pane) => {
                let pane = pane.clone();
                set_pane_visible(&mut self.state.view_toggles, &pane, false);
                false
            }
            None => false,
        }
    }

    fn tab_ui(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        ui: &mut egui::Ui,
        id: egui::Id,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        // Render with the shared `Tab` widget so the sidebar and tray tabs and
        // their small close button match the graph tabs.
        let title = self.tab_title_for_tile(tiles, tile_id);
        let mut tab = widget::Tab::new(title, id)
            .active(state.active)
            .closable(state.closable);
        // The GUI panes pick the `gui` role they show from a badge on their
        // tab, so their bodies stay free of controls.
        let badge = match tiles.get_pane(&tile_id) {
            Some(Pane::GuiPreview | Pane::GuiTree) => {
                gui_role_badge(self.access, self.focused_head, ui.ctx())
            }
            _ => None,
        };
        if let Some((_, _, role)) = &badge {
            tab = tab.badge(format!("[{}]", role.as_str()));
        }
        let res = tab.show(ui);
        if let (Some((head, roles, role)), Some(badge_res)) = (badge, &res.badge) {
            egui::Popup::menu(badge_res)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    for r in roles {
                        if ui.selectable_label(r == role, r.as_str()).clicked() {
                            set_gui_role(ui.ctx(), &head, r);
                            ui.close();
                        }
                    }
                });
        }
        if res.close.is_some_and(|r| r.clicked()) && self.on_tab_close(tiles, tile_id) {
            tiles.remove(tile_id);
        }
        // Preserve the right-click-to-hide menu.
        self.on_tab_button(tiles, tile_id, res.tab)
    }

    fn tab_bar_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        // This matches the `CentralPanel` fill so that the color looks
        // continuous.
        visuals.panel_fill
    }

    fn resize_stroke(
        &self,
        style: &egui::Style,
        resize_state: egui_tiles::ResizeState,
    ) -> egui::Stroke {
        separator_stroke(self.state.style.separator, style, resize_state)
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        self.state.style.separator.width
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<Pane>,
        _tile_id: egui_tiles::TileId,
        _state: &egui_tiles::TabState,
    ) -> egui::Stroke {
        egui::Stroke::NONE
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        // The tree is simplified manually before `tree.ui`. See `simplify_tree`.
        egui_tiles::SimplificationOptions::OFF
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut Pane,
    ) -> egui_tiles::UiResponse {
        let mut cx = PaneCtx {
            gantz: &mut *self.gantz,
            state: &mut *self.state,
            access: &mut *self.access,
            focused_head: self.focused_head,
            base_names: self.base_names,
            response: &mut *self.gantz_response,
        };
        render_pane(&mut cx, ui, pane);
        // Propagate a focus change made while rendering. The graph scene
        // changes focus on graph-tab clicks.
        self.focused_head = cx.focused_head;
        egui_tiles::UiResponse::None
    }
}

/// Render a single pane into `ui`.
///
/// Shared by the tile tree in [`TreeBehaviour::pane_ui`], floating
/// `egui::Window`s and a native host's OS windows, so a pane looks and
/// behaves identically wherever it is shown.
fn render_pane<Access>(cx: &mut PaneCtx<'_, '_, Access>, ui: &mut egui::Ui, pane: &mut Pane)
where
    Access: HeadAccess,
{
    let PaneCtx {
        gantz,
        state,
        access,
        focused_head,
        base_names,
        response: gantz_response,
    } = cx;
    match pane {
        Pane::Ext(key) => {
            let focused = access.heads().get(*focused_head).cloned();
            // The focused head's selection as sorted root-level node ids,
            // mirroring the Steel pane's span-highlight input.
            let mut selection: Vec<node::Id> = focused
                .as_ref()
                .and_then(|h| state.open_heads.get(h))
                .map(|hs| {
                    hs.scene
                        .interaction
                        .selection
                        .nodes
                        .iter()
                        .map(|n| n.index())
                        .collect()
                })
                .unwrap_or_default();
            selection.sort_unstable();
            match gantz.ext_panes.iter_mut().find(|p| p.key() == *key) {
                Some(p) => {
                    let cx = widget::ExtPaneCtx {
                        focused: focused.as_ref(),
                        selection: &selection,
                    };
                    let res = pane_ui(ui, |ui| p.ui(cx, ui));
                    let mut ext_responses = res.inner;
                    gantz_response
                        .responses
                        .extend(focused.as_ref(), ext_responses.drain().map(|(_, d)| d));
                }
                None => {
                    ui.centered_and_justified(|ui| {
                        ui.weak(format!("'{key}' pane unavailable (no provider)"));
                    });
                }
            }
        }
        Pane::GraphConfig => match access.heads().get(*focused_head).cloned() {
            Some(head) => {
                let merge_resolutions = &mut state.merge_resolutions;
                let head_state = state.open_heads.entry(head.clone()).or_default();
                let names = crate::reg::names(gantz.env.registry);
                let is_base = match &head {
                    gantz_ca::Head::Branch(name) => base_names.contains_key(name),
                    _ => false,
                };
                let immutable = head_immutable(&head, gantz.base_immutable, base_names);

                // Collect demo-* names for the dropdown.
                let demo_names: Vec<String> = names
                    .iter()
                    .filter(|(n, _)| widget::graph_select::is_demo(n))
                    .map(|(n, _)| n.to_string())
                    .collect();
                let demo_names_vec: Vec<&str> = demo_names.iter().map(|s| s.as_str()).collect();

                // Look up the current demo association for this head.
                let current_demo = match &head {
                    gantz_ca::Head::Branch(name) => gantz.env.demo_graph(&name.to_string()),
                    _ => None,
                };

                // The graph's current description. Only named graphs have one.
                let current_description = match &head {
                    gantz_ca::Head::Branch(name) => {
                        crate::section::description(gantz.env.registry, name)
                    }
                    _ => None,
                };

                // The head's session display, when a collab layer is wired.
                let session = gantz.collab.map(|c| match &head {
                    gantz_ca::Head::Branch(name) => c.sessions.get(name),
                    _ => None,
                });

                let res = pane_ui(ui, |ui| {
                    let mut config = widget::GraphConfig::new(&head, head_state, &names)
                        .is_base(is_base)
                        .immutable(immutable)
                        .demo_names(&demo_names_vec)
                        .current_demo(current_demo.as_deref())
                        .current_description(current_description.as_deref())
                        .merge_env(gantz.env, merge_resolutions);
                    // The base source dropdown, when authoring context was
                    // supplied. See `Gantz::base_sources`.
                    if let (Some(ctx), gantz_ca::Head::Branch(name)) = (&gantz.base_sources, &head)
                    {
                        let current = ctx
                            .name_sources
                            .get(&name.to_string())
                            .copied()
                            .unwrap_or(ctx.default_source);
                        config = config.base_sources(ctx.sources, Some(current));
                    }
                    if let Some(session) = session {
                        config = config.collab(session);
                    }
                    config.show(ui)
                });
                if res.inner.new_branch.is_some() {
                    gantz_response.new_branch = res.inner.new_branch;
                }
                if let Some(merge) = res.inner.merge {
                    gantz_response.responses.push(Some(head.clone()), merge);
                }
                if res.inner.share {
                    gantz_response
                        .responses
                        .push(Some(head.clone()), crate::ShareHead { public: true });
                }
                if res.inner.stop_sharing {
                    gantz_response
                        .responses
                        .push(Some(head.clone()), crate::StopSharing);
                }
                if let Some(demo_val) = res.inner.demo_changed {
                    gantz_response.demo_changed = Some((head.clone(), demo_val));
                }
                if let Some(description) = res.inner.description_changed {
                    gantz_response.description_changed = Some((head.clone(), description));
                }
                if res.inner.reset_base_graph {
                    gantz_response.reset_base_graph = Some(head.clone());
                }
                if let Some(source) = res.inner.base_source_changed {
                    gantz_response.base_source_changed = Some((head.clone(), source));
                }
                if res.inner.export {
                    gantz_response.responses.push(Some(head), ExportHead);
                }
            }
            None => {
                pane_ui(ui, |ui| {
                    ui.label("No graph focused");
                });
            }
        },
        Pane::GraphScene => {
            paint_gantz_file_hover_overlay(ui);

            // The rect positions the floating sidebar toggle.
            let rect = ui.available_rect_before_wrap();

            // Extension-pane toggle entries for the graph-area context menu.
            let ext_panes = ext_pane_entries(gantz);

            let graph_tree_id = egui::Id::new(GRAPH_TREE_ID);
            let mut graph_tree: egui_tiles::Tree<GraphPane> =
                load_tree(ui.ctx(), graph_tree_id).unwrap_or_else(create_empty_graph_tree);

            sync_graph_panes(&mut graph_tree, access.heads());

            // Activate the tab corresponding to the focused head.
            if let Some(fh) = access.heads().get(*focused_head) {
                let fh = fh.clone();
                graph_tree.make_active(|_, tile| match tile {
                    egui_tiles::Tile::Pane(GraphPane(head)) => *head == fh,
                    _ => false,
                });
            }

            let mut graph_behaviour = GraphTreeBehaviour {
                env: gantz.env,
                codec: gantz.codec,
                access: *access,
                state,
                focused_head,
                closed_heads: &mut gantz_response.closed_heads,
                new_branch: &mut gantz_response.new_branch,
                responses: &mut gantz_response.responses,
                changed_heads: &mut gantz_response.changed_heads,
                reindexes: &mut gantz_response.node_view_reindexes,
                base_names,
                base_immutable: gantz.base_immutable,
                // With the instance cache, an unmarked weight mutation
                // persists in the cached instance instead of visibly
                // reverting next pass, so debug builds default the validator
                // on to keep the contract violation loud.
                validate_change_tracking: gantz
                    .validate_change_tracking
                    .unwrap_or(cfg!(debug_assertions)),
                ext_panes: &ext_panes,
                edge_styles: gantz.edge_styles,
                collab: gantz.collab,
                audio_heads: gantz.audio_heads,
            };
            graph_tree.ui(&mut graph_behaviour, ui);

            store_tree(ui.ctx(), graph_tree_id, &graph_tree);

            // Show the node palette once rather than per pane, operating on
            // the focused head.
            if let Some(fh) = access.heads().get(*focused_head).cloned() {
                let focused_immutable = head_immutable(&fh, gantz.base_immutable, base_names);

                let head_state = state.open_heads.entry(fh.clone()).or_default();

                // Command keyboard shortcuts, sourced from the keymap.
                if !ui.ctx().egui_wants_keyboard_input() {
                    let keymap = &state.keymap;
                    // Copy is always allowed.
                    if keymap.consume(ui, Action::Copy) {
                        let nodes = head_state.scene.interaction.selection.nodes.clone();
                        gantz_response
                            .responses
                            .push(Some(fh.clone()), CopyNodes(nodes));
                    }
                    if keymap.consume(ui, Action::NewGraph) {
                        let gs = gantz_response
                            .graph_select
                            .get_or_insert_with(Default::default);
                        gs.new_graph = true;
                    }
                    // Paste, undo, redo are gated by immutable.
                    if !focused_immutable {
                        // Detect paste from an `Event::Paste` or the Paste
                        // shortcut. eframe and web send `Event::Paste`.
                        // bevy_egui desktop sends `Event::Text` instead.
                        let paste_text = ui.input(|i| {
                            i.events.iter().find_map(|e| match e {
                                egui::Event::Paste(s) => Some(s.clone()),
                                _ => None,
                            })
                        });
                        if paste_text.is_some() || keymap.consume(ui, Action::Paste) {
                            let paste = Paste {
                                text: paste_text,
                                pos: crate::PastePos::Offset(egui::vec2(20.0, 20.0)),
                            };
                            gantz_response.responses.push(Some(fh.clone()), paste);
                        }
                        // Redo before Undo. `consume_shortcut` matches
                        // modifiers logically, so `Cmd+Z` also matches a
                        // `Cmd+Shift+Z` event. Check and consume the more
                        // specific binding first.
                        if keymap.consume(ui, Action::Redo) {
                            gantz_response.responses.push(Some(fh.clone()), Redo);
                        }
                        if keymap.consume(ui, Action::Undo) {
                            gantz_response.responses.push(Some(fh.clone()), Undo);
                        }
                        // Cut copies the selection, then removes it.
                        if keymap.consume(ui, Action::Cut) {
                            let nodes = head_state.scene.interaction.selection.nodes.clone();
                            gantz_response
                                .responses
                                .push(Some(fh.clone()), CutNodes(nodes));
                        }
                        if keymap.consume(ui, Action::Duplicate) {
                            let nodes = head_state.scene.interaction.selection.nodes.clone();
                            gantz_response
                                .responses
                                .push(Some(fh.clone()), DuplicateNodes(nodes));
                        }
                    }
                }

                // Skip node palette when immutable.
                if !focused_immutable {
                    let editing_name = match &fh {
                        gantz_ca::Head::Branch(name) => Some(name.to_string()),
                        _ => None,
                    };
                    let editing = editing_name.as_deref();
                    // The pointer position over the focused head's scene in
                    // graph coords recorded this frame. New nodes are placed
                    // here. It is `Copy`, so no borrow is held across the call.
                    let pointer_pos = head_state.scene.interaction.last_pointer_pos;
                    let created = node_palette(
                        gantz.env,
                        editing,
                        &mut state.node_palette,
                        &state.keymap,
                        ui,
                    );
                    match created {
                        Some(PaletteChoice::Node(mut create)) => {
                            create.pos = pointer_pos;
                            gantz_response.responses.push(Some(fh), create);
                        }
                        Some(PaletteChoice::NestedGraph(mut create)) => {
                            create.pos = pointer_pos;
                            gantz_response.responses.push(Some(fh), create);
                        }
                        None => {}
                    }
                }
            }

            // Floating hamburger over the bottom-left corner of the graph
            // scene that opens and closes the sidebar.
            let space = ui.style().interaction.interact_radius * 3.0;
            let anchor = rect.left_bottom() + egui::vec2(space, -space);
            sidebar_toggle(ui.ctx(), anchor, &mut state.view_toggles.sidebar_open);
        }
        Pane::Graphs => {
            // Store the pane rect for file drop targeting.
            ui.ctx().memory_mut(|m| {
                m.data
                    .insert_temp(egui::Id::new(GRAPHS_PANE_RECT_ID), ui.max_rect())
            });
            paint_gantz_file_hover_overlay(ui);

            let heads = access.heads();
            let mut res = graph_select(
                gantz.env,
                heads,
                *focused_head,
                *base_names,
                gantz.collab,
                gantz.clipboard,
                ui,
            );

            if res.inner.export_all {
                gantz_response.responses.push(None, ExportAllNamed);
            }
            if let Some(ticket) = res.inner.join_ticket.take() {
                gantz_response
                    .responses
                    .push(None, crate::JoinSession { ticket });
            }
            match &mut gantz_response.graph_select {
                Some(gs) => *gs |= res.inner,
                None => gantz_response.graph_select = Some(res.inner),
            }
        }
        Pane::GuiPerf => {
            if let Some(ref mut capture) = gantz.perf_gui {
                perf_view("GUI Perf", capture, ui);
            }
        }
        Pane::History => {
            let heads = access.heads();
            let res = history_view(gantz.env, heads, *focused_head, ui);
            match &mut gantz_response.graph_select {
                Some(gs) => *gs |= res.inner,
                None => gantz_response.graph_select = Some(res.inner),
            }
        }
        Pane::Logs => match &gantz.log_source {
            None => (),
            Some(LogSource::Logger(logger)) => {
                // Resolve labels for entries emitted by nodes of the
                // focused head. The target encodes the node's path.
                let focused = access.heads().get(*focused_head).cloned();
                let mut labels: HashMap<Vec<node::Id>, String> = HashMap::new();
                if let Some(fh) = &focused {
                    let paths: BTreeSet<Vec<node::Id>> = logger
                        .get_entries()
                        .iter()
                        .filter_map(|e| gantz_std::log::parse_log_target(&e.target))
                        .collect();
                    if !paths.is_empty() {
                        let env = gantz.env;
                        let codec = gantz.codec;
                        access.with_head_mut(fh, |data| {
                            for path in paths {
                                // Log targets are state paths. Only root-level
                                // single-segment ones name a node in this graph.
                                let [ix] = path[..] else { continue };
                                let Some(weight) =
                                    data.graph.node_weight(graph_scene::NodeIndex::new(ix))
                                else {
                                    continue;
                                };
                                let Ok(inst) = codec.reify_ui(weight) else {
                                    continue;
                                };
                                labels.insert(path, inst.node.name(env).to_string());
                            }
                        });
                    }
                }
                let res = log_view(logger, &labels, ui);
                // Clicking an entry selects its node. Only root-level nodes
                // live in the focused head, so entries from a nested graph
                // are skipped.
                if let (Some(path), Some(fh)) = (res.inner.clicked_path, focused) {
                    if let [node_id] = path[..] {
                        let head_state = state.open_heads.entry(fh.clone()).or_default();
                        let selection = &mut head_state.scene.interaction.selection;
                        selection.clear();
                        selection.nodes.insert(graph_scene::NodeIndex::new(node_id));
                    }
                }
            }
            #[cfg(feature = "tracing")]
            Some(LogSource::TraceCapture(trace_capture, level)) => {
                trace_view(trace_capture, *level, ui);
            }
        },
        Pane::NodeInspector => {
            if let Some(fh) = access.heads().get(*focused_head).cloned() {
                let immutable = head_immutable(&fh, gantz.base_immutable, base_names);
                let head_state = state.open_heads.entry(fh.clone()).or_default();
                let ref_ext_uis = gantz.ref_ext_uis;
                let codec = gantz.codec;
                let result = access.with_head_mut(&fh, |data| {
                    node_inspector(
                        gantz.env,
                        codec,
                        data.graph,
                        data.instances,
                        data.vm,
                        head_state,
                        &fh,
                        immutable,
                        ref_ext_uis,
                        ui,
                    )
                    .inner
                });
                if let Some((changed, payloads)) = result {
                    if changed {
                        gantz_response.changed_heads.push(fh.clone());
                    }
                    gantz_response.responses.extend(Some(&fh), payloads);
                }
            }
        }
        Pane::NodeView(view) => {
            // A detached node view renders the node's `view_ui` against its
            // head's live graph and VM, as a mirror sharing state with the
            // in-graph node. A `CentralPanel` gives it the same background as
            // the other panes. No-margin views such as plot drop the pane
            // margin so they fill edge to edge. A placeholder shows when the
            // head is closed.
            let head = view.head.clone();
            let path = view.path.clone();
            let codec = gantz.codec;
            let no_margin = access
                .with_head_mut(&head, |data| {
                    let &[ix] = path.as_slice() else {
                        return false;
                    };
                    data.graph
                        .node_weight(graph_scene::NodeIndex::new(ix))
                        .is_some_and(|w| match data.instances.peek(ix, w) {
                            Some(inst) => inst.node.view_no_margin(),
                            None => codec
                                .reify_ui(w)
                                .is_ok_and(|inst| inst.node.view_no_margin()),
                        })
                })
                .unwrap_or(false);
            let mut frame = egui::Frame::central_panel(ui.style());
            if no_margin {
                frame.inner_margin = egui::Margin::ZERO;
            }
            egui::CentralPanel::default()
                .frame(frame)
                .show_inside(ui, |ui| {
                    if !access.heads().iter().any(|h| h == &head) {
                        ui.centered_and_justified(|ui| {
                            ui.weak("node's graph is not open");
                        });
                        return;
                    }
                    let env = gantz.env;
                    // VM-state writes recorded by the node's `NodeCtx`.
                    let mut writes = Vec::new();
                    // Scope child widget ids by head and path so views never
                    // share ids with each other or the in-graph node.
                    let result = ui
                        .push_id((&head, &path), |ui| {
                            access.with_head_mut(&head, |data| {
                                let &[n_ix] = path.as_slice() else {
                                    return None;
                                };
                                let (inlets, outlets) = crate::inlet_outlet_ids(env, data.graph);
                                let n_id = graph_scene::NodeIndex::new(n_ix);
                                let weight = data.graph.node_weight(n_id)?;
                                // Take the one node's cached instance. See
                                // `graph_scene::nodes`. Panes render
                                // sequentially, so each take and put pair
                                // completes within its site. Erase back only
                                // when changed, updating the witness.
                                let mut entry = data.instances.take(codec, n_ix, weight).ok()?;
                                let ctx = NodeCtx::new(
                                    env,
                                    data.graph,
                                    &path,
                                    &inlets,
                                    &outlets,
                                    &[],
                                    data.vm,
                                    &mut writes,
                                );
                                let r = entry.inst.node.view_ui(ctx, ui);
                                if r.changed {
                                    match entry.inst.erase() {
                                        Ok(node_data) => {
                                            entry.src = node_data.clone();
                                            data.graph[n_id] = node_data;
                                            data.instances.put(n_ix, entry);
                                        }
                                        Err(e) => log::error!(
                                            "node view {n_ix}: failed to erase edited node, \
                                             edit dropped: {e}"
                                        ),
                                    }
                                } else {
                                    data.instances.put(n_ix, entry);
                                }
                                Some((r.changed, r.payloads))
                            })
                        })
                        .inner;
                    match result {
                        Some(Some((changed, payloads))) => {
                            if changed {
                                gantz_response.changed_heads.push(head.clone());
                            }
                            gantz_response.responses.extend(Some(&head), payloads);
                            gantz_response
                                .responses
                                .extend(Some(&head), crate::action::state_written(&mut writes));
                        }
                        _ => {
                            // Head open but node missing at `path`, for example
                            // removed this frame before migration drops the
                            // view.
                            ui.centered_and_justified(|ui| {
                                ui.weak("node not found");
                            });
                        }
                    }
                });
        }
        Pane::Steel => {
            // Use the focused head's compiled module, highlighting the
            // selected nodes' emitted fns and call sites and any diagnostic
            // spans. A failed compile's error renders above the code.
            let focused = access.heads().get(*focused_head).cloned();
            let compile_error = focused.as_ref().and_then(|h| access.compile_error(h));
            let compiled_steel = focused
                .as_ref()
                .and_then(|h| access.module(h))
                .map(|m| m.src.as_str())
                .unwrap_or("");
            let mut highlights: Vec<std::ops::Range<usize>> = vec![];
            let mut scroll_to = None;
            let mut errors: Vec<std::ops::Range<usize>> = vec![];
            if let Some(h) = &focused {
                errors = access
                    .diagnostics(h)
                    .iter()
                    .filter_map(|d| d.span.clone())
                    .collect();
                let head_state = state.open_heads.get(h);
                if let (Some(module), Some(head_state)) = (access.module(h), head_state) {
                    let mut selected: Vec<node::Id> = head_state
                        .scene
                        .interaction
                        .selection
                        .nodes
                        .iter()
                        .map(|n| n.index())
                        .collect();
                    selected.sort_unstable();
                    for &ix in &selected {
                        // A node at this root level has the single-element
                        // path `[ix]` in the compiled module's source map.
                        let spans = module.map.node_spans(&[ix]);
                        highlights.extend(spans.defs);
                        highlights.extend(spans.refs);
                    }
                    // Scroll to the first highlighted span when the
                    // selection changes.
                    let state_id = egui::Id::new("steel_view_selection");
                    let current = egui::Id::new(("steel_sel", h, &selected));
                    let prev: Option<egui::Id> = ui.ctx().data(|d| d.get_temp(state_id));
                    if prev != Some(current) {
                        ui.ctx().data_mut(|d| d.insert_temp(state_id, current));
                        scroll_to = highlights.iter().map(|r| r.start).min();
                    }
                }
            }
            steel_view(
                compiled_steel,
                compile_error,
                &highlights,
                &errors,
                scroll_to,
                ui,
            );
        }
        Pane::GuiPreview => {
            // The focused graph's own GUI, rendered live against the head's
            // VM. Bindings are correct by construction here, since the tree
            // is this graph's GUI.
            let Some(head) = access.heads().get(*focused_head).cloned() else {
                pane_ui(ui, |ui| ui.weak("no focused graph"));
                return;
            };
            let env = gantz.env;
            let codec = gantz.codec;
            let payloads = pane_ui(ui, |ui| {
                access.with_head_mut(&head, |data| {
                    let (role, val) = gui_marker_tree(&head, data.graph, data.vm, ui)?;
                    let decoded =
                        gantz_ui::codec::steel::decode(&val, &gantz_ui::Limits::default());
                    let payloads = match role {
                        // The node-body surfaces preview inside the node
                        // chrome, as a reference to this graph shows them.
                        crate::node::GuiRole::Body | crate::node::GuiRole::Compact => {
                            gui_preview_node(env, codec, &head, data.graph, data.vm, &decoded, ui)
                        }
                        crate::node::GuiRole::View | crate::node::GuiRole::Inspector => {
                            egui::ScrollArea::both()
                                .show(ui, |ui| {
                                    gui_preview_tree(
                                        env, codec, &head, data.graph, data.vm, &decoded, ui,
                                    )
                                })
                                .inner
                        }
                    };
                    Some(payloads)
                })
            })
            .inner;
            if let Some(payloads) = payloads.flatten() {
                gantz_response.responses.extend(Some(&head), payloads);
            }
        }
        Pane::GuiTree => {
            // The stored tree of the focused graph's gui marker as scheme
            // text, with its decode warnings below.
            let Some(head) = access.heads().get(*focused_head).cloned() else {
                pane_ui(ui, |ui| ui.weak("no focused graph"));
                return;
            };
            pane_ui(ui, |ui| {
                access.with_head_mut(&head, |data| {
                    let Some((_, val)) = gui_marker_tree(&head, data.graph, data.vm, ui) else {
                        return;
                    };
                    let text =
                        gantz_ui::pretty(&gantz_ui::codec::steel::lower(&val), GUI_TREE_WIDTH);
                    let decoded =
                        gantz_ui::codec::steel::decode(&val, &gantz_ui::Limits::default());
                    egui::ScrollArea::both().show(ui, |ui| {
                        widget::SteelView::new(&text).show(ui);
                        gui_tree_warnings(&decoded, ui);
                    });
                });
            });
        }
        Pane::VmPerf => {
            if let Some(ref mut capture) = gantz.perf_vm {
                perf_view("VM Perf", capture, ui);
            }
        }
        Pane::Settings => {
            let compile_config = gantz.compile_config;
            let validate_change_tracking = gantz.validate_change_tracking;
            let ext_panes = ext_pane_entries(gantz);
            let ext_tabs = &mut *gantz.settings_tabs;
            let res = pane_ui(ui, |ui| {
                widget::settings(
                    &mut state.view_toggles,
                    compile_config,
                    validate_change_tracking,
                    &mut state.layout_config,
                    &mut state.scene_config,
                    &mut state.style,
                    &mut state.keymap,
                    ext_tabs,
                    &ext_panes,
                    ui,
                )
            });
            if let Some(cfg) = res.inner.compile_config {
                gantz_response.compile_config = Some(cfg);
            }
            if let Some(v) = res.inner.validate_change_tracking {
                gantz_response.validate_change_tracking = Some(v);
            }
            if res.inner.reset_all_demos {
                gantz_response.reset_all_demos = true;
            }
            if res.inner.reset_layout {
                gantz_response.responses.push(None, ResetTilesLayout);
            }
            if res.inner.export_style {
                gantz_response.responses.push(None, ExportStyle);
            }
            if res.inner.import_style {
                gantz_response.responses.push(None, ImportStyle);
            }
            let mut ext_responses = res.inner.responses;
            gantz_response
                .responses
                .extend(None, ext_responses.drain().map(|(_, d)| d));
        }
    }
}

/// The context passed to the inner graph `egui_tiles::Tree` widget.
struct GraphTreeBehaviour<'a, Access>
where
    Access: HeadAccess,
{
    env: &'a Env<'a>,
    codec: &'a NodeCodec,
    access: &'a mut Access,
    state: &'a mut GantzState,
    focused_head: &'a mut usize,
    /// Heads closed via the tab close button.
    closed_heads: &'a mut Vec<gantz_ca::Head>,
    /// A new branch created from a tab double-click, as the original head and
    /// the new branch name.
    new_branch: &'a mut Option<(gantz_ca::Head, String)>,
    /// Dynamic payloads emitted from within the graph scenes.
    responses: &'a mut Responses,
    /// Heads whose graph had a CA-affecting edit this frame.
    changed_heads: &'a mut Vec<gantz_ca::Head>,
    /// Per-head node index remappings from this frame's deletions, applied to
    /// the top-level tree's node views after layout. See
    /// `migrate_node_view_paths`.
    reindexes: &'a mut Vec<(gantz_ca::Head, crate::ops::Reindex)>,
    base_names: &'a crate::reg::Names,
    base_immutable: bool,
    /// Whether the per-node change-tracking validator is enabled. See
    /// [`GraphScene::validate_change_tracking`].
    validate_change_tracking: bool,
    /// Extension-pane toggle entries for the scene's "Panes" context submenu.
    /// See [`ext_pane_entries`].
    ext_panes: &'a [widget::ExtPaneEntry],
    /// Domain edge stylers for the graph scenes. See [`widget::EdgeStyle`].
    edge_styles: &'a [&'a dyn widget::EdgeStyle],
    /// Collaborative-session display state, when a collab layer is wired. It
    /// drives the per-tab session dot and the connecting and error overlay.
    collab: Option<&'a crate::collab::CollabUiState>,
    /// See [`Gantz::audio_heads`].
    audio_heads: Option<&'a HashSet<gantz_ca::Head>>,
}

impl<'a, Access> egui_tiles::Behavior<GraphPane> for GraphTreeBehaviour<'a, Access>
where
    Access: HeadAccess,
{
    fn tab_title_for_pane(&mut self, pane: &GraphPane) -> egui::WidgetText {
        let GraphPane(head) = pane;
        head.to_string().into()
    }

    fn tab_bar_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        visuals.panel_fill
    }

    fn resize_stroke(
        &self,
        style: &egui::Style,
        resize_state: egui_tiles::ResizeState,
    ) -> egui::Stroke {
        separator_stroke(self.state.style.separator, style, resize_state)
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        self.state.style.separator.width
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<GraphPane>,
        _tile_id: egui_tiles::TileId,
        _state: &egui_tiles::TabState,
    ) -> egui::Stroke {
        egui::Stroke::NONE
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }

    fn is_tab_closable(
        &self,
        _tiles: &egui_tiles::Tiles<GraphPane>,
        _tile_id: egui_tiles::TileId,
    ) -> bool {
        // Allow closing tabs if there is more than one head open.
        self.access.heads().len() > 1
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<GraphPane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        if let Some(GraphPane(head)) = tiles.get_pane(&tile_id).cloned() {
            self.closed_heads.push(head);
        }
        // Return true to allow egui_tiles to remove the tile.
        true
    }

    fn tab_ui(
        &mut self,
        tiles: &mut egui_tiles::Tiles<GraphPane>,
        ui: &mut egui::Ui,
        id: egui::Id,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        let edit_state_id = egui::Id::new("tab_edit_state");
        let mut edit_state: TabEditState = ui
            .memory_mut(|m| m.data.get_temp(edit_state_id))
            .unwrap_or_default();

        let is_editing = edit_state.editing_tile_id == Some(tile_id);

        let response = if is_editing {
            let head = tiles.get_pane(&tile_id).map(|GraphPane(h)| h.clone());
            let names = crate::reg::names(self.env.registry);

            let name_res = head.as_ref().map(|h| {
                ui.scope(|ui| {
                    ui.set_max_width(ui.available_width().min(150.0));
                    widget::head_name_edit(h, &mut edit_state.edit_text, &names, ui)
                })
                .inner
            });

            let Some(name_res) = name_res else {
                edit_state.editing_tile_id = None;
                edit_state.edit_text.clear();
                ui.memory_mut(|m| m.data.insert_temp(edit_state_id, edit_state));
                return ui.label("");
            };

            // Request focus on the first frame after entering edit mode.
            if edit_state.request_focus {
                name_res.response.request_focus();
                edit_state.request_focus = false;
            }

            // head_name_edit resets the text on commit or cancel, so detect
            // focus loss or escape to clear the tab editing state.
            let editing_ended =
                name_res.response.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            if editing_ended {
                if let Some(new_branch) = name_res.new_branch {
                    *self.new_branch = Some(new_branch);
                }
                edit_state.editing_tile_id = None;
                edit_state.edit_text.clear();
            }

            name_res.response
        } else {
            // Append a filled circle if this head is focused.
            let mut title = self.tab_title_for_tile(tiles, tile_id).text().to_string();
            let mut session = None;
            let mut audio = None;
            if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
                let heads = self.access.heads();
                if crate::head_is_focused(heads, *self.focused_head, head) {
                    title.push_str(" ⚫");
                }
                // The head's collab session, when shared.
                if let (Some(collab), gantz_ca::Head::Branch(name)) = (self.collab, head) {
                    session = collab.sessions.get(name);
                }
                if self.audio_heads.is_some_and(|heads| heads.contains(head)) {
                    audio = Some(self.state.open_heads.get(head).is_some_and(|s| s.muted));
                }
            }
            let mut tab = widget::Tab::new(title, id)
                .active(state.active)
                .closable(state.closable)
                .hint("double-click to rename");
            if let Some(display) = session {
                tab = tab.status_dot(display.conn.color(), display.hover_text());
            }
            if let Some(muted) = audio {
                tab = tab.audio(muted);
            }
            let res = tab.show(ui);

            if res.audio.as_ref().is_some_and(|r| r.clicked()) {
                if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
                    let head_state = self.state.open_heads.entry(head.clone()).or_default();
                    head_state.muted = !head_state.muted;
                }
            }

            // Handle double-click to enter edit mode.
            if res.tab.double_clicked() {
                if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
                    let initial_text = match head {
                        gantz_ca::Head::Branch(name) => name.to_string(),
                        gantz_ca::Head::Commit(_) => String::new(),
                    };
                    edit_state.editing_tile_id = Some(tile_id);
                    edit_state.edit_text = initial_text;
                    edit_state.request_focus = true;
                }
            }

            // Update focused_head when this tab is clicked.
            if res.tab.clicked() {
                if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
                    if let Some(ix) = self.access.heads().iter().position(|h| h == head) {
                        *self.focused_head = ix;
                    }
                }
            }

            // Handle close button click directly, like egui_tiles default does.
            if res.close.is_some_and(|r| r.clicked()) {
                if self.on_tab_close(tiles, tile_id) {
                    tiles.remove(tile_id);
                }
            }

            res.tab
        };

        ui.memory_mut(|m| m.data.insert_temp(edit_state_id, edit_state));

        response
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut GraphPane,
    ) -> egui_tiles::UiResponse {
        let GraphPane(pane_head) = pane;

        // Find the index of this head, for updating `focused_head`.
        let ix = self
            .access
            .heads()
            .iter()
            .position(|h| h == pane_head)
            .expect("pane head not found in heads");

        let immutable = head_immutable(pane_head, self.base_immutable, self.base_names);
        let diagnostics = self.access.diagnostics(pane_head).to_vec();

        // Global layout params combined with this head's flow.
        let layout_config = self.state.layout_config;
        // Global grid, snap and align options, applied to every head.
        let scene_config = self.state.scene_config;
        let head_state = self.state.open_heads.entry(pane_head.clone()).or_default();
        let layout_params = layout_config.to_params(head_state.layout_flow);
        // Disjoint borrow of a sibling field of `open_heads` for the graph
        // scene's "Panes" context submenu.
        let view_toggles = &mut self.state.view_toggles;
        // Disjoint borrow for the scene-level Select-all shortcut.
        let keymap = &self.state.keymap;

        // The rect positions the floating breadcrumb window.
        let rect = ui.available_rect_before_wrap();

        // Get mutable access to this head's data and render the graph scene.
        // The camera rides out for overlays that map graph-space positions to
        // the pane, such as peer pointers.
        let (graph_response, camera) = match self.access.with_head_mut(pane_head, |data| {
            let camera = data.view.camera;
            let res = graph_scene(
                self.env,
                self.codec,
                data.graph,
                data.instances,
                pane_head,
                head_state,
                view_toggles,
                self.ext_panes,
                self.edge_styles,
                data.view,
                layout_params,
                scene_config,
                immutable,
                self.validate_change_tracking,
                keymap,
                &diagnostics,
                data.vm,
                ui,
            );
            (res, camera)
        }) {
            Some((res, camera)) => (res, Some(camera)),
            None => (None, None),
        };

        if let Some(response) = graph_response {
            // Focus this head when clicking on the graph or any of its nodes.
            if response.scene.clicked() || response.any_node_interacted() {
                *self.focused_head = ix;
            }
            // Record a CA-affecting edit so the app can commit just this head.
            if response.changed {
                self.changed_heads.push(pane_head.clone());
            }
            // Collect this frame's deletions. `Gantz::show` migrates the
            // top-level tree's node-view paths past them after layout, since
            // the live top-level tree is not reachable here mid-traversal.
            if !response.reindex.is_empty() {
                self.reindexes.push((pane_head.clone(), response.reindex));
            }
            // Tag the scene's emissions with this head.
            self.responses.extend(Some(&*pane_head), response.responses);
        }

        // Floating name breadcrumb for nested graphs.
        let crumbs = name_breadcrumb(rect, pane_head, ui);
        self.responses.extend(Some(&*pane_head), crumbs);

        // A collab-session overlay. While a join is still connecting or has
        // failed, the pane's graph is only a placeholder, so dim the scene and
        // say what is happening. Peers' live pointers paint beneath it.
        if let (Some(collab), gantz_ca::Head::Branch(name)) = (self.collab, &*pane_head) {
            if let Some(display) = collab.sessions.get(name) {
                if self.state.collab.show_pointers && !display.pointers.is_empty() {
                    if let Some(camera) = camera {
                        paint_peer_pointers(rect, camera, &display.pointers, ui);
                        // Cursors move between this peer's frames.
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(100));
                    }
                }
                paint_session_overlay(rect, display, ui);
            }
        }

        egui_tiles::UiResponse::None
    }
}

/// Paint session peers' live pointers over the pane.
///
/// Positions arrive in graph-space coordinates. The head's camera maps them
/// to screen space, so cursors land on the right nodes regardless of either
/// peer's viewport. Painted on a foreground layer for the same reason as
/// [`paint_session_overlay`]. The scene's sublayer background would hide a
/// plain `ui.painter()` overlay.
fn paint_peer_pointers(
    rect: egui::Rect,
    camera: crate::Camera,
    pointers: &[crate::collab::PointerDisplay],
    ui: &egui::Ui,
) {
    let layer = egui::LayerId::new(egui::Order::Foreground, ui.id().with("peer_pointers"));
    let mut painter = ui.ctx().layer_painter(layer);
    painter.set_clip_rect(rect);
    for pointer in pointers {
        let screen = rect.center() + (pointer.pos - camera.center) * camera.zoom;
        // Skip cursors far outside the viewport. The clip rect would hide
        // them anyway, and this skips the label layout too.
        if !rect.expand(24.0).contains(screen) {
            continue;
        }
        painter.circle(
            screen,
            4.0,
            pointer.color,
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
        );
        painter.text(
            screen + egui::vec2(8.0, 6.0),
            egui::Align2::LEFT_TOP,
            &pointer.label,
            egui::FontId::proportional(11.0),
            pointer.color,
        );
    }
}

/// Dim a joining session's still-empty scene with its sync progress, or the
/// error when the join failed. Painted only while the join placeholder is
/// shown, that is while `awaiting_snapshot`. Once the snapshot arrives the
/// graph renders unobscured. A host never shows a placeholder.
fn paint_session_overlay(rect: egui::Rect, display: &crate::collab::SessionDisplay, ui: &egui::Ui) {
    use crate::collab::SessionConn;
    // Only ever cover the empty placeholder scene. Once the graph has loaded a
    // mid-session error surfaces through the tab's status dot, not a full-scene
    // overlay that would obscure a usable graph.
    if !display.awaiting_snapshot {
        return;
    }
    let (heading, detail, color) = if let Some(error) = &display.error {
        (
            "Failed to connect".to_string(),
            Some(error.clone()),
            SessionConn::Degraded.color(),
        )
    } else {
        // Animated ellipsis while waiting.
        let dots = 1 + (ui.input(|i| i.time) * 2.0) as usize % 3;
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
        let heading = format!("Connecting{}", ".".repeat(dots));
        (
            heading,
            Some(display.sync_status()),
            ui.visuals().strong_text_color(),
        )
    };
    // egui_graph draws the scene in a sublayer with an opaque background,
    // composited directly above this pane's own layer, so a `ui.painter()`
    // overlay would be hidden beneath it. Paint on a foreground layer instead.
    let layer = egui::LayerId::new(egui::Order::Foreground, ui.id().with("session_overlay"));
    let mut painter = ui.ctx().layer_painter(layer);
    painter.set_clip_rect(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(120));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        heading,
        egui::FontId::proportional(20.0),
        color,
    );
    if let Some(detail) = detail {
        painter.text(
            rect.center() + egui::vec2(0.0, 28.0),
            egui::Align2::CENTER_CENTER,
            detail,
            egui::FontId::proportional(14.0),
            ui.visuals().weak_text_color(),
        );
    }
}

/// The tab title for a node-view pane. `<head>:<path>` with the final path
/// segment rendered as `<index>-<ty_name>`, for example `main:3-plot`.
/// Intermediate segments are shown as raw indices.
fn node_view_title(pane: &NodeViewPane) -> String {
    use std::fmt::Write;
    let mut s = format!("{}", pane.head);
    let last = pane.path.len().saturating_sub(1);
    for (i, seg) in pane.path.iter().enumerate() {
        if i == last {
            let _ = write!(s, ":{seg}-{}", pane.ty_name);
        } else {
            let _ = write!(s, ":{seg}");
        }
    }
    s
}

/// The display title for a pane, shared by the tab bar, floating windows and
/// the `windowed_panes` report. Panes tied to the focused head suffix it,
/// for example `Steel - main`.
fn pane_title<Access>(
    gantz: &Gantz<'_>,
    access: &Access,
    focused_head: usize,
    pane: &Pane,
) -> String
where
    Access: HeadAccess,
{
    let with_head = |label: &str| match access.heads().get(focused_head) {
        Some(head) => format!("{label} - {head}"),
        None => label.to_string(),
    };
    match pane {
        Pane::Ext(key) => match gantz.ext_panes.iter().find(|p| p.key() == *key) {
            Some(p) => with_head(p.title()),
            None => key.clone(),
        },
        Pane::GraphConfig => with_head("Graph"),
        Pane::GraphScene => "Graphs".to_string(),
        Pane::Graphs => "Graphs".to_string(),
        Pane::GuiPerf => "GUI Perf".to_string(),
        Pane::History => "History".to_string(),
        Pane::Settings => "Settings".to_string(),
        Pane::Logs => match gantz.log_source {
            None => "Logs (No Source)".to_string(),
            Some(LogSource::Logger(_)) => "Logs".to_string(),
            #[cfg(feature = "tracing")]
            Some(LogSource::TraceCapture(..)) => "Tracing".to_string(),
        },
        Pane::NodeInspector => with_head("Node Inspector"),
        Pane::NodeView(p) => node_view_title(p),
        Pane::Steel => with_head("Steel"),
        Pane::GuiPreview => with_head("GUI Preview"),
        Pane::GuiTree => with_head("GUI Tree"),
        Pane::VmPerf => "VM Perf".to_string(),
    }
}

impl widget::node_palette::Command for NodeTyCmd<'_> {
    fn text(&self) -> &str {
        self.name
    }

    fn description(&self) -> Option<std::borrow::Cow<'static, str>> {
        self.env.node_description(self.name)
    }

    fn info_ui(&self, ui: &mut egui::Ui) {
        crate::node_info_ui(&self.env.command_info(self.name), ui);
    }

    fn formatted_kb_shortcut(&self, ctx: &egui::Context) -> Option<String> {
        self.env.command_formatted_kb_shortcut(ctx, self.name)
    }
}

impl Clone for NodeTyCmd<'_> {
    fn clone(&self) -> Self {
        Self {
            env: self.env,
            name: self.name,
        }
    }
}

impl Copy for NodeTyCmd<'_> {}

impl Default for GantzState {
    fn default() -> Self {
        Self::new()
    }
}

/// Create the initial layout of the tree of tiles.
///
/// Roughly something like this:
///
/// ```text
/// -----------------------------------------
/// |grs/hist/settings |scene               |
/// |------------------|                     |
/// |vm/gui            |                     |
/// |------------------|---------------------|
/// |conf              |logs      |steel     |
/// |------------------|          |          |
/// |insp              |          |          |
/// -----------------------------------------
/// ```
///
/// The active tab of each tab container defaults to its first child. See
/// `egui_tiles::Tabs::new`. Child ordering picks the default tabs.
fn create_tree() -> egui_tiles::Tree<Pane> {
    let mut tiles = egui_tiles::Tiles::default();

    // The leaf panes. The GUI Preview and GUI Tree panes are not created
    // here. They join the tray via `sync_singleton_pane`, the same path that
    // serves persisted trees predating them.
    let graph_config = tiles.insert_pane(Pane::GraphConfig);
    let graph_scene = tiles.insert_pane(Pane::GraphScene);
    let graphs = tiles.insert_pane(Pane::Graphs);
    let gui_perf = tiles.insert_pane(Pane::GuiPerf);
    let history = tiles.insert_pane(Pane::History);
    let logs = tiles.insert_pane(Pane::Logs);
    let node_inspector = tiles.insert_pane(Pane::NodeInspector);
    let settings = tiles.insert_pane(Pane::Settings);
    let steel = tiles.insert_pane(Pane::Steel);
    let vm_perf = tiles.insert_pane(Pane::VmPerf);

    // Sidebar tab containers. The first child is the default-active tab.
    let graphs_history_settings = tiles.insert_tab_tile(vec![graphs, history, settings]);
    // VM Perf and GUI Perf sit side by side rather than as tabs, so both plots
    // are visible at once.
    let perf = tiles.insert_horizontal_tile(vec![vm_perf, gui_perf]);

    // The sidebar column.
    let mut shares = egui_tiles::Shares::default();
    shares.set_share(graphs_history_settings, 0.30);
    shares.set_share(perf, 0.05);
    shares.set_share(graph_config, 0.13);
    shares.set_share(node_inspector, 0.25);
    let left_column = tiles.insert_container(egui_tiles::Linear {
        children: vec![graphs_history_settings, perf, graph_config, node_inspector],
        dir: egui_tiles::LinearDir::Vertical,
        shares,
    });

    // Logs and steel code in bottom "tray".
    let tray = tiles.insert_horizontal_tile(vec![logs, steel]);

    // The right column with the graph scene above the tray.
    let right_column = tiles.insert_container(egui_tiles::Linear::new_binary(
        egui_tiles::LinearDir::Vertical,
        [graph_scene, tray],
        0.7,
    ));

    // The root with both columns. The split here is only a fallback. The
    // sidebar normally has a fixed pixel width maintained across window
    // resizes. See `impose_fixed_sizes` and `default_sidebar_width`.
    let root = tiles.insert_container(egui_tiles::Linear::new_binary(
        egui_tiles::LinearDir::Horizontal,
        [left_column, right_column],
        0.18,
    ));

    egui_tiles::Tree::new("gantz-tiles-tree", root, tiles)
}

/// Create an empty graph tree. Panes will be added by `sync_graph_panes`.
fn create_empty_graph_tree() -> egui_tiles::Tree<GraphPane> {
    egui_tiles::Tree::empty("graph-tiles")
}

/// Insert a [`Pane::NodeView`] for `(head, path)` into the top-level `tree`, or
/// activate the existing one. Views are deduped by head and path. `ty_name`
/// is the node's type name used for the tab title. New views land in the
/// tray, a layout-safe default that is opaque to the fixed-size anchors. The
/// user can then drag them anywhere in the tree.
fn add_node_view_pane(
    tree: &mut egui_tiles::Tree<Pane>,
    head: gantz_ca::Head,
    path: Vec<node::Id>,
    ty_name: String,
) {
    // If a view for this head and path already exists, focus it.
    let existing = tree.tiles.iter().find_map(|(id, tile)| match tile {
        egui_tiles::Tile::Pane(Pane::NodeView(p)) if p.head == head && p.path == path => Some(*id),
        _ => None,
    });
    if let Some(id) = existing {
        tree.make_active(|tile_id, _| tile_id == id);
        return;
    }
    let pane = Pane::NodeView(NodeViewPane {
        head,
        path,
        ty_name,
    });
    insert_tray_pane(tree, pane);
}

/// Insert a new tile for `pane`, defaulting to the tray and falling back to
/// the root if the layout is not canonical.
fn insert_tray_pane(tree: &mut egui_tiles::Tree<Pane>, pane: Pane) {
    let pane_id = tree.tiles.insert_pane(pane);
    let container = layout_anchors(tree).map(|a| a.tray).or_else(|| tree.root());
    match container {
        Some(c) => tree.move_tile_to_container(pane_id, c, usize::MAX, true),
        None => tree.root = Some(pane_id),
    }
}

/// Ensure a [`Pane::Ext`] tile exists for each supplied provider key, adding
/// missing ones to the tray. They stay hidden until toggled, like Logs and
/// Steel. Tiles whose provider is absent are left in place. They render a
/// placeholder and keep their spot in the layout for when the provider
/// returns.
fn sync_ext_panes(tree: &mut egui_tiles::Tree<Pane>, keys: &[&str]) {
    for &key in keys {
        let exists = tree
            .tiles
            .iter()
            .any(|(_, tile)| matches!(tile, egui_tiles::Tile::Pane(Pane::Ext(k)) if k == key));
        if !exists {
            insert_tray_pane(tree, Pane::Ext(key.to_string()));
        }
    }
}

/// Ensure a tile exists for the given singleton pane, adding it to the tray
/// when missing. A tree persisted before the pane existed lacks it.
fn sync_singleton_pane(tree: &mut egui_tiles::Tree<Pane>, pane: Pane) {
    let exists = tree
        .tiles
        .iter()
        .any(|(_, tile)| matches!(tile, egui_tiles::Tile::Pane(p) if *p == pane));
    if !exists {
        insert_tray_pane(tree, pane);
    }
}

/// Sync the graph tree panes with the current heads.
///
/// Adds missing panes for new heads and removes panes for heads that no longer exist.
fn sync_graph_panes(tree: &mut egui_tiles::Tree<GraphPane>, heads: &[gantz_ca::Head]) {
    use std::collections::HashSet;

    let existing: HashSet<gantz_ca::Head> = tree
        .tiles
        .iter()
        .filter_map(|(_, tile)| match tile {
            egui_tiles::Tile::Pane(GraphPane(head)) => Some(head.clone()),
            _ => None,
        })
        .collect();

    let current: HashSet<gantz_ca::Head> = heads.iter().cloned().collect();

    for head in heads {
        if !existing.contains(head) {
            let pane_id = tree.tiles.insert_pane(GraphPane(head.clone()));
            if let Some(root_id) = tree.root() {
                tree.move_tile_to_container(pane_id, root_id, usize::MAX, true);
            } else {
                let root = tree.tiles.insert_tab_tile(vec![pane_id]);
                tree.root = Some(root);
            }
        }
    }

    let orphaned: Vec<egui_tiles::TileId> = tree
        .tiles
        .iter()
        .filter_map(|(id, tile)| match tile {
            egui_tiles::Tile::Pane(GraphPane(head)) if !current.contains(head) => Some(*id),
            _ => None,
        })
        .collect();
    for id in orphaned {
        tree.tiles.remove(id);
    }
}

/// All panes should have tab bars besides the main graph scene.
///
/// In the case that a tile is being dragged, even the graph scene should show a
/// tab bar in case the user wants to add a tab there.
fn simplify_tree(tree: &mut egui_tiles::Tree<Pane>, ctx: &egui::Context) {
    // Default options, but ensure panes have tabs.
    tree.simplify(&egui_tiles::SimplificationOptions {
        all_panes_must_have_tabs: true,
        ..Default::default()
    });
    // If a tile is being dragged, show all tab bars.
    if tree.dragged_id(ctx).is_some() {
        return;
    }
    let Some(graph_scene_id) = tree.tiles.find_pane(&Pane::GraphScene) else {
        return;
    };
    // Find its parent. This must be `Tabs` after the `simplify` pass above.
    let Some(parent_id) = tree.tiles.parent_of(graph_scene_id) else {
        return;
    };
    // If the parent has one child, replace it with the graph scene.
    let Some(parent) = tree.tiles.get_container(parent_id) else {
        return;
    };
    if parent.num_children() == 1 {
        tree.tiles.remove(graph_scene_id);
        tree.tiles
            .insert(parent_id, egui_tiles::Tile::Pane(Pane::GraphScene));
    }
}

/// The gap between sibling tiles, in points. It must match the default
/// `egui_tiles::Behavior::gap_width`, which is not overridden, so imposed
/// pixel sizes are exact and do not drift when re-imposed each frame.
const TILE_GAP: f32 = 1.0;

/// The minimum sidebar width and tray height, in points.
const MIN_PANE_SIZE: f32 = 80.0;

/// The tiles whose Linear share splits hold the sidebar width and tray height,
/// when the tree has its default top-level shape.
struct LayoutAnchors {
    /// The root horizontal Linear, `[left_column | right_column]`.
    root: egui_tiles::TileId,
    left_column: egui_tiles::TileId,
    /// The right column vertical Linear, `[graph_scene / tray]`.
    right_column: egui_tiles::TileId,
    graph_scene: egui_tiles::TileId,
    tray: egui_tiles::TileId,
}

/// Identify the layout anchors, or `None` if the tree is not in its default
/// top-level shape, for example mid-drag or after the user rearranged panes.
/// In that case the proportional layout is left untouched.
fn layout_anchors(tree: &egui_tiles::Tree<Pane>) -> Option<LayoutAnchors> {
    let graph_scene = tree.tiles.find_pane(&Pane::GraphScene)?;
    let right_column = tree.tiles.parent_of(graph_scene)?;
    let root = tree.root()?;
    let &[a, b] = linear_children(tree, root, egui_tiles::LinearDir::Horizontal)?.as_slice() else {
        return None;
    };
    let left_column = match (a == right_column, b == right_column) {
        (false, true) => a,
        (true, false) => b,
        _ => return None,
    };
    let &[c, d] = linear_children(tree, right_column, egui_tiles::LinearDir::Vertical)?.as_slice()
    else {
        return None;
    };
    let tray = match (c == graph_scene, d == graph_scene) {
        (true, false) => d,
        (false, true) => c,
        _ => return None,
    };
    Some(LayoutAnchors {
        root,
        left_column,
        right_column,
        graph_scene,
        tray,
    })
}

/// The children of `id` if it is a Linear container with direction `dir`.
fn linear_children(
    tree: &egui_tiles::Tree<Pane>,
    id: egui_tiles::TileId,
    dir: egui_tiles::LinearDir,
) -> Option<Vec<egui_tiles::TileId>> {
    match tree.tiles.get_container(id)? {
        egui_tiles::Container::Linear(l) if l.dir == dir => Some(l.children.clone()),
        _ => None,
    }
}

/// Set the two shares of a binary Linear container.
fn set_linear_shares(
    tree: &mut egui_tiles::Tree<Pane>,
    container: egui_tiles::TileId,
    a: egui_tiles::TileId,
    a_share: f32,
    b: egui_tiles::TileId,
    b_share: f32,
) {
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(l))) =
        tree.tiles.get_mut(container)
    {
        l.shares.set_share(a, a_share);
        l.shares.set_share(b, b_share);
    }
}

/// Impose the stored sidebar width and tray height in points on the tree's
/// share splits so they stay fixed as the window resizes. Call after
/// `simplify_tree` and before `tree.ui`.
fn impose_fixed_sizes(tree: &mut egui_tiles::Tree<Pane>, state: &GantzState, area: egui::Rect) {
    let Some(anchors) = layout_anchors(tree) else {
        return;
    };
    // Both columns span the full height, so the tray's available height is the
    // area height less the gap. The sidebar's available width likewise.
    if state.view_toggles.sidebar_open {
        let avail = area.width() - TILE_GAP;
        let width = state
            .sidebar_width
            .clamp(MIN_PANE_SIZE, (avail - MIN_PANE_SIZE).max(MIN_PANE_SIZE));
        set_linear_shares(
            tree,
            anchors.root,
            anchors.left_column,
            width,
            anchors.right_column,
            (avail - width).max(1.0),
        );
    }
    if state.view_toggles.logs
        || state.view_toggles.steel
        || state.view_toggles.gui_preview
        || state.view_toggles.gui_tree
    {
        let avail = area.height() - TILE_GAP;
        let height = state
            .tray_height
            .clamp(MIN_PANE_SIZE, (avail - MIN_PANE_SIZE).max(MIN_PANE_SIZE));
        set_linear_shares(
            tree,
            anchors.right_column,
            anchors.graph_scene,
            (avail - height).max(1.0),
            anchors.tray,
            height,
        );
    }
}

/// Capture the sidebar width and tray height in points from the laid-out
/// tree, so they can be re-imposed next frame, including after manual divider
/// drags. Call after `tree.ui`.
///
/// This reads the post-layout shares rather than the cached rects. A resize
/// drag updates the shares during `tree.ui`, but the rects it computes
/// reflect the pre-drag split, so reading rects would never see the drag.
fn capture_fixed_sizes(tree: &egui_tiles::Tree<Pane>, state: &mut GantzState, area: egui::Rect) {
    let Some(anchors) = layout_anchors(tree) else {
        return;
    };
    // Gate on the laid-out visibility, not `sidebar_open`. The hamburger can
    // flip `sidebar_open` mid-frame, but `set_tile_visibility` only runs at the
    // frame start, so the layout and the captured share reflect the
    // visibility from frame start. Capturing against a stale layout would
    // compute the column's size against the wrong set of visible siblings.
    if tree.is_visible(anchors.left_column) {
        if let Some(width) = linear_child_points(
            tree,
            anchors.root,
            anchors.left_column,
            area.width() - TILE_GAP,
        ) {
            if width > 1.0 {
                state.sidebar_width = width;
            }
        }
    }
    if tree.is_visible(anchors.tray) {
        if let Some(height) = linear_child_points(
            tree,
            anchors.right_column,
            anchors.tray,
            area.height() - TILE_GAP,
        ) {
            if height > 1.0 {
                state.tray_height = height;
            }
        }
    }
}

/// The points a Linear child currently occupies, derived from its share of the
/// visible children, mirroring `egui_tiles::Shares::split`.
fn linear_child_points(
    tree: &egui_tiles::Tree<Pane>,
    container: egui_tiles::TileId,
    child: egui_tiles::TileId,
    available: f32,
) -> Option<f32> {
    let egui_tiles::Container::Linear(l) = tree.tiles.get_container(container)? else {
        return None;
    };
    let total: f32 = l
        .children
        .iter()
        .filter(|&&c| tree.is_visible(c))
        .map(|&c| l.shares[c])
        .sum();
    (total > 0.0).then(|| available * l.shares[child] / total)
}

/// The supplied extension panes' checkbox entries. Both pane-toggle UIs
/// render from this single source, so a new pane cannot appear in one and
/// not the other.
fn ext_pane_entries(gantz: &Gantz) -> Vec<widget::ExtPaneEntry> {
    gantz
        .ext_panes
        .iter()
        .map(|p| widget::ExtPaneEntry {
            key: p.key().to_string(),
            title: p.title().to_string(),
            description: p.description().to_string(),
        })
        .collect()
}

/// Whether a tab's pane can be hidden via its right-click menu. The main graph
/// scene is not hideable. Node views are closed and removed, not hidden.
fn pane_is_hideable(pane: &Pane) -> bool {
    !matches!(pane, Pane::GraphScene | Pane::NodeView(_))
}

/// Set a pane's visibility toggle. No-op for panes without one.
fn set_pane_visible(view: &mut ViewToggles, pane: &Pane, visible: bool) {
    match pane {
        Pane::Ext(key) => {
            view.ext.insert(key.clone(), visible);
        }
        Pane::Graphs => view.graphs = visible,
        Pane::History => view.history = visible,
        Pane::Settings => view.settings = visible,
        Pane::GraphConfig => view.graph_config = visible,
        Pane::NodeInspector => view.node_inspector = visible,
        Pane::VmPerf => view.perf_vm = visible,
        Pane::GuiPerf => view.perf_gui = visible,
        Pane::Logs => view.logs = visible,
        Pane::Steel => view.steel = visible,
        Pane::GuiPreview => view.gui_preview = visible,
        Pane::GuiTree => view.gui_tree = visible,
        // No visibility toggle. The scene is always visible and node views
        // are closable.
        Pane::GraphScene | Pane::NodeView(_) => {}
    }
}

/// Whether a pane's visibility toggle is currently on. Panes without a toggle
/// are always considered visible.
fn pane_is_visible(view: &ViewToggles, pane: &Pane) -> bool {
    match pane {
        Pane::Ext(key) => view.ext.get(key).copied().unwrap_or(false),
        Pane::Graphs => view.graphs,
        Pane::History => view.history,
        Pane::Settings => view.settings,
        Pane::GraphConfig => view.graph_config,
        Pane::NodeInspector => view.node_inspector,
        Pane::VmPerf => view.perf_vm,
        Pane::GuiPerf => view.perf_gui,
        Pane::Logs => view.logs,
        Pane::Steel => view.steel,
        Pane::GuiPreview => view.gui_preview,
        Pane::GuiTree => view.gui_tree,
        Pane::GraphScene | Pane::NodeView(_) => true,
    }
}

/// Whether a pane may be popped out into a window. Every pane but the graph
/// scene qualifies, since it hosts the inner graph tile tree.
fn pane_is_poppable(pane: &Pane) -> bool {
    !matches!(pane, Pane::GraphScene)
}

/// A stable identity for a pane, used to key its window and to dedupe the
/// windowed set. Singletons key on their variant. A node view keys on its
/// `(head, path)`, the same identity `add_node_view_pane` dedupes on. Public
/// so a native host can key a pop-out window's persisted geometry in
/// [`GantzState::windowed_geometry`] by the same identity.
pub fn pane_key(pane: &Pane) -> String {
    match pane {
        Pane::Ext(key) => format!("ext:{key}"),
        Pane::GraphConfig => "graph-config".to_string(),
        Pane::GraphScene => "graph-scene".to_string(),
        Pane::Graphs => "graphs".to_string(),
        Pane::GuiPreview => "gui-preview".to_string(),
        Pane::GuiTree => "gui-tree".to_string(),
        Pane::GuiPerf => "gui-perf".to_string(),
        Pane::History => "history".to_string(),
        Pane::Logs => "logs".to_string(),
        Pane::NodeInspector => "node-inspector".to_string(),
        Pane::Settings => "settings".to_string(),
        Pane::Steel => "steel".to_string(),
        Pane::VmPerf => "vm-perf".to_string(),
        Pane::NodeView(p) => {
            use std::fmt::Write;
            let mut s = format!("node-view:{}", p.head);
            for seg in &p.path {
                let _ = write!(s, ":{seg}");
            }
            s
        }
    }
}

/// egui-memory id under which the set of windowed panes persists, stored as a
/// RON `String` alongside the tile tree. See [`load_ron`].
fn windowed_panes_id() -> egui::Id {
    egui::Id::new("gantz-windowed-panes-storage-v2")
}

/// egui-memory id for panes a host has requested be re-docked before the next
/// `Gantz::show`. See [`redock_windowed_pane`].
fn pending_redock_id() -> egui::Id {
    egui::Id::new("gantz-pending-redock")
}

/// egui-memory id for node views a host has requested be closed before the
/// next `Gantz::show`. See [`close_windowed_pane`].
fn pending_close_id() -> egui::Id {
    egui::Id::new("gantz-pending-close")
}

/// Load and clear a pending-pane list stored in egui memory.
fn drain_pending(ctx: &egui::Context, id: egui::Id) -> Vec<Pane> {
    let pending: Vec<Pane> = load_ron(ctx, id).unwrap_or_default();
    if !pending.is_empty() {
        store_ron(ctx, id, &Vec::<Pane>::new());
    }
    pending
}

/// Append a pane to a pending-intent list in egui memory.
fn enqueue_pending(ctx: &egui::Context, id: egui::Id, pane: &Pane) {
    let mut pending: Vec<Pane> = load_ron(ctx, id).unwrap_or_default();
    pending.push(pane.clone());
    store_ron(ctx, id, &pending);
}

/// Request that a popped-out pane return to the tile tree, from outside a
/// [`Gantz::show`] call, for example a native host's OS-window close button.
///
/// The request is queued in egui memory and applied on the next `show`, so it
/// works from any egui context. A no-op if the pane is not windowed.
pub fn redock_windowed_pane(ctx: &egui::Context, pane: &Pane) {
    enqueue_pending(ctx, pending_redock_id(), pane);
}

/// Request that a popped-out node view be closed and destroyed rather than
/// re-docked. Queued and applied like [`redock_windowed_pane`]. Singletons
/// have no destructive close, so they are re-docked instead.
pub fn close_windowed_pane(ctx: &egui::Context, pane: &Pane) {
    enqueue_pending(ctx, pending_close_id(), pane);
}

/// Add `pane` to the windowed set unless one with the same identity is already
/// there.
fn push_windowed(windowed: &mut Vec<Pane>, pane: Pane) {
    let key = pane_key(&pane);
    if windowed.iter().all(|p| pane_key(p) != key) {
        windowed.push(pane);
    }
}

/// Pop a pane out of the tile tree into a window.
///
/// Node views are real tiles, so the tile is removed. Singletons stay in the
/// tree but hidden, since their window shows them instead. Either way the
/// pane joins the windowed set.
fn detach_pane(
    view: &mut ViewToggles,
    windowed: &mut Vec<Pane>,
    tiles: &mut egui_tiles::Tiles<Pane>,
    tile_id: egui_tiles::TileId,
    pane: &Pane,
) {
    match pane {
        Pane::NodeView(_) => {
            tiles.remove(tile_id);
        }
        _ => set_pane_visible(view, pane, false),
    }
    push_windowed(windowed, pane.clone());
}

/// Return a windowed pane to the tile tree. A node view re-enters as a tray
/// tile. A singleton just becomes visible again, since its tile stayed in the
/// tree.
fn redock_pane(view: &mut ViewToggles, tree: &mut egui_tiles::Tree<Pane>, pane: Pane) {
    match pane {
        Pane::NodeView(p) => add_node_view_pane(tree, p.head, p.path, p.ty_name),
        pane => set_pane_visible(view, &pane, true),
    }
}

/// Migrate windowed [`Pane::NodeView`] entries for `head` after a node removal,
/// mirroring [`migrate_node_view_paths`] for the windowed set. A view of a
/// removed node is dropped. A view of a swapped node has its path rewritten.
fn migrate_windowed_node_views(
    windowed: &mut Vec<Pane>,
    head: &gantz_ca::Head,
    reindex: &crate::ops::Reindex,
) {
    if reindex.is_empty() {
        return;
    }
    windowed.retain_mut(|pane| {
        let Pane::NodeView(p) = pane else {
            return true;
        };
        if p.head != *head {
            return true;
        }
        let [ix] = p.path[..] else {
            return true;
        };
        match reindex.apply_to_index(ix) {
            Some(new_ix) => {
                p.path = vec![new_ix];
                true
            }
            None => false,
        }
    });
}

/// Ensure the view toggles match the pane visibility.
fn set_tile_visibility(tree: &mut egui_tiles::Tree<Pane>, view: &ViewToggles) {
    let ids: Vec<_> = tree.tiles.tile_ids().collect();
    let open = view.sidebar_open;
    // Sidebar panes are gated by both the sidebar being open and their
    // individual toggle. The tray panes are independent of the sidebar.
    for &id in &ids {
        if let Some(pane) = tree.tiles.get_pane(&id) {
            match pane {
                Pane::GraphScene => (),
                Pane::Ext(key) => tree.set_visible(id, view.ext.get(key).copied().unwrap_or(false)),
                Pane::Settings => tree.set_visible(id, open && view.settings),
                Pane::GraphConfig => tree.set_visible(id, open && view.graph_config),
                Pane::Graphs => tree.set_visible(id, open && view.graphs),
                Pane::GuiPerf => tree.set_visible(id, open && view.perf_gui),
                Pane::History => tree.set_visible(id, open && view.history),
                Pane::NodeInspector => tree.set_visible(id, open && view.node_inspector),
                Pane::VmPerf => tree.set_visible(id, open && view.perf_vm),
                Pane::Logs => tree.set_visible(id, view.logs),
                Pane::Steel => tree.set_visible(id, view.steel),
                Pane::GuiPreview => tree.set_visible(id, view.gui_preview),
                Pane::GuiTree => tree.set_visible(id, view.gui_tree),
                // Always visible. A node view is removed by closing, not hiding.
                Pane::NodeView(_) => tree.set_visible(id, true),
            }
        }
    }
    // A container is visible when any child is.
    for &id in &ids {
        if let Some(container) = tree.tiles.get_container(id) {
            let has_visible_child = container.children().any(|&id| tree.is_visible(id));
            tree.set_visible(id, has_visible_child);
        }
    }
}

/// The egui ID used to store the Graphs pane rect for file drop targeting.
const GRAPHS_PANE_RECT_ID: &str = "gantz-graphs-pane-rect";

/// Paint a hover overlay when `.gantz` files are being dragged over this pane.
///
/// The overlay is best-effort. It only appears when the pointer position is
/// available and within the pane. Some platforms do not track the pointer
/// during OS file drags.
fn paint_gantz_file_hover_overlay(ui: &mut egui::Ui) {
    let rect = ui.max_rect();
    let latest_pos = ui.ctx().input(|i| i.pointer.latest_pos());
    let pointer_over = latest_pos.map(|p| rect.contains(p)).unwrap_or(false);
    let has_hovered = ui.ctx().input(|i| {
        i.raw
            .hovered_files
            .iter()
            .any(|f| export::is_maybe_gantz(f.path.as_deref()))
    });

    if has_hovered && pointer_over {
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(100));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop to import",
            egui::FontId::proportional(24.0),
            egui::Color32::WHITE,
        );
    }
}

/// Detect `.gantz` file drops from egui's raw input.
///
/// Called from [`Gantz::show`] after the tile tree renders, so that detection
/// is independent of pointer position, which may be unavailable during OS
/// file drags on some platforms. The target pane is Graphs when the pointer
/// is over the stored Graphs pane rect, and [`FileDropTarget::GraphScene`]
/// otherwise.
fn collect_gantz_file_drops(ctx: &egui::Context) -> Vec<FileDrop> {
    let dropped = ctx.input(|i| i.raw.dropped_files.clone());
    if dropped.is_empty() {
        return Vec::new();
    }

    let graphs_rect: Option<egui::Rect> =
        ctx.memory(|m| m.data.get_temp(egui::Id::new(GRAPHS_PANE_RECT_ID)));
    let latest_pos = ctx.input(|i| i.pointer.latest_pos());
    let over_graphs = match (graphs_rect, latest_pos) {
        (Some(rect), Some(pos)) => rect.contains(pos),
        _ => false,
    };
    let target = if over_graphs {
        FileDropTarget::Graphs
    } else {
        FileDropTarget::GraphScene
    };

    dropped
        .iter()
        .filter(|f| export::is_maybe_gantz(f.path.as_deref()))
        .filter_map(|f| export::read_dropped_file(f))
        .map(|bytes| FileDrop { bytes, target })
        .collect()
}

/// The output count of every node in `g`. It backs the GUI Preview tree's
/// push-eval resolver, since a push entry fn's identity covers the count.
///
/// Nodes reify transiently through the codec. A weight with an unknown tag
/// fails to reify and reports no count.
fn node_output_counts(
    registry: &Env<'_>,
    codec: &crate::node::NodeCodec,
    g: &gantz_ca::DataGraph,
) -> HashMap<node::Id, usize> {
    use gantz_core::Node;
    use petgraph::visit::{IntoNodeReferences, NodeRef};
    let get_node = |ca: &gantz_ca::ContentAddr| registry.node(ca);
    let ctx = gantz_core::node::MetaCtx::new(&get_node);
    g.node_references()
        .filter_map(|n| {
            let inst = codec.reify_ui(n.weight()).ok()?;
            Some((n.id().index(), inst.node.n_outputs(ctx)))
        })
        .collect()
}

/// The column the GUI Tree pane wraps the printed tree at.
const GUI_TREE_WIDTH: usize = 80;

/// The egui memory slot holding the picked `gui` role for `head`. The GUI
/// Preview and GUI Tree panes and their tab badges share it, so one pick
/// drives both panes.
fn gui_role_id(head: &gantz_ca::Head) -> egui::Id {
    egui::Id::new(("gantz-gui-role", head))
}

/// The role the GUI panes show for `head`, given its markers in index
/// order. The picked role when a marker of it exists, else the body, else
/// the first marker's role. `None` when the graph has no marker.
fn picked_gui_role(
    ctx: &egui::Context,
    head: &gantz_ca::Head,
    markers: &[(node::Id, crate::node::Gui)],
) -> Option<crate::node::GuiRole> {
    use crate::node::GuiRole;
    let has = |r: GuiRole| markers.iter().any(|(_, g)| g.role == r);
    ctx.data(|d| d.get_temp::<GuiRole>(gui_role_id(head)))
        .filter(|&r| has(r))
        .or_else(|| has(GuiRole::Body).then_some(GuiRole::Body))
        .or_else(|| markers.first().map(|&(_, g)| g.role))
}

/// Pick the `gui` role the GUI panes show for `head`.
fn set_gui_role(ctx: &egui::Context, head: &gantz_ca::Head, role: crate::node::GuiRole) {
    ctx.data_mut(|d| d.insert_temp(gui_role_id(head), role));
}

/// The role badge for a GUI pane's tab, as the focused head, its marker
/// roles in [`GuiRole::ALL`][crate::node::GuiRole::ALL] order and the picked
/// role. `None` unless the head declares at least two roles, since a single
/// role needs no picker.
fn gui_role_badge<Access: HeadAccess>(
    access: &mut Access,
    focused_head: usize,
    ctx: &egui::Context,
) -> Option<(
    gantz_ca::Head,
    Vec<crate::node::GuiRole>,
    crate::node::GuiRole,
)> {
    let head = access.heads().get(focused_head).cloned()?;
    let markers = access.with_head_mut(&head, |data| crate::node::gui::markers(data.graph))?;
    let roles: Vec<_> = crate::node::GuiRole::ALL
        .into_iter()
        .filter(|&r| markers.iter().any(|(_, g)| g.role == r))
        .collect();
    if roles.len() < 2 {
        return None;
    }
    let role = picked_gui_role(ctx, &head, &markers)?;
    Some((head, roles, role))
}

/// The picked role and the stored tree of the head's `gui` marker for it.
/// See [`picked_gui_role`].
///
/// `None` when the graph has no marker or the marker has no stored tree
/// yet. Both cases show a hint in place of the content.
fn gui_marker_tree(
    head: &gantz_ca::Head,
    graph: &gantz_ca::DataGraph,
    vm: &Engine,
    ui: &mut egui::Ui,
) -> Option<(crate::node::GuiRole, steel::SteelVal)> {
    let markers = crate::node::gui::markers(graph);
    let Some(role) = picked_gui_role(ui.ctx(), head, &markers) else {
        ui.weak("add a `gui` node to this graph to define its GUI");
        return None;
    };
    // First marker of the role, in index order.
    let &(ix, _) = markers
        .iter()
        .find(|(_, g)| g.role == role)
        .expect("role picked from existing markers");
    let val = node::state::extract_value(vm, &[ix])
        .ok()
        .flatten()
        .filter(|v| !matches!(v, steel::SteelVal::Void));
    if val.is_none() {
        ui.weak("the marker has no stored tree yet");
    }
    Some((role, val?))
}

/// Render a decoded body tree inside the node chrome a reference to `head`
/// has in a scene: the node frame with the graph's inlet and outlet sockets
/// and their docs. The node is static and centred in the pane. The returned
/// payloads are the tree's, see [`gui_preview_tree`].
fn gui_preview_node(
    env: &Env<'_>,
    codec: &crate::node::NodeCodec,
    head: &gantz_ca::Head,
    graph: &gantz_ca::DataGraph,
    vm: &mut Engine,
    decoded: &gantz_ui::Decoded,
    ui: &mut egui::Ui,
) -> Vec<crate::response::DynResponse> {
    let (inlets, outlets) = crate::inlet_outlet_ids(env, graph);
    let head_ca: Option<gantz_ca::ContentAddr> = env
        .registry
        .head_commit(head)
        .map(|commit| commit.graph.into());

    // Centre on the size measured last frame. The first frame draws at the
    // top left.
    let size_id = egui::Id::new(("gantz-gui-preview-node-size", head));
    let last_size: egui::Vec2 = ui
        .ctx()
        .data(|d| d.get_temp(size_id))
        .unwrap_or(egui::Vec2::ZERO);
    let avail = ui.available_rect_before_wrap();
    let offset = ((avail.size() - last_size) * 0.5).max(egui::Vec2::ZERO);
    let rect = egui::Rect::from_min_size(avail.min + offset, avail.size() - offset);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));

    let resp = egui_graph::node::Node::from_id(egui_graph::NodeId(0))
        .inputs(inlets.len())
        .outputs(outlets.len())
        .flow(egui::Direction::TopDown)
        .max_width(f32::INFINITY)
        .show_static(&mut child, |uictx| {
            uictx.framed(|ui, _sockets| gui_preview_tree(env, codec, head, graph, vm, decoded, ui))
        });
    ui.ctx()
        .data_mut(|d| d.insert_temp(size_id, resp.inner.response.rect.size()));

    // The sockets carry the graph's marker docs, as they do on a reference
    // node in a scene.
    if let Some(ca) = head_ca {
        for (ix, sock) in resp.sockets.inputs() {
            if let Some(doc) = env.socket_doc(&ca, crate::SocketKind::Input, ix) {
                super::graph_scene::socket_hover(sock, &doc);
            }
        }
        for (ix, sock) in resp.sockets.outputs() {
            if let Some(doc) = env.socket_doc(&ca, crate::SocketKind::Output, ix) {
                super::graph_scene::socket_hover(sock, &doc);
            }
        }
    }
    resp.inner.inner
}

/// Render a decoded tree against a head's live VM. This is the GUI Preview
/// pane's body. Bindings resolve into the head's node state both ways. The
/// returned payloads carry the tree's push evaluations and state writes.
fn gui_preview_tree(
    env: &Env<'_>,
    codec: &crate::node::NodeCodec,
    head: &gantz_ca::Head,
    graph: &gantz_ca::DataGraph,
    vm: &mut Engine,
    decoded: &gantz_ui::Decoded,
    ui: &mut egui::Ui,
) -> Vec<crate::response::DynResponse> {
    let (inlets, outlets) = crate::inlet_outlet_ids(env, graph);
    // The focused head's committed graph address. Resolver hops into
    // instances go through the registry, since instances always resolve
    // committed children.
    let head_ca: Option<gantz_ca::ContentAddr> = env
        .registry
        .head_commit(head)
        .map(|commit| commit.graph.into());
    let n_outs = node_output_counts(env, codec, graph);
    let resolver = |p: &[node::Id]| -> Option<usize> {
        match p {
            // Top-level nodes read the working graph.
            [ix] => n_outs.get(ix).copied(),
            _ => crate::reg::n_outputs_at(env, head_ca.as_ref()?, p),
        }
    };
    let ref_gui = |chain: &[node::Id]| -> Option<node::Id> {
        let (_, marker) = crate::reg::resolve_ref_chain(env, head_ca?, chain)?;
        Some(marker)
    };
    let mut writes = Vec::new();
    let mut node_ctx = NodeCtx::new(env, graph, &[], &inlets, &outlets, &[], vm, &mut writes);
    let root_id = egui::Id::new(("gantz-gui-preview", head));
    let r = crate::ui_tree::UiTree::new(root_id)
        .n_outputs(&resolver)
        .ref_gui(&ref_gui)
        .show(&decoded.root, &mut node_ctx, ui);
    let mut payloads = r.payloads;
    payloads.extend(
        writes
            .drain(..)
            .map(|w| crate::DynResponse::new(crate::StateWritten(w))),
    );
    payloads
}

/// The GUI Tree pane's collapsible decode-warnings list.
fn gui_tree_warnings(decoded: &gantz_ui::Decoded, ui: &mut egui::Ui) {
    if decoded.warnings.is_empty() {
        return;
    }
    let title = format!("warnings ({})", decoded.warnings.len());
    egui::CollapsingHeader::new(title).show(ui, |ui| {
        for w in &decoded.warnings {
            ui.horizontal_wrapped(|ui| {
                ui.weak(format!("{:?}", w.path.0));
                ui.label(w.kind.to_string());
            });
        }
    });
}

/// Provides a consistent frame and styling for the panes.
fn pane_ui<R>(ui: &mut egui::Ui, pane: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::CentralPanel::default().show_inside(ui, |ui| pane(ui))
}

/// The size of the floating sidebar toggle glyph. It also offsets the
/// nested-graph breadcrumb to its right, since they share the scene's
/// bottom-left corner.
const SIDEBAR_TOGGLE_ICON_SIZE: f32 = 18.0;

/// A floating hamburger button that toggles the sidebar.
///
/// Anchored to the given bottom-left position over the graph scene, so it
/// tracks the scene's corner rather than the whole window.
fn sidebar_toggle(ctx: &egui::Context, anchor_pos: egui::Pos2, open: &mut bool) {
    let id = egui::Id::new("gantz-sidebar-toggle");
    egui::Area::new(id)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(anchor_pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE.show(ui, |ui| {
                // Idle, the hamburger matches the faint colour of egui_graph's
                // dot grid. On hover it brightens a little to signal it is
                // interactive. There is no selection colour when open. Laid
                // out manually so the colour can depend on hover.
                let font = egui::FontId::proportional(SIDEBAR_TOGGLE_ICON_SIZE);
                let galley =
                    ui.painter()
                        .layout_no_wrap("☰".to_owned(), font, egui::Color32::PLACEHOLDER);
                let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
                let color = if response.hovered() {
                    ui.visuals().weak_text_color()
                } else {
                    ui.style().noninteractive().bg_stroke.color
                };
                ui.painter().galley(rect.min, galley, color);
                if response.clicked() {
                    *open = !*open;
                }
                let hint = if *open {
                    "close sidebar"
                } else {
                    "open sidebar"
                };
                response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(hint);
            });
        });
}

fn graph_select(
    env: &Env<'_>,
    heads: &[gantz_ca::Head],
    focused_head: usize,
    base_names: &crate::reg::Names,
    collab: Option<&crate::collab::CollabUiState>,
    clipboard: Option<&dyn Fn() -> Option<String>>,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<widget::graph_select::GraphSelectResponse> {
    pane_ui(ui, |ui| {
        widget::GraphSelect::new(env, heads, base_names)
            .focused_head(focused_head)
            .collab(collab)
            .clipboard(clipboard)
            .show(ui)
    })
}

fn history_view(
    env: &Env<'_>,
    heads: &[gantz_ca::Head],
    focused_head: usize,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<widget::graph_select::GraphSelectResponse> {
    pane_ui(ui, |ui| {
        widget::HistoryView::new(env, heads)
            .focused_head(focused_head)
            .show(ui)
    })
}

fn perf_view(title: &str, capture: &mut widget::PerfCapture, ui: &mut egui::Ui) {
    // Use Frame::NONE to fill the entire pane with no padding.
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show_inside(ui, |ui| {
            widget::PerfView::new(title, capture).show(ui);
        });
}

/// Returns the response from the graph scene if it was shown.
///
/// Payloads emitted within the scene are returned in
/// [`GraphSceneResponse::responses`][graph_scene::GraphSceneResponse] for the
/// caller to tag and merge.
#[allow(clippy::too_many_arguments)]
fn graph_scene(
    registry: &Env<'_>,
    codec: &NodeCodec,
    graph: &mut gantz_ca::DataGraph,
    instances: &mut crate::node::NodeInstances,
    head: &gantz_ca::Head,
    head_state: &mut OpenHeadState,
    view_toggles: &mut ViewToggles,
    ext_panes: &[widget::ExtPaneEntry],
    edge_styles: &[&dyn widget::EdgeStyle],
    head_view: &mut crate::SceneView,
    layout_params: egui_graph::LayoutParams,
    scene_config: SceneConfig,
    immutable: bool,
    validate_change_tracking: bool,
    keymap: &Keymap,
    diagnostics: &[gantz_core::Diagnostic],
    vm: &mut Engine,
    ui: &mut egui::Ui,
) -> Option<graph_scene::GraphSceneResponse> {
    // A head shows exactly its root graph. Nested graphs are separate heads.
    let id = egui::Id::new(head);

    // Select-all replaces the selection with every node. Handled here rather
    // than the outer command block because this is where the graph is in
    // scope. Gated like other command shortcuts so it does not fire while
    // typing.
    if !ui.ctx().egui_wants_keyboard_input() && keymap.consume(ui, Action::SelectAll) {
        head_state.scene.interaction.selection.nodes = graph.node_indices().collect();
        head_state.scene.interaction.selection.edges.clear();
    }

    // Seed the node layout the first time this graph is shown, and centre the
    // camera on it at zoom 1. Without an explicit camera the scene would fall
    // back to egui's fit-to-bounds, zooming out to frame every node. A freshly
    // opened graph should instead sit at its natural 1:1 zoom.
    if head_view.layout.is_empty() {
        head_view.layout =
            widget::graph_scene::layout(registry, codec, graph, id, &layout_params, ui.ctx(), None);
        let mut bounds: Option<egui::Rect> = None;
        for &pos in head_view.layout.values() {
            let r = egui::Rect::from_min_size(pos, egui::Vec2::ZERO);
            bounds = Some(bounds.map_or(r, |b| b.union(r)));
        }
        head_view.camera = crate::Camera {
            center: bounds.map_or(egui::Pos2::ZERO, |b| b.center()),
            zoom: 1.0,
        };
    }

    let response = GraphScene::new(registry, codec, graph, instances)
        .with_id(id)
        .layout_params(layout_params)
        .scene_config(scene_config)
        .immutable(immutable)
        .validate_change_tracking(validate_change_tracking)
        .view_toggles(view_toggles)
        .ext_panes(ext_panes)
        .edge_styles(head, edge_styles)
        .show(head_view, &mut head_state.scene, vm, ui);

    graph_scene::paint_diagnostics(diagnostics, &[], &response, ui);

    Some(response)
}

/// Floating name breadcrumb over the bottom-left corner of the scene, shown
/// when viewing a nested `parent:child` graph. Each crumb is a `:`-separated
/// name segment. The prefix it represents is the head it navigates to.
///
/// Returns the [`ReplaceHead`] payloads emitted by clicked crumbs, which
/// navigate the focused tab to an ancestor level in place.
fn name_breadcrumb(
    scene_rect: egui::Rect,
    head: &gantz_ca::Head,
    ui: &mut egui::Ui,
) -> Vec<DynResponse> {
    let mut responses = Vec::new();
    let gantz_ca::Head::Branch(name) = head else {
        return responses;
    };
    if !name.is_nested() {
        return responses; // a root graph has no ancestor levels
    }
    let segs: Vec<&str> = name.segments().iter().map(|s| s.as_str()).collect();
    let sep_str = gantz_ca::name::SEP.to_string();
    let space = ui.style().interaction.interact_radius * 3.0;
    // Sit to the right of the floating sidebar toggle, which occupies the very
    // bottom-left corner of the scene, so the levels stay on the bottom row.
    let toggle_w = SIDEBAR_TOGGLE_ICON_SIZE + ui.style().spacing.item_spacing.x;
    egui::Window::new("breadcrumb_window")
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(scene_rect.left_bottom() + egui::vec2(space + toggle_w, -space))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .frame(egui::Frame::NONE)
        .show(ui.ctx(), |ui| {
            fn button(s: &str) -> widget::LabelButton {
                let text = egui::RichText::new(s).size(24.0);
                widget::LabelButton::new(text)
            }
            let col_w = ui.style().interaction.interact_radius * 4.0;
            egui::Grid::new("breadcrumb")
                .min_col_width(col_w)
                .max_col_width(col_w)
                .show(ui, |ui| {
                    for (i, seg) in segs.iter().enumerate() {
                        let is_current = i + 1 == segs.len();
                        let prefix = segs[..=i].join(&sep_str);
                        // The crumbs are tiny. The root is `R` since its name
                        // is too big to fit, and each nested level is its
                        // short leaf.
                        let (label, hover) = if i == 0 {
                            ("R".to_string(), format!("navigate to {seg} root"))
                        } else {
                            (seg.to_string(), format!("navigate to {prefix}"))
                        };
                        ui.vertical_centered_justified(|ui| {
                            let resp = ui.add(button(&label)).on_hover_text(hover);
                            if resp.clicked() && !is_current {
                                responses.push(DynResponse::new(ReplaceHead(
                                    gantz_ca::Head::Branch(prefix.parse().expect("infallible")),
                                )));
                            }
                        });
                    }
                })
        });
    responses
}

/// A node-creation choice made in the node palette.
enum PaletteChoice {
    /// Create an ordinary node of the given type.
    Node(CreateNode),
    /// Create a new nested graph via the reserved [`NESTED_GRAPH_TYPE`] entry.
    NestedGraph(CreateNestedGraph),
}

/// Returns a node-creation payload when a node type is chosen.
///
/// `editing` is the focused head's name when it is a branch. It hides node
/// types whose reference would cycle back to the graph being edited.
fn node_palette(
    env: &Env<'_>,
    editing: Option<&str>,
    node_palette: &mut widget::NodePalette,
    keymap: &Keymap,
    ui: &mut egui::Ui,
) -> Option<PaletteChoice> {
    // Toggle node palette visibility via its keymap binding.
    if !ui.ctx().egui_wants_keyboard_input() && keymap.consume(ui, Action::ToggleNodePalette) {
        node_palette.toggle();
    }

    // Map the node types to commands for the node palette, dropping any type
    // whose reference would form a cycle back to the editing graph. The reserved
    // nested-graph entry always mints a fresh child, so it is never cyclic.
    let types: Vec<&str> = env
        .node_types()
        .into_iter()
        .filter(|&k| k == NESTED_GRAPH_TYPE || editing.is_none_or(|e| !env.would_ref_cycle(k, e)))
        .collect();
    let cmds = types.iter().map(|&k| NodeTyCmd { env, name: k });

    // The chosen node type becomes a creation payload. The reserved
    // `NESTED_GRAPH_TYPE` routes to the registry-aware nested-graph op. The
    // palette is centered over the graph scene, which is this `ui`'s rect.
    let scene_rect = ui.max_rect();
    node_palette.show(ui.ctx(), scene_rect, cmds).map(|cmd| {
        // The placement position is filled in by the caller, which has access to
        // the focused head's last pointer position.
        if cmd.name == NESTED_GRAPH_TYPE {
            PaletteChoice::NestedGraph(CreateNestedGraph { pos: None })
        } else {
            PaletteChoice::Node(CreateNode {
                node_type: cmd.name.to_string(),
                pos: None,
            })
        }
    })
}

fn log_view(
    logger: &widget::log_view::Logger,
    node_labels: &HashMap<Vec<node::Id>, String>,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<widget::log_view::LogViewResponse> {
    pane_ui(ui, |ui| {
        widget::log_view::LogView::new("log-view".into(), logger.clone())
            .node_labels(node_labels)
            .show(ui)
    })
}

fn trace_view(
    trace_capture: &widget::trace_view::TraceCapture,
    level: tracing::level_filters::LevelFilter,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<()> {
    pane_ui(ui, |ui| {
        widget::trace_view::TraceView::new("trace-view".into(), trace_capture.clone(), level)
            .show(ui);
    })
}

/// Whether the given head should be treated as immutable.
///
/// A head is immutable when `base_immutable` is enabled and the head is a base
/// graph that is not a demo. Demo base graphs are always mutable so users can
/// experiment.
fn head_immutable(
    head: &gantz_ca::Head,
    base_immutable: bool,
    base_names: &crate::reg::Names,
) -> bool {
    let is_base = matches!(head, gantz_ca::Head::Branch(name) if base_names.contains_key(name));
    let is_demo =
        matches!(head, gantz_ca::Head::Branch(name) if widget::graph_select::is_demo(name));
    base_immutable && is_base && !is_demo
}

/// Returns whether any inspected node had a CA-affecting edit, together with
/// the payloads emitted by node UIs within the inspector.
#[allow(clippy::too_many_arguments)]
fn node_inspector<'a>(
    registry: &'a Env<'a>,
    codec: &NodeCodec,
    root: &mut gantz_ca::DataGraph,
    instances: &mut crate::node::NodeInstances,
    vm: &mut Engine,
    head_state: &mut OpenHeadState,
    head: &gantz_ca::Head,
    immutable: bool,
    ref_ext_uis: &'a [&'a dyn crate::node::RefExtUi],
    ui: &mut egui::Ui,
) -> egui::InnerResponse<(bool, Vec<DynResponse>)> {
    pane_ui(ui, |ui| {
        let mut responses = Vec::new();
        let mut changed = false;
        egui::ScrollArea::vertical()
            .auto_shrink(egui::Vec2b::FALSE)
            .show(ui, |ui| {
                let graph = &mut *root;
                let ids: Vec<_> = graph.node_identifiers().collect();
                let (inlets, outlets) = crate::inlet_outlet_ids(registry, graph);
                // The rect of the first selected node, used to scroll to it.
                let mut selected_rect: Option<egui::Rect> = None;
                // VM-state writes recorded by each node's `NodeCtx`. They
                // drain per node into `StateWritten` payloads.
                let mut writes = Vec::new();
                for id in ids {
                    let mut frame = egui::Frame::group(ui.style());
                    let is_selected = head_state.scene.interaction.selection.nodes.contains(&id);
                    if is_selected {
                        frame.stroke.color = ui.visuals().selection.stroke.color;
                    }
                    let frame_resp = frame.show(ui, |ui| {
                        let Some(weight) = graph.node_weight(id) else {
                            return;
                        };
                        let ix = id.index();
                        // Take the node's cached instance. See
                        // `graph_scene::nodes`. Erase back below only when
                        // changed, updating the witness. An unknown tag shows
                        // a weak placeholder row.
                        let Ok(mut entry) = instances.take(codec, ix, weight) else {
                            ui.weak(format!("{} (unknown node type)", weight.tag));
                            return;
                        };
                        let path = [ix];
                        let ctx = NodeCtx::new(
                            registry,
                            graph,
                            &path[..],
                            &inlets,
                            &outlets,
                            ref_ext_uis,
                            vm,
                            &mut writes,
                        );
                        let resp = widget::NodeInspector::new(&mut entry.inst.node, ctx, immutable)
                            .show(ui);
                        if resp.changed {
                            changed = true;
                            match entry.inst.erase() {
                                Ok(node_data) => {
                                    entry.src = node_data.clone();
                                    graph[id] = node_data;
                                    instances.put(ix, entry);
                                }
                                Err(e) => log::error!(
                                    "inspector: failed to erase edited node {ix}, \
                                     edit dropped: {e}"
                                ),
                            }
                        } else {
                            instances.put(ix, entry);
                        }
                        responses.extend(resp.payloads);
                        if resp.label_response.clicked() {
                            let sel = &mut head_state.scene.interaction.selection.nodes;
                            if ui.input(|i| i.modifiers.command) {
                                if !sel.remove(&id) {
                                    sel.insert(id);
                                }
                            } else {
                                sel.clear();
                                sel.insert(id);
                            }
                        }
                    });
                    responses.extend(crate::action::state_written(&mut writes));
                    if is_selected && selected_rect.is_none() {
                        selected_rect = Some(frame_resp.response.rect);
                    }
                }

                // Scroll to the first selected node when the selection changes,
                // mirroring the Steel view's scroll-to-span on selection.
                let state_id = egui::Id::new("node_inspector_selection");
                let mut selected: Vec<node::Id> = head_state
                    .scene
                    .interaction
                    .selection
                    .nodes
                    .iter()
                    .map(|n| n.index())
                    .collect();
                selected.sort_unstable();
                let current = egui::Id::new(("inspector_sel", head, &selected));
                let prev: Option<egui::Id> = ui.ctx().data(|d| d.get_temp(state_id));
                if prev != Some(current) {
                    ui.ctx().data_mut(|d| d.insert_temp(state_id, current));
                    if let Some(rect) = selected_rect {
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    }
                }
            });
        (changed, responses)
    })
}

fn steel_view(
    compiled_steel: &str,
    compile_error: Option<&str>,
    highlights: &[std::ops::Range<usize>],
    errors: &[std::ops::Range<usize>],
    scroll_to: Option<usize>,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<()> {
    pane_ui(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink(egui::Vec2b::FALSE)
            .show(ui, |ui| {
                if let Some(error) = compile_error {
                    let color = ui.visuals().error_fg_color;
                    let text = egui::RichText::new(error).monospace().color(color);
                    ui.add(egui::Label::new(text).selectable(true));
                    if !compiled_steel.is_empty() {
                        ui.separator();
                    }
                }
                widget::SteelView::new(compiled_steel)
                    .highlights(highlights)
                    .errors(errors)
                    .scroll_to(scroll_to)
                    .show(ui);
            });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixed sidebar width must be imposed on the sidebar, not the main
    /// area. The anchors must identify the sidebar as the column that does
    /// not contain the graph scene.
    #[test]
    fn impose_sets_sidebar_width_on_left_column() {
        let mut tree = create_tree();
        let anchors = layout_anchors(&tree).expect("default tree has layout anchors");

        let graph_scene = tree.tiles.find_pane(&Pane::GraphScene).unwrap();
        assert_ne!(anchors.left_column, anchors.right_column);
        assert_eq!(
            tree.tiles.parent_of(graph_scene),
            Some(anchors.right_column)
        );

        let mut state = GantzState::new();
        state.view_toggles.sidebar_open = true;
        state.sidebar_width = 240.0;
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        impose_fixed_sizes(&mut tree, &state, area);

        let Some(egui_tiles::Container::Linear(root)) = tree.tiles.get_container(anchors.root)
        else {
            panic!("root is not a linear container");
        };
        let avail = 1000.0 - TILE_GAP;
        // The sidebar gets the fixed width. The main area gets the remainder.
        assert!((root.shares[anchors.left_column] - 240.0).abs() < 0.01);
        assert!((root.shares[anchors.right_column] - (avail - 240.0)).abs() < 0.01);
    }

    /// `capture_fixed_sizes` must recover the same width `impose_fixed_sizes`
    /// set, so a sidebar that is not dragged does not drift frame to frame.
    #[test]
    fn capture_round_trips_imposed_sidebar_width() {
        let mut tree = create_tree();
        let mut state = GantzState::new();
        state.view_toggles.sidebar_open = true;
        state.sidebar_width = 240.0;
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        impose_fixed_sizes(&mut tree, &state, area);
        capture_fixed_sizes(&tree, &mut state, area);
        assert!((state.sidebar_width - 240.0).abs() < 0.01);
    }

    /// Reopening the sidebar must not inflate its width. On the open-transition
    /// frame `sidebar_open` is already true but the layout still has the left
    /// column hidden. Capturing then would size it against the wrong siblings.
    #[test]
    fn capture_skips_while_sidebar_laid_out_hidden() {
        let mut tree = create_tree();
        let anchors = layout_anchors(&tree).unwrap();
        // The layout has the sidebar hidden, as at frame start.
        tree.set_visible(anchors.left_column, false);
        let mut state = GantzState::new();
        // `sidebar_open` was just toggled on mid-frame.
        state.view_toggles.sidebar_open = true;
        state.sidebar_width = 240.0;
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        capture_fixed_sizes(&tree, &mut state, area);
        assert!((state.sidebar_width - 240.0).abs() < 0.01);
    }

    fn node_view(head: &str, path: &[node::Id]) -> Pane {
        Pane::NodeView(NodeViewPane {
            head: gantz_ca::Head::Branch(head.parse().unwrap()),
            path: path.to_vec(),
            ty_name: "plot".to_string(),
        })
    }

    /// Pane identity keys are stable per pane and independent of a node view's
    /// `ty_name`. Two views of the same node are the same identity.
    #[test]
    fn pane_key_identity() {
        assert_eq!(pane_key(&Pane::Logs), "logs");
        assert_ne!(pane_key(&Pane::Logs), pane_key(&Pane::Steel));
        assert_eq!(pane_key(&Pane::GuiPreview), "gui-preview");
        assert_eq!(pane_key(&Pane::GuiTree), "gui-tree");

        let plot = node_view("main", &[3]);
        let same_node_number = Pane::NodeView(NodeViewPane {
            head: gantz_ca::Head::Branch("main".parse().unwrap()),
            path: vec![3],
            ty_name: "number".to_string(),
        });
        assert_eq!(pane_key(&plot), pane_key(&same_node_number));
        assert_ne!(pane_key(&plot), pane_key(&node_view("main", &[4])));
        assert_ne!(pane_key(&plot), pane_key(&node_view("other", &[3])));
    }

    /// `push_windowed` dedupes by pane identity, so a repeated pop-out or a
    /// same-node view with a different `ty_name` does not add a second entry.
    #[test]
    fn push_windowed_dedupes() {
        let mut windowed = Vec::new();
        push_windowed(&mut windowed, Pane::Logs);
        push_windowed(&mut windowed, Pane::Logs);
        push_windowed(&mut windowed, node_view("main", &[3]));
        push_windowed(
            &mut windowed,
            Pane::NodeView(NodeViewPane {
                head: gantz_ca::Head::Branch("main".parse().unwrap()),
                path: vec![3],
                ty_name: "number".to_string(),
            }),
        );
        assert_eq!(windowed.len(), 2);
    }

    /// After a node removal, a windowed view of the removed node is dropped and a
    /// view of a swapped node has its path rewritten. Other heads and non-view
    /// panes are untouched.
    #[test]
    fn migrate_windowed_node_views_drops_and_rewrites() {
        let head = gantz_ca::Head::Branch("main".parse().unwrap());
        let mut windowed = vec![
            node_view("main", &[1]),  // removed node, dropped
            node_view("main", &[3]),  // swapped from 3 to 1
            node_view("main", &[2]),  // unaffected
            node_view("other", &[1]), // different head, untouched
            Pane::Logs,               // non-view, untouched
        ];
        // Node 1 is removed. The node that was at index 3 swaps down into
        // slot 1.
        let reindex = crate::ops::Reindex(vec![crate::ops::RemoveOp {
            removed: 1,
            moved_from: Some(3),
        }]);
        migrate_windowed_node_views(&mut windowed, &head, &reindex);

        let main_paths: Vec<Vec<node::Id>> = windowed
            .iter()
            .filter_map(|p| match p {
                Pane::NodeView(nv) if nv.head == head => Some(nv.path.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(main_paths, vec![vec![1], vec![2]]);
        assert!(windowed.iter().any(|p| matches!(p, Pane::Logs)));
        assert!(
            windowed
                .iter()
                .any(|p| matches!(p, Pane::NodeView(nv) if matches!(&nv.head, gantz_ca::Head::Branch(n) if n.to_string() == "other")))
        );
        assert_eq!(windowed.len(), 4);
    }
}
