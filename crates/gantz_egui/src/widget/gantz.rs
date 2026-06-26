use crate::{
    CopyNodes, CreateNestedGraph, CreateNode, ExportAllNamed, ExportHead, HeadAccess, NodeCtx,
    NodeUi, OpenCommandPalette, OpenLogs, Paste, Redo, Registry, ReplaceHead, ResetTilesLayout,
    Undo, export,
    response::{DynResponse, Responses},
    widget::{self, GraphScene, GraphSceneState, graph_scene},
};
use gantz_core::{Node, node};
use petgraph::visit::{IntoNodeReferences, NodeRef};
use std::collections::{BTreeSet, HashMap};
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
    /// Graphs pane: merge registry + views only.
    Graphs,
    /// GraphScene pane: merge + open the root graph if unique.
    GraphScene,
}

/// The reserved node-type name that creates a new nested graph.
///
/// Selecting this entry in the command palette emits a
/// [`CreateNestedGraph`] rather than a [`CreateNode`], so a nested graph is
/// created like any other node but routed through the registry-aware op.
pub const NESTED_GRAPH_TYPE: &str = "graph";

/// A registry of available node types.
///
/// Provides the list of node type names available for creation.
/// Actual node creation is handled via [`crate::CreateNode`].
pub trait NodeTypeRegistry {
    /// The unique name of each node available.
    fn node_types(&self) -> Vec<&str>;

    /// The formatted keyboard shortcut for the command palette.
    fn command_formatted_kb_shortcut(
        &self,
        _ctx: &egui::Context,
        _node_type: &str,
    ) -> Option<String> {
        None
    }
}

/// The top-level gantz widget.
pub struct Gantz<'a> {
    env: &'a dyn Registry,
    base_names: &'a gantz_ca::registry::Names,
    demos: Option<&'a HashMap<String, String>>,
    log_source: Option<LogSource>,
    perf_vm: Option<&'a mut widget::PerfCapture>,
    perf_gui: Option<&'a mut widget::PerfCapture>,
    base_immutable: bool,
    compile_config: Option<gantz_core::compile::Config>,
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
    pub command_palette: widget::CommandPalette,
    /// Global auto-layout parameters (the non-flow `egui_graph` layout params;
    /// flow stays per-head in [`OpenHeadState::layout_flow`]).
    #[serde(default)]
    pub layout_config: LayoutConfig,
    /// Per-head redo stacks for undo/redo support.
    #[serde(default, serialize_with = "gantz_ca::serde_sorted::serialize_map")]
    pub redo_stacks: HashMap<gantz_ca::Head, Vec<gantz_ca::CommitAddr>>,
    /// The sidebar's pixel width, maintained across window resizes (fixed, not
    /// proportional). Updated when the user drags the divider.
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    /// The bottom tray's pixel height, maintained across window resizes.
    #[serde(default = "default_tray_height")]
    pub tray_height: f32,
}

/// The default fixed sidebar width, in points.
fn default_sidebar_width() -> f32 {
    270.0
}

/// The default fixed bottom-tray height, in points.
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
}

fn default_layout_flow() -> egui::Direction {
    GantzState::DEFAULT_DIRECTION
}

impl Default for OpenHeadState {
    fn default() -> Self {
        Self {
            scene: GraphSceneState::default(),
            layout_flow: GantzState::DEFAULT_DIRECTION,
        }
    }
}

/// Global auto-layout parameters, mirroring the non-flow fields of
/// [`egui_graph::LayoutParams`]. Flow stays per-head (see
/// [`OpenHeadState::layout_flow`]); these apply to every head's auto-layout.
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

/// A pane within the outer tree.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Pane {
    GraphConfig,
    /// Contains the inner graph tree with all open graph tabs.
    GraphScene,
    Graphs,
    GuiPerf,
    History,
    Logs,
    NodeInspector,
    /// Globally relevant configuration grouped into Panes / Style / Global
    /// subtabs (pane visibility, style, compile options, reset all demos).
    Settings,
    Steel,
    VmPerf,
}

/// A pane within the inner graph tree.
/// Contains the head (branch or commit) that this pane displays.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
struct GraphPane(gantz_ca::Head);

/// The egui ID used to store the inner graph tree.
const GRAPH_TREE_ID: &str = "gantz-graph-tiles-tree";

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
    ctx.memory_mut(|m| {
        let Some(tree) = m
            .data
            .get_persisted_mut_or_default::<Option<egui_tiles::Tree<GraphPane>>>(graph_tree_id)
            .as_mut()
        else {
            return;
        };
        // Find and update the pane with the old head.
        for (_, tile) in tree.tiles.iter_mut() {
            if let egui_tiles::Tile::Pane(GraphPane(head)) = tile {
                if head == old_head {
                    *head = new_head.clone();
                    break;
                }
            }
        }
    });
}

/// The context passed to the `egui_tiles::Tree` widget.
struct TreeBehaviour<'a, 's, Access>
where
    Access: HeadAccess,
    Access::Node: Node + NodeUi,
{
    gantz: &'a mut Gantz<'s>,
    state: &'a mut GantzState,
    access: &'a mut Access,
    focused_head: usize,
    base_names: &'a gantz_ca::registry::Names,
    gantz_response: &'a mut GantzResponse,
}

/// Response from the top-level gantz widget.
///
/// Whole-widget outcomes (focus, tab management, file drops, config) are
/// plain fields; operations emitted from deeper within the widget tree
/// (node UIs, context menus, shortcuts) arrive as dynamic payloads in
/// [`responses`][Self::responses] for the application to drain and handle.
#[derive(Debug)]
pub struct GantzResponse {
    /// The focused head index (may have changed due to user interaction).
    pub focused_head: usize,
    pub graph_select: Option<widget::graph_select::GraphSelectResponse>,
    /// Heads that were closed via the tab close button.
    pub closed_heads: Vec<gantz_ca::Head>,
    /// New branch created from tab double-click: (original_head, new_branch_name).
    pub new_branch: Option<(gantz_ca::Head, String)>,
    /// Files dropped onto gantz panes.
    pub file_drops: Vec<FileDrop>,
    /// Demo graph association changed: (head, Some(demo_name) | None).
    pub demo_changed: Option<(gantz_ca::Head, Option<String>)>,
    /// A named graph's description was edited: (head, new_description). An empty
    /// string clears the description.
    pub description_changed: Option<(gantz_ca::Head, String)>,
    /// A base graph should be reset to its original state.
    pub reset_base_graph: Option<gantz_ca::Head>,
    /// All `demo-*` base graphs should be reset to their original state.
    pub reset_all_demos: bool,
    /// The global compile config was changed via the Graph Config pane.
    pub compile_config: Option<gantz_core::compile::Config>,
    /// Dynamic payloads emitted from within the widget tree, tagged with the
    /// emitting head. See [`crate::response`] for the handling contract.
    pub responses: Responses,
}

/// State for editing a tab name via double-click.
#[derive(Clone, Default)]
struct TabEditState {
    /// The tile currently being edited, if any.
    editing_tile_id: Option<egui_tiles::TileId>,
    /// The text being edited.
    edit_text: String,
    /// Whether we need to request focus on the next frame.
    request_focus: bool,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ViewToggles {
    /// Whether the sidebar (left column) is open. Toggled by the hamburger.
    pub sidebar_open: bool,
    pub graphs: bool,
    pub history: bool,
    pub settings: bool,
    pub logs: bool,
    pub node_inspector: bool,
    pub perf_gui: bool,
    pub perf_vm: bool,
    pub steel: bool,
    pub graph_config: bool,
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
            graph_config: true,
        }
    }
}

struct NodeTyCmd<'a> {
    env: &'a dyn Registry,
    name: &'a str,
}

impl GantzResponse {
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
    pub fn graph_name_removed(&self) -> Option<String> {
        self.graph_select
            .as_ref()
            .and_then(|g| g.name_removed.clone())
    }

    /// New branch created from tab double-click: (original_head, new_branch_name).
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
    pub fn new(env: &'a dyn Registry, base_names: &'a gantz_ca::registry::Names) -> Self {
        Self {
            env,
            base_names,
            demos: None,
            log_source: None,
            perf_vm: None,
            perf_gui: None,
            base_immutable: true,
            compile_config: None,
        }
    }

    /// Provide demo graph associations for the config dropdown.
    pub fn demos(mut self, demos: &'a HashMap<String, String>) -> Self {
        self.demos = Some(demos);
        self
    }

    /// Provide the current compile config so the Graph Config pane shows
    /// the compile toggles. The config is global: it applies to all open
    /// heads, and a change is reported via [`GantzResponse::compile_config`].
    pub fn compile_config(mut self, config: gantz_core::compile::Config) -> Self {
        self.compile_config = Some(config);
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

    /// Whether base node graphs should be immutable (view-only).
    ///
    /// When `true` (the default), graphs for heads whose branch name
    /// appears in `base_names` are shown in immutable mode - navigation
    /// and selection work, but structural edits are disabled.
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
    /// Returns a response containing the (possibly updated) focused head index.
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
        Access::Node: Node + NodeUi,
    {
        // The persisted outer tree. The version suffix invalidates any tree
        // persisted before the sidebar overhaul (which lacks the new panes and
        // tab containers), forcing a rebuild via `create_tree`.
        let tree_id = egui::Id::new("gantz-tiles-tree-storage-v3");

        // Retrieve the tree of persistent storage, or load the default.
        let mut tree: egui_tiles::Tree<Pane> = ui
            .memory_mut(|m| {
                m.data
                    .get_persisted_mut_or_default::<Option<egui_tiles::Tree<Pane>>>(tree_id)
                    .take()
            })
            .unwrap_or_else(create_tree);

        // Check the `view_toggles` match the pane visibility.
        set_tile_visibility(&mut tree, &state.view_toggles);

        // Simplify the tree, and ensure tabs are where they should be.
        simplify_tree(&mut tree, ui.ctx());

        // Maintain a fixed sidebar width / tray height across window resizes by
        // imposing the stored pixel sizes on the share splits before layout.
        // `available_rect_before_wrap` matches the root rect `Tree::ui` uses.
        let widget_area = ui.available_rect_before_wrap();
        impose_fixed_sizes(&mut tree, state, widget_area);

        // Initialise the response.
        // We'll collect it during traversal of the tree of tiles.
        let mut response = GantzResponse {
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
            responses: Responses::default(),
        };

        // The context for traversing the tree of tiles.
        let base_names = self.base_names;
        let mut behaviour = TreeBehaviour {
            gantz: &mut self,
            state: &mut *state,
            access,
            focused_head,
            base_names,
            gantz_response: &mut response,
        };
        tree.ui(&mut behaviour, ui);

        // Update the response with the final focused head.
        response.focused_head = behaviour.focused_head;

        // Capture the sidebar width / tray height from the laid-out tree (which
        // reflects any manual divider drag) to re-impose next frame.
        capture_fixed_sizes(&tree, state, widget_area);

        // Detect .gantz file drops globally (not per-pane, since pointer
        // position may be unavailable during OS file drags on some platforms).
        response.file_drops = collect_gantz_file_drops(ui.ctx());

        // Apply payloads that only affect the widget's own state, so
        // applications never see them: command palette toggling and resetting
        // the tile layout to its default arrangement.
        for _ in response.responses.take::<OpenCommandPalette>() {
            state.command_palette.toggle();
        }
        for _ in response.responses.take::<ResetTilesLayout>() {
            tree = create_tree();
            // Restore the default sidebar width / tray height too.
            state.sidebar_width = default_sidebar_width();
            state.tray_height = default_tray_height();
            // Restore default pane visibility (e.g. the perf panes turn off),
            // but keep the sidebar's open/closed state since the user just
            // acted from within it.
            let sidebar_open = state.view_toggles.sidebar_open;
            state.view_toggles = ViewToggles::default();
            state.view_toggles.sidebar_open = sidebar_open;
        }
        for _ in response.responses.take::<OpenLogs>() {
            state.view_toggles.logs = true;
        }

        // Persist the tree.
        ui.memory_mut(|m| m.data.insert_persisted(tree_id, Some(tree)));

        response
    }
}

impl GantzState {
    pub const DEFAULT_DIRECTION: egui::Direction = egui::Direction::TopDown;

    /// Shorthand for initialising graph state, with no intial layout so that on
    /// the first pass, the layout is automatically determined.
    pub fn new() -> Self {
        Self::from_open_heads(Default::default())
    }

    pub fn from_open_heads(open_heads: OpenHeadStates) -> Self {
        Self {
            open_heads,
            command_palette: widget::CommandPalette::default(),
            view_toggles: ViewToggles::default(),
            layout_config: LayoutConfig::default(),
            redo_stacks: HashMap::new(),
            sidebar_width: default_sidebar_width(),
            tray_height: default_tray_height(),
        }
    }

    /// Migrate GUI state when a head's identity changes.
    ///
    /// Moves `open_heads` entry from old to new key. When `clear_redo` is
    /// true (new edit commit), removes redo stacks for both keys. Otherwise
    /// migrates the redo stack to the new key.
    pub fn migrate_head(&mut self, old: &gantz_ca::Head, new: &gantz_ca::Head, clear_redo: bool) {
        if let Some(state) = self.open_heads.remove(old) {
            self.open_heads.insert(new.clone(), state);
        }
        if clear_redo {
            self.redo_stacks.remove(old);
            self.redo_stacks.remove(new);
        } else if let Some(stack) = self.redo_stacks.remove(old) {
            self.redo_stacks.insert(new.clone(), stack);
        }
    }
}

impl<'a, 's, Access> egui_tiles::Behavior<Pane> for TreeBehaviour<'a, 's, Access>
where
    Access: HeadAccess,
    Access::Node: Node + NodeUi,
{
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        match pane {
            Pane::GraphConfig => match self.access.heads().get(self.focused_head) {
                Some(head) => format!("Graph - {head}").into(),
                None => "Graph".into(),
            },
            Pane::GraphScene => "Graphs".into(),
            Pane::Graphs => "Graphs".into(),
            Pane::GuiPerf => "GUI Perf".into(),
            Pane::History => "History".into(),
            Pane::Settings => "Settings".into(),
            Pane::Logs => match self.gantz.log_source {
                None => "Logs (No Source)".into(),
                Some(LogSource::Logger(_)) => "Logs".into(),
                #[cfg(feature = "tracing")]
                Some(LogSource::TraceCapture(..)) => "Tracing".into(),
            },
            Pane::NodeInspector => match self.access.heads().get(self.focused_head) {
                Some(head) => format!("Node Inspector - {head}").into(),
                None => "Node Inspector".into(),
            },
            Pane::Steel => match self.access.heads().get(self.focused_head) {
                Some(head) => format!("Steel - {head}").into(),
                None => "Steel".into(),
            },
            Pane::VmPerf => "VM Perf".into(),
        }
    }

    fn on_tab_button(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
        button_response: egui::Response,
    ) -> egui::Response {
        // Right-click a (hideable) tab to hide its pane.
        let pane = match tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(pane)) if pane_is_hideable(pane) => pane.clone(),
            _ => return button_response,
        };
        button_response.context_menu(|ui| {
            if ui.button("hide").clicked() {
                set_pane_visible(&mut self.state.view_toggles, &pane, false);
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
        // The tray panes get a close button; the rest are toggled via the
        // Panes settings or the tab right-click menu.
        matches!(tiles.get_pane(&tile_id), Some(Pane::Logs | Pane::Steel))
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        // Hide the pane via its toggle (so it can be reopened) rather than
        // letting egui_tiles remove the tile from the tree.
        if let Some(pane) = tiles.get_pane(&tile_id).cloned() {
            set_pane_visible(&mut self.state.view_toggles, &pane, false);
        }
        false
    }

    fn tab_ui(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        ui: &mut egui::Ui,
        id: egui::Id,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        // Render with the shared `Tab` widget so the sidebar/tray tabs (and
        // their small close button) match the graph tabs.
        let title = self.tab_title_for_tile(tiles, tile_id);
        let res = widget::Tab::new(title, id)
            .active(state.active)
            .closable(state.closable)
            .show(ui);
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
        _resize_state: egui_tiles::ResizeState,
    ) -> egui::Stroke {
        let w = 2.0;
        egui::Stroke::new(w, style.visuals.extreme_bg_color)
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
        // We will manually simplify before calling `tree.ui`. See `simplify_tree`.
        egui_tiles::SimplificationOptions::OFF
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut Pane,
    ) -> egui_tiles::UiResponse {
        let Self {
            ref mut gantz,
            ref mut state,
            ref mut access,
            ref mut focused_head,
            ref base_names,
            ref mut gantz_response,
        } = *self;
        match pane {
            Pane::GraphConfig => match access.heads().get(*focused_head).cloned() {
                Some(head) => {
                    let head_state = state.open_heads.entry(head.clone()).or_default();
                    let names = gantz.env.names();
                    let is_base = match &head {
                        gantz_ca::Head::Branch(name) => base_names.contains_key(name),
                        _ => false,
                    };
                    let immutable = head_immutable(&head, gantz.base_immutable, base_names);

                    // Collect demo-* names for the dropdown.
                    let demo_names_vec: Vec<&str> = names
                        .keys()
                        .filter(|n| n.starts_with("demo-"))
                        .map(|n| n.as_str())
                        .collect();

                    // Look up the current demo association for this head.
                    let current_demo = match &head {
                        gantz_ca::Head::Branch(name) => {
                            gantz.demos.and_then(|d| d.get(name)).map(|s| s.as_str())
                        }
                        _ => None,
                    };

                    // The graph's current description (named graphs only).
                    let current_description = match &head {
                        gantz_ca::Head::Branch(name) => gantz.env.graph_description(name),
                        _ => None,
                    };

                    let res = pane_ui(ui, |ui| {
                        widget::GraphConfig::new(&head, head_state, names)
                            .is_base(is_base)
                            .immutable(immutable)
                            .demo_names(&demo_names_vec)
                            .current_demo(current_demo)
                            .current_description(current_description)
                            .show(ui)
                    });
                    if res.inner.new_branch.is_some() {
                        gantz_response.new_branch = res.inner.new_branch;
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

                // We'll use this to position the floating sidebar toggle.
                let rect = ui.available_rect_before_wrap();

                // Retrieve the inner graph tree from persistent storage, or create empty.
                let graph_tree_id = egui::Id::new(GRAPH_TREE_ID);
                let mut graph_tree: egui_tiles::Tree<GraphPane> = ui
                    .memory_mut(|m| {
                        m.data
                            .get_persisted_mut_or_default::<Option<egui_tiles::Tree<GraphPane>>>(
                                graph_tree_id,
                            )
                            .take()
                    })
                    .unwrap_or_else(create_empty_graph_tree);

                // Sync the graph tree panes with the heads list.
                sync_graph_panes(&mut graph_tree, access.heads());

                // Activate the tab corresponding to the focused head.
                if let Some(fh) = access.heads().get(*focused_head) {
                    let fh = fh.clone();
                    graph_tree.make_active(|_, tile| match tile {
                        egui_tiles::Tile::Pane(GraphPane(head)) => *head == fh,
                        _ => false,
                    });
                }

                // Render the inner tree.
                let mut graph_behaviour = GraphTreeBehaviour {
                    env: gantz.env,
                    access: *access,
                    state,
                    focused_head,
                    closed_heads: &mut gantz_response.closed_heads,
                    new_branch: &mut gantz_response.new_branch,
                    responses: &mut gantz_response.responses,
                    base_names,
                    base_immutable: gantz.base_immutable,
                };
                graph_tree.ui(&mut graph_behaviour, ui);

                // Persist the inner tree.
                ui.memory_mut(|m| m.data.insert_persisted(graph_tree_id, Some(graph_tree)));

                // Show the command palette once (not per-pane), operating on the focused head.
                if let Some(fh) = access.heads().get(*focused_head).cloned() {
                    let focused_immutable = head_immutable(&fh, gantz.base_immutable, base_names);

                    let head_state = state.open_heads.entry(fh.clone()).or_default();

                    // Copy/paste/undo/redo keyboard shortcuts.
                    if !ui.ctx().egui_wants_keyboard_input() {
                        // Copy is always allowed.
                        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::C)) {
                            let nodes = head_state.scene.interaction.selection.nodes.clone();
                            gantz_response
                                .responses
                                .push(Some(fh.clone()), CopyNodes(nodes));
                        }
                        // New graph: Cmd/Ctrl+N.
                        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::N)) {
                            let gs = gantz_response
                                .graph_select
                                .get_or_insert_with(Default::default);
                            gs.new_graph = true;
                        }
                        // Paste, undo, redo are gated by immutable.
                        if !focused_immutable {
                            // Detect paste: Event::Paste (eframe/web) or Ctrl+V
                            // key press (bevy_egui desktop, which sends Event::Text
                            // instead of Event::Paste).
                            let paste_text = ui.input(|i| {
                                i.events.iter().find_map(|e| match e {
                                    egui::Event::Paste(s) => Some(s.clone()),
                                    _ => None,
                                })
                            });
                            let ctrl_v = paste_text.is_some()
                                || ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::V));
                            if ctrl_v {
                                let paste = Paste {
                                    text: paste_text,
                                    pos: crate::PastePos::Offset(egui::vec2(20.0, 20.0)),
                                };
                                gantz_response.responses.push(Some(fh.clone()), paste);
                            }
                            // Undo: Cmd/Ctrl+Z (without Shift).
                            if ui.input(|i| {
                                i.modifiers.command
                                    && !i.modifiers.shift
                                    && i.key_pressed(egui::Key::Z)
                            }) {
                                gantz_response.responses.push(Some(fh.clone()), Undo);
                            }
                            // Redo: Cmd/Ctrl+Shift+Z or Cmd/Ctrl+Y.
                            if ui.input(|i| {
                                (i.modifiers.command
                                    && i.modifiers.shift
                                    && i.key_pressed(egui::Key::Z))
                                    || (i.modifiers.command && i.key_pressed(egui::Key::Y))
                            }) {
                                gantz_response.responses.push(Some(fh.clone()), Redo);
                            }
                        }
                    }

                    // Skip command palette when immutable.
                    if !focused_immutable {
                        let editing = match &fh {
                            gantz_ca::Head::Branch(name) => Some(name.as_str()),
                            _ => None,
                        };
                        // The pointer position over the focused head's scene
                        // (graph coords) recorded this frame; new nodes are placed
                        // here. `Copy`, so no borrow is held across the call.
                        let pointer_pos = head_state.scene.interaction.last_pointer_pos;
                        let created =
                            command_palette(gantz.env, editing, &mut state.command_palette, ui);
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
                // scene that opens/closes the sidebar (left column).
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
                let res = graph_select(
                    gantz.env,
                    heads,
                    *focused_head,
                    *base_names,
                    gantz.demos,
                    ui,
                );

                if res.inner.export_all {
                    gantz_response.responses.push(None, ExportAllNamed);
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
                    // focused head (the target encodes the node's path).
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
                            access.with_head_mut(fh, |data| {
                                for path in paths {
                                    // Log targets are state paths; only root-level
                                    // (single-segment) ones name a node in this graph.
                                    let [ix] = path[..] else { continue };
                                    let Some(node) =
                                        data.graph.node_weight_mut(graph_scene::NodeIndex::new(ix))
                                    else {
                                        continue;
                                    };
                                    labels.insert(path, node.name(env).to_string());
                                }
                            });
                        }
                    }
                    let res = log_view(logger, &labels, ui);
                    // Clicking an entry selects its node. Only root-level nodes
                    // live in the focused head; entries from a nested graph
                    // (deeper path) are skipped until name-based navigation lands.
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
                // Use the focused head for the node inspector.
                if let Some(fh) = access.heads().get(*focused_head).cloned() {
                    let immutable = head_immutable(&fh, gantz.base_immutable, base_names);
                    let head_state = state.open_heads.entry(fh.clone()).or_default();
                    let responses = access.with_head_mut(&fh, |data| {
                        node_inspector(gantz.env, data.graph, data.vm, head_state, immutable, ui)
                            .inner
                    });
                    gantz_response
                        .responses
                        .extend(Some(&fh), responses.into_iter().flatten());
                }
            }
            Pane::Steel => {
                // Use the focused head's compiled module, highlighting the
                // selected nodes' emitted fns/call sites and any diagnostic
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
                            // A node at this (root) level has the single-element
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
            Pane::VmPerf => {
                if let Some(ref mut capture) = gantz.perf_vm {
                    perf_view("VM Perf", capture, ui);
                }
            }
            Pane::Settings => {
                let compile_config = gantz.compile_config;
                let res = pane_ui(ui, |ui| {
                    widget::settings(
                        &mut state.view_toggles,
                        compile_config,
                        &mut state.layout_config,
                        ui,
                    )
                });
                if let Some(cfg) = res.inner.compile_config {
                    gantz_response.compile_config = Some(cfg);
                }
                if res.inner.reset_all_demos {
                    gantz_response.reset_all_demos = true;
                }
                if res.inner.reset_layout {
                    gantz_response.responses.push(None, ResetTilesLayout);
                }
            }
        }
        egui_tiles::UiResponse::None
    }
}

/// The context passed to the inner graph `egui_tiles::Tree` widget.
struct GraphTreeBehaviour<'a, Access>
where
    Access: HeadAccess,
    Access::Node: Node + NodeUi,
{
    env: &'a dyn Registry,
    access: &'a mut Access,
    state: &'a mut GantzState,
    focused_head: &'a mut usize,
    /// Heads closed via the tab close button.
    closed_heads: &'a mut Vec<gantz_ca::Head>,
    /// New branch created from tab double-click: (original_head, new_branch_name).
    new_branch: &'a mut Option<(gantz_ca::Head, String)>,
    /// Dynamic payloads emitted from within the graph scenes.
    responses: &'a mut Responses,
    base_names: &'a gantz_ca::registry::Names,
    base_immutable: bool,
}

impl<'a, Access> egui_tiles::Behavior<GraphPane> for GraphTreeBehaviour<'a, Access>
where
    Access: HeadAccess,
    Access::Node: Node + NodeUi,
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
        _resize_state: egui_tiles::ResizeState,
    ) -> egui::Stroke {
        let w = 2.0;
        egui::Stroke::new(w, style.visuals.extreme_bg_color)
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
        // Allow closing tabs if there's more than one head open.
        self.access.heads().len() > 1
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<GraphPane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        // Get the head from the pane being closed.
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
        // Load tab edit state from temp memory.
        let edit_state_id = egui::Id::new("tab_edit_state");
        let mut edit_state: TabEditState = ui
            .memory_mut(|m| m.data.get_temp(edit_state_id))
            .unwrap_or_default();

        let is_editing = edit_state.editing_tile_id == Some(tile_id);

        let response = if is_editing {
            let head = tiles.get_pane(&tile_id).map(|GraphPane(h)| h.clone());
            let names = self.env.names();

            let name_res = head.as_ref().map(|h| {
                ui.scope(|ui| {
                    ui.set_max_width(ui.available_width().min(150.0));
                    widget::head_name_edit(h, &mut edit_state.edit_text, names, ui)
                })
                .inner
            });

            let Some(name_res) = name_res else {
                edit_state.editing_tile_id = None;
                edit_state.edit_text.clear();
                // Store edit state back to temp memory.
                ui.memory_mut(|m| m.data.insert_temp(edit_state_id, edit_state));
                return ui.label("");
            };

            // Request focus on the first frame after entering edit mode.
            if edit_state.request_focus {
                name_res.response.request_focus();
                edit_state.request_focus = false;
            }

            // head_name_edit resets the text on commit/cancel, so detect
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
            // Render the tab using our custom widget.
            // Append a filled circle if this head is focused.
            let mut title = self.tab_title_for_tile(tiles, tile_id).text().to_string();
            if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
                let heads = self.access.heads();
                if crate::head_is_focused(heads, *self.focused_head, head) {
                    title.push_str(" ⚫");
                }
            }
            let res = widget::Tab::new(title, id)
                .active(state.active)
                .closable(state.closable)
                .hint("double-click to rename")
                .show(ui);

            // Handle double-click to enter edit mode.
            if res.tab.double_clicked() {
                if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
                    // Initialize edit text based on head type.
                    let initial_text = match head {
                        gantz_ca::Head::Branch(name) => name.clone(),
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

        // Store edit state back to temp memory.
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

        // Find the index of this head (for updating focused_head).
        let ix = self
            .access
            .heads()
            .iter()
            .position(|h| h == pane_head)
            .expect("pane head not found in heads");

        let immutable = head_immutable(pane_head, self.base_immutable, self.base_names);
        let diagnostics = self.access.diagnostics(pane_head).to_vec();

        // Global layout params (Copy) combined with this head's flow.
        let layout_config = self.state.layout_config;
        let head_state = self.state.open_heads.entry(pane_head.clone()).or_default();
        let layout_params = layout_config.to_params(head_state.layout_flow);
        // Disjoint borrow of a sibling field of `open_heads` for the graph
        // scene's "Panes" context submenu.
        let view_toggles = &mut self.state.view_toggles;

        // We'll use this for positioning the fixed path labels window.
        let rect = ui.available_rect_before_wrap();

        // Get mutable access to this head's data and render the graph scene.
        let graph_response = self.access.with_head_mut(pane_head, |data| {
            graph_scene(
                self.env,
                data.graph,
                pane_head,
                head_state,
                view_toggles,
                data.view,
                layout_params,
                immutable,
                &diagnostics,
                data.vm,
                ui,
            )
        });

        if let Some(Some(response)) = graph_response {
            // Focus this head when clicking on the graph or any of its nodes.
            if response.scene.clicked() || response.any_node_interacted() {
                *self.focused_head = ix;
            }
            // Tag the scene's emissions with this head.
            self.responses.extend(Some(&*pane_head), response.responses);
        }

        // Floating name breadcrumb for nested graphs.
        let crumbs = name_breadcrumb(rect, pane_head, ui);
        self.responses.extend(Some(&*pane_head), crumbs);

        egui_tiles::UiResponse::None
    }
}

impl widget::command_palette::Command for NodeTyCmd<'_> {
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
/// -----------------------------------------
/// |grs/hist/settings |scene               |
/// |------------------|                     |
/// |vm/gui            |                     |
/// |------------------|---------------------|
/// |conf              |logs      |steel     |
/// |------------------|          |          |
/// |insp              |          |          |
/// -----------------------------------------
///
/// The active tab of each tab container defaults to its first child (see
/// `egui_tiles::Tabs::new`), so child ordering picks the default tabs.
fn create_tree() -> egui_tiles::Tree<Pane> {
    let mut tiles = egui_tiles::Tiles::default();

    // The leaf panes.
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

    // Sidebar tab containers (first child is the default-active tab).
    let graphs_history_settings = tiles.insert_tab_tile(vec![graphs, history, settings]);
    let perf = tiles.insert_tab_tile(vec![vm_perf, gui_perf]);

    // The left column (sidebar).
    let mut shares = egui_tiles::Shares::default();
    shares.set_share(graphs_history_settings, 0.30);
    shares.set_share(perf, 0.10);
    shares.set_share(graph_config, 0.13);
    shares.set_share(node_inspector, 0.25);
    let left_column = tiles.insert_container(egui_tiles::Linear {
        children: vec![graphs_history_settings, perf, graph_config, node_inspector],
        dir: egui_tiles::LinearDir::Vertical,
        shares,
    });

    // Logs and steel code in bottom "tray".
    let tray = tiles.insert_horizontal_tile(vec![logs, steel]);

    // The right column with main area (graph scene above logs and steel code).
    let right_column = tiles.insert_container(egui_tiles::Linear::new_binary(
        egui_tiles::LinearDir::Vertical,
        [graph_scene, tray],
        0.7,
    ));

    // The root with both columns. The split here is only a fallback; the
    // sidebar normally has a fixed pixel width maintained across window resizes
    // (see `impose_fixed_sizes` / `default_sidebar_width`).
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

/// Sync the graph tree panes with the current heads.
///
/// Adds missing panes for new heads and removes panes for heads that no longer exist.
fn sync_graph_panes(tree: &mut egui_tiles::Tree<GraphPane>, heads: &[gantz_ca::Head]) {
    use std::collections::HashSet;

    // Collect existing heads in panes.
    let existing: HashSet<gantz_ca::Head> = tree
        .tiles
        .iter()
        .filter_map(|(_, tile)| match tile {
            egui_tiles::Tile::Pane(GraphPane(head)) => Some(head.clone()),
            _ => None,
        })
        .collect();

    // Collect current heads.
    let current: HashSet<gantz_ca::Head> = heads.iter().cloned().collect();

    // Add missing panes for heads that don't have a pane yet.
    for head in heads {
        if !existing.contains(head) {
            let pane_id = tree.tiles.insert_pane(GraphPane(head.clone()));
            // Add to root container, or set as root if tree is empty.
            if let Some(root_id) = tree.root() {
                tree.move_tile_to_container(pane_id, root_id, usize::MAX, true);
            } else {
                // Tree is empty, create a tabs container as root.
                let root = tree.tiles.insert_tab_tile(vec![pane_id]);
                tree.root = Some(root);
            }
        }
    }

    // Remove panes for heads that no longer exist.
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
    // Otherwise, find the graph scene ID.
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

/// The gap between sibling tiles, in points. Must match the (unoverridden)
/// default `egui_tiles::Behavior::gap_width`, so imposed pixel sizes are exact
/// and don't drift when re-imposed each frame.
const TILE_GAP: f32 = 1.0;

/// The minimum sidebar width / tray height, in points.
const MIN_PANE_SIZE: f32 = 80.0;

/// The tiles whose Linear share splits hold the sidebar width and tray height,
/// when the tree has its default top-level shape.
struct LayoutAnchors {
    /// Root horizontal Linear: `[left_column | right_column]`.
    root: egui_tiles::TileId,
    left_column: egui_tiles::TileId,
    /// Right column vertical Linear: `[graph_scene / tray]`.
    right_column: egui_tiles::TileId,
    graph_scene: egui_tiles::TileId,
    tray: egui_tiles::TileId,
}

/// Identify the layout anchors, or `None` if the tree isn't in its default
/// top-level shape (mid-drag, or after the user rearranged panes), in which
/// case the proportional layout is left untouched.
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

/// Impose the stored sidebar width / tray height (in points) on the tree's
/// share splits so they stay fixed as the window resizes. Call after
/// `simplify_tree`, before `tree.ui`.
fn impose_fixed_sizes(tree: &mut egui_tiles::Tree<Pane>, state: &GantzState, area: egui::Rect) {
    let Some(anchors) = layout_anchors(tree) else {
        return;
    };
    // Both columns span the full height, so the tray's available height is the
    // area height less the gap; the sidebar's available width likewise.
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
    if state.view_toggles.logs || state.view_toggles.steel {
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

/// Capture the sidebar width / tray height (in points) from the laid-out tree,
/// so they can be re-imposed next frame (including after manual divider drags).
/// Call after `tree.ui`.
///
/// This reads the post-layout *shares* rather than the cached rects: a resize
/// drag updates the shares during `tree.ui`, but the rects it computes reflect
/// the pre-drag split, so reading rects would never see the drag.
fn capture_fixed_sizes(tree: &egui_tiles::Tree<Pane>, state: &mut GantzState, area: egui::Rect) {
    let Some(anchors) = layout_anchors(tree) else {
        return;
    };
    // Gate on the *laid-out* visibility, not `sidebar_open`: the hamburger can
    // flip `sidebar_open` mid-frame, but `set_tile_visibility` only runs at the
    // frame start, so the layout (and thus the captured share) reflects the
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
/// visible children (mirroring `egui_tiles::Shares::split`).
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

/// Whether a tab's pane can be hidden via its right-click menu. The main graph
/// scene and the Settings control surface are not hideable this way.
fn pane_is_hideable(pane: &Pane) -> bool {
    !matches!(pane, Pane::GraphScene)
}

/// Set a pane's visibility toggle. No-op for panes without one.
fn set_pane_visible(view: &mut ViewToggles, pane: &Pane, visible: bool) {
    match pane {
        Pane::Graphs => view.graphs = visible,
        Pane::History => view.history = visible,
        Pane::Settings => view.settings = visible,
        Pane::GraphConfig => view.graph_config = visible,
        Pane::NodeInspector => view.node_inspector = visible,
        Pane::VmPerf => view.perf_vm = visible,
        Pane::GuiPerf => view.perf_gui = visible,
        Pane::Logs => view.logs = visible,
        Pane::Steel => view.steel = visible,
        Pane::GraphScene => {}
    }
}

/// Ensure the view toggles match the pane visibility.
fn set_tile_visibility(tree: &mut egui_tiles::Tree<Pane>, view: &ViewToggles) {
    let ids: Vec<_> = tree.tiles.tile_ids().collect();
    let open = view.sidebar_open;
    // Set visibility for panes. Sidebar content panes are gated by both the
    // sidebar being open and their individual toggle; the Settings control
    // pane is gated only by the sidebar being open; the tray panes
    // (Logs/Steel) are independent of the sidebar.
    for &id in &ids {
        if let Some(pane) = tree.tiles.get_pane(&id) {
            match pane {
                Pane::GraphScene => (),
                Pane::Settings => tree.set_visible(id, open && view.settings),
                Pane::GraphConfig => tree.set_visible(id, open && view.graph_config),
                Pane::Graphs => tree.set_visible(id, open && view.graphs),
                Pane::GuiPerf => tree.set_visible(id, open && view.perf_gui),
                Pane::History => tree.set_visible(id, open && view.history),
                Pane::NodeInspector => tree.set_visible(id, open && view.node_inspector),
                Pane::VmPerf => tree.set_visible(id, open && view.perf_vm),
                Pane::Logs => tree.set_visible(id, view.logs),
                Pane::Steel => tree.set_visible(id, view.steel),
            }
        }
    }
    // Set visibility for containers.
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
/// The overlay is best-effort: it only appears when the pointer position is
/// available and within the pane (some platforms don't track the pointer
/// during OS file drags).
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
/// is independent of pointer position (which may be unavailable during OS
/// file drags on some platforms). The target pane is determined by checking
/// the pointer against the stored Graphs pane rect when available, defaulting
/// to [`FileDropTarget::GraphScene`].
fn collect_gantz_file_drops(ctx: &egui::Context) -> Vec<FileDrop> {
    let dropped = ctx.input(|i| i.raw.dropped_files.clone());
    if dropped.is_empty() {
        return Vec::new();
    }

    // Determine target: Graphs if pointer is over the Graphs pane, else GraphScene.
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

/// Provides a consistent frame and styling for the panes.
fn pane_ui<R>(ui: &mut egui::Ui, pane: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::CentralPanel::default().show_inside(ui, |ui| pane(ui))
}

/// The size of the floating sidebar toggle glyph, also used to offset the
/// nested-graph breadcrumb to its right (they share the scene's bottom-left
/// corner).
const SIDEBAR_TOGGLE_ICON_SIZE: f32 = 18.0;

/// A floating hamburger button that toggles the sidebar open/closed.
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
                // A hamburger that toggles the sidebar. Idle, it matches the
                // faint colour of egui_graph's dot grid; on hover it brightens a
                // little to signal it's interactive (no selection colour when
                // open). Laid out manually so the colour can depend on hover.
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
    env: &dyn Registry,
    heads: &[gantz_ca::Head],
    focused_head: usize,
    base_names: &gantz_ca::registry::Names,
    demos: Option<&HashMap<String, String>>,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<widget::graph_select::GraphSelectResponse> {
    pane_ui(ui, |ui| {
        widget::GraphSelect::new(env, heads, base_names)
            .focused_head(focused_head)
            .demos(demos)
            .show(ui)
    })
}

fn history_view(
    env: &dyn Registry,
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
fn graph_scene<N>(
    registry: &dyn Registry,
    graph: &mut gantz_core::node::graph::Graph<N>,
    head: &gantz_ca::Head,
    head_state: &mut OpenHeadState,
    view_toggles: &mut ViewToggles,
    head_view: &mut egui_graph::View,
    layout_params: egui_graph::LayoutParams,
    immutable: bool,
    diagnostics: &[gantz_core::Diagnostic],
    vm: &mut Engine,
    ui: &mut egui::Ui,
) -> Option<graph_scene::GraphSceneResponse>
where
    N: Node + NodeUi,
{
    // A head shows exactly its root graph (nested graphs are separate heads).
    let id = egui::Id::new(head);

    // Seed the node layout the first time this graph is shown.
    if head_view.layout.is_empty() {
        head_view.layout =
            widget::graph_scene::layout(registry, graph, id, &layout_params, ui.ctx(), None);
    }

    let response = GraphScene::new(registry, graph)
        .with_id(id)
        .layout_params(layout_params)
        .immutable(immutable)
        .view_toggles(view_toggles)
        .show(head_view, &mut head_state.scene, vm, ui);

    graph_scene::paint_diagnostics(diagnostics, &[], &response, ui);

    Some(response)
}

/// Floating name breadcrumb over the bottom-left corner of the scene, shown
/// when viewing a nested graph (a `parent:child` head). Each crumb is a
/// `:`-separated name segment; the prefix it represents is the head it
/// navigates to.
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
    let sep = crate::node::NESTED_SEP;
    if !name.contains(sep) {
        return responses; // a root graph has no ancestor levels
    }
    let segs: Vec<&str> = name.split(sep).collect();
    let sep_str = sep.to_string();
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
                        // The crumbs are tiny: the root is `R` (its name is too
                        // big to fit), and each nested level is its short leaf.
                        let (label, hover) = if i == 0 {
                            ("R".to_string(), format!("navigate to {seg} root"))
                        } else {
                            (seg.to_string(), format!("navigate to {prefix}"))
                        };
                        ui.vertical_centered_justified(|ui| {
                            let resp = ui.add(button(&label)).on_hover_text(hover);
                            if resp.clicked() && !is_current {
                                responses.push(DynResponse::new(ReplaceHead(
                                    gantz_ca::Head::Branch(prefix),
                                )));
                            }
                        });
                    }
                })
        });
    responses
}

/// A node-creation choice made in the command palette.
enum PaletteChoice {
    /// Create an ordinary node of the given type.
    Node(CreateNode),
    /// Create a new nested graph (the reserved [`NESTED_GRAPH_TYPE`] entry).
    NestedGraph(CreateNestedGraph),
}

/// Returns a node-creation payload when a node type is chosen.
///
/// `editing` is the focused head's name (when it is a branch), used to hide node
/// types whose reference would cycle back to the graph being edited.
fn command_palette(
    env: &dyn Registry,
    editing: Option<&str>,
    cmd_palette: &mut widget::CommandPalette,
    ui: &mut egui::Ui,
) -> Option<PaletteChoice> {
    // If space is pressed, toggle command palette visibility.
    if !ui.ctx().egui_wants_keyboard_input() {
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) {
            cmd_palette.toggle();
        }
    }

    // Map the node types to commands for the command palette, dropping any type
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
    // palette is centered over the graph scene (this `ui`'s rect).
    let scene_rect = ui.max_rect();
    cmd_palette.show(ui.ctx(), scene_rect, cmds).map(|cmd| {
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
/// graph that is not a demo (demo base graphs are always mutable so users can
/// experiment).
fn head_immutable(
    head: &gantz_ca::Head,
    base_immutable: bool,
    base_names: &gantz_ca::registry::Names,
) -> bool {
    let is_base = matches!(head, gantz_ca::Head::Branch(name) if base_names.contains_key(name));
    let is_demo = matches!(head, gantz_ca::Head::Branch(name) if name.starts_with("demo-"));
    base_immutable && is_base && !is_demo
}

/// Returns the payloads emitted by node UIs within the inspector.
fn node_inspector<N>(
    registry: &dyn Registry,
    root: &mut gantz_core::node::graph::Graph<N>,
    vm: &mut Engine,
    head_state: &mut OpenHeadState,
    immutable: bool,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<Vec<DynResponse>>
where
    N: Node + NodeUi,
{
    pane_ui(ui, |ui| {
        let mut responses = Vec::new();
        egui::ScrollArea::vertical()
            .auto_shrink(egui::Vec2b::FALSE)
            .show(ui, |ui| {
                let graph = &mut *root;
                let ids: Vec<_> = graph.node_references().map(|n_ref| n_ref.id()).collect();
                // Collect the inlets and outlets.
                let (inlets, outlets) = crate::inlet_outlet_ids(registry, graph);
                for id in ids {
                    let mut frame = egui::Frame::group(ui.style());
                    if head_state.scene.interaction.selection.nodes.contains(&id) {
                        frame.stroke.color = ui.visuals().selection.stroke.color;
                    }
                    frame.show(ui, |ui| {
                        let Some(node) = graph.node_weight_mut(id) else {
                            return;
                        };
                        let ix = id.index();
                        let path = [ix];
                        let ctx = NodeCtx::new(
                            registry,
                            &path[..],
                            &inlets,
                            &outlets,
                            vm,
                            &mut responses,
                        );
                        let resp = widget::NodeInspector::new(node, ctx, immutable).show(ui);
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
                }
            });
        responses
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

    /// The fixed sidebar width must be imposed on the sidebar (left column),
    /// not the main area, and the anchors must identify the sidebar as the
    /// column that does not contain the graph scene.
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
        // The sidebar gets the fixed width; the main area gets the remainder.
        assert!((root.shares[anchors.left_column] - 240.0).abs() < 0.01);
        assert!((root.shares[anchors.right_column] - (avail - 240.0)).abs() < 0.01);
    }

    /// `capture_fixed_sizes` must recover the same width `impose_fixed_sizes`
    /// set, so a sidebar that isn't dragged doesn't drift frame to frame.
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
    /// column hidden; capturing then would size it against the wrong siblings.
    #[test]
    fn capture_skips_while_sidebar_laid_out_hidden() {
        let mut tree = create_tree();
        let anchors = layout_anchors(&tree).unwrap();
        // Layout state: sidebar hidden (as at frame start)...
        tree.set_visible(anchors.left_column, false);
        let mut state = GantzState::new();
        // ...but `sidebar_open` was just toggled on mid-frame.
        state.view_toggles.sidebar_open = true;
        state.sidebar_width = 240.0;
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        capture_fixed_sizes(&tree, &mut state, area);
        assert!((state.sidebar_width - 240.0).abs() < 0.01);
    }
}
