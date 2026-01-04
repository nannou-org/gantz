use crate::{
    Cmd, NodeCtx, NodeUi,
    widget::{
        self, GraphScene, GraphSceneState,
        graph_scene::{self, ToGraphMut},
    },
};
use gantz_core::{Node, node};
use petgraph::visit::{IntoNodeReferences, NodeRef};
use std::collections::HashMap;
use steel::steel_vm::engine::Engine;

/// A registry of available nodes.
///
/// This should be implemented for the `Node`'s input `Env` type.
pub trait NodeTypeRegistry {
    /// The gantz node type that can be produced by the registry.
    type Node;

    /// The unique name of each node available.
    fn node_types(&self) -> impl Iterator<Item = &str>;

    /// Create a node of the given type name.
    fn new_node(&self, node_type: &str) -> Option<Self::Node>;

    /// The tooltip shown for this node type within the command palette.
    fn command_tooltip(&self, _node_type: &str) -> &str {
        ""
    }

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
pub struct Gantz<'a, Env>
where
    Env: NodeTypeRegistry,
{
    env: &'a mut Env,
    heads: &'a mut [(
        gantz_ca::Head,
        &'a mut gantz_core::node::graph::Graph<Env::Node>,
    )],
    log_source: Option<LogSource>,
}

enum LogSource {
    Logger(widget::log_view::Logger),
    #[cfg(feature = "tracing")]
    TraceCapture(widget::trace_view::TraceCapture),
}

/// All state for the widget.
#[derive(serde::Deserialize, serde::Serialize)]
pub struct GantzState {
    /// State for each open head.
    pub open_heads: OpenHeadStates,
    /// The index of the currently focused head (into the `heads` slice).
    pub focused_head: usize,
    pub view_toggles: ViewToggles,
    pub command_palette: widget::CommandPalette,
    pub auto_layout: bool,
    pub layout_flow: egui::Direction,
    pub center_view: bool,
}

pub type OpenHeadStates = HashMap<gantz_ca::Head, OpenHeadState>;

/// State associated with a single open root graph.
#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct OpenHeadState {
    /// The path to the currently visible graph within the open graph tree.
    pub path: Vec<node::Id>,
    /// State associated with the `GraphScene` widget.
    pub scene: GraphSceneState,
    /// State for each individual graph at each unique path within the tree.
    pub graphs: GraphStates,
}

/// A pane within the outer tree.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Pane {
    GraphConfig,
    /// Contains the inner graph tree with all open graph tabs.
    GraphScene,
    GraphSelect,
    Logs,
    NodeInspector,
    Steel,
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
struct TreeBehaviour<'a, 'g, Env>
where
    Env: NodeTypeRegistry,
{
    gantz: &'a mut Gantz<'g, Env>,
    state: &'a mut GantzState,
    /// Per-head compiled steel modules.
    compiled_modules: &'a [String],
    /// Per-head VMs.
    vms: &'a mut [Engine],
    gantz_response: &'a mut GantzResponse,
}

/// Response from the top-level gantz widget.
#[derive(Debug, Default)]
pub struct GantzResponse {
    pub graph_select: Option<widget::graph_select::GraphSelectResponse>,
    /// Heads that were closed via the tab close button.
    pub closed_heads: Vec<gantz_ca::Head>,
}

/// UI state relevant to each nested graph within the tree.
pub type GraphStates = HashMap<Vec<node::Id>, GraphState>;

/// UI state relevant to a graph at a certain path within the root.
#[derive(serde::Deserialize, serde::Serialize)]
pub struct GraphState {
    pub view: egui_graph::View,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct ViewToggles {
    pub graph_select: bool,
    pub node_inspector: bool,
    pub logs: bool,
    pub steel: bool,
    pub graph_config: bool,
}

struct NodeTyCmd<'a, Env> {
    env: &'a Env,
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

    /// Single click: replace the focused head with this one.
    pub fn graph_replaced(&self) -> Option<&gantz_ca::Head> {
        self.graph_select.as_ref().and_then(|g| g.replaced.as_ref())
    }

    /// Open this head as a new tab.
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
}

impl<'a, Env> Gantz<'a, Env>
where
    Env: widget::graph_select::GraphRegistry + NodeTypeRegistry,
    Env::Node: gantz_core::Node<Env> + NodeUi<Env> + graph_scene::ToGraphMut<Node = Env::Node>,
{
    /// Instantiate the full top-level gantz widget.
    ///
    /// The head CA should match the `root`'s CA.
    pub fn new(
        env: &'a mut Env,
        heads: &'a mut [(
            gantz_ca::Head,
            &'a mut gantz_core::node::graph::Graph<Env::Node>,
        )],
    ) -> Self {
        Self {
            env,
            heads,
            log_source: None,
        }
    }

    /// Enable the logging window with a basic env logger.
    pub fn logger(mut self, logger: widget::log_view::Logger) -> Self {
        self.log_source = Some(LogSource::Logger(logger));
        self
    }

    /// Enable the logging window for tracking tracing.
    pub fn trace_capture(mut self, trace_capture: widget::trace_view::TraceCapture) -> Self {
        self.log_source = Some(LogSource::TraceCapture(trace_capture));
        self
    }

    /// Present the gantz UI.
    ///
    /// `compiled_modules` and `vms` should have the same length as `heads`,
    /// with each index corresponding to the same head.
    pub fn show(
        mut self,
        state: &mut GantzState,
        compiled_modules: &[String],
        vms: &mut [Engine],
        ui: &mut egui::Ui,
    ) -> GantzResponse {
        // TODO: Load the tiling tree, or initialise.
        let tree_id = egui::Id::new("gantz-tiles-tree-storage");

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

        // Initialise the response.
        // We'll collect it during traversal of the tree of tiles.
        let mut response = GantzResponse::default();

        // The context for traversing the tree of tiles.
        let mut behaviour = TreeBehaviour {
            gantz: &mut self,
            state: &mut *state,
            compiled_modules,
            vms,
            gantz_response: &mut response,
        };
        tree.ui(&mut behaviour, ui);

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
            focused_head: 0,
            auto_layout: false,
            center_view: false,
            command_palette: widget::CommandPalette::default(),
            layout_flow: Self::DEFAULT_DIRECTION,
            view_toggles: ViewToggles::default(),
        }
    }
}

impl<'a, 'g, Env> egui_tiles::Behavior<Pane> for TreeBehaviour<'a, 'g, Env>
where
    Env: widget::graph_select::GraphRegistry + NodeTypeRegistry,
    Env::Node: gantz_core::Node<Env> + NodeUi<Env> + graph_scene::ToGraphMut<Node = Env::Node>,
{
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        match pane {
            Pane::GraphConfig => "Graph Config".into(),
            Pane::GraphScene => "Graphs".into(),
            Pane::GraphSelect => "Graph Select".into(),
            Pane::Logs => match self.gantz.log_source {
                None => "Logs (No Source)".into(),
                Some(LogSource::Logger(_)) => "Logs".into(),
                Some(LogSource::TraceCapture(_)) => "Tracing".into(),
            },
            Pane::NodeInspector => match self.gantz.heads.get(self.state.focused_head) {
                Some((head, _)) => format!("Node Inspector - {head}").into(),
                None => "Node Inspector".into(),
            },
            Pane::Steel => "Steel".into(),
        }
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
            compiled_modules,
            ref mut vms,
            ref mut gantz_response,
        } = *self;
        match pane {
            Pane::GraphConfig => {
                graph_config(state, ui);
            }
            Pane::GraphScene => {
                // We'll use this for positioning the floating toggle window.
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

                // Sync the graph tree panes with the heads vec.
                sync_graph_panes(&mut graph_tree, gantz.heads);

                // Activate the tab corresponding to the focused head.
                if let Some((focused_head, _)) = gantz.heads.get(state.focused_head) {
                    graph_tree.make_active(|_, tile| match tile {
                        egui_tiles::Tile::Pane(GraphPane(head)) => head == focused_head,
                        _ => false,
                    });
                }

                // Render the inner tree.
                let mut graph_behaviour = GraphTreeBehaviour {
                    env: gantz.env,
                    heads: gantz.heads,
                    state,
                    vms,
                    closed_heads: &mut gantz_response.closed_heads,
                };
                graph_tree.ui(&mut graph_behaviour, ui);

                // Persist the inner tree.
                ui.memory_mut(|m| m.data.insert_persisted(graph_tree_id, Some(graph_tree)));

                // Floating toggles over the bottom right corner of the graph scene pane.
                let space = ui.style().interaction.interact_radius * 3.0;
                egui::Window::new("view_toggle_window")
                    .pivot(egui::Align2::RIGHT_BOTTOM)
                    .fixed_pos(rect.right_bottom() + egui::vec2(-space, -space))
                    .title_bar(false)
                    .resizable(false)
                    .collapsible(false)
                    .frame(egui::Frame::NONE)
                    .show(ui.ctx(), |ui| {
                        fn toggle<'a>(s: &str, b: &'a mut bool) -> widget::LabelToggle<'a> {
                            let text = egui::RichText::new(s).size(24.0);
                            widget::LabelToggle::new(text, b)
                        }
                        let grid_w = 150.0;
                        let n_cols = 5;
                        let gap_space = ui.spacing().item_spacing.x * (n_cols as f32 - 1.0);
                        let col_w = (grid_w - gap_space) / n_cols as f32;
                        egui::Grid::new("view_toggles")
                            .min_col_width(col_w)
                            .max_col_width(col_w)
                            .show(ui, |ui| {
                                ui.vertical_centered_justified(|ui| {
                                    ui.add(toggle("C", &mut state.view_toggles.graph_config))
                                        .on_hover_text("Graph Configuration");
                                });
                                ui.vertical_centered_justified(|ui| {
                                    ui.add(toggle("G", &mut state.view_toggles.graph_select))
                                        .on_hover_text("Graph Select");
                                });
                                ui.vertical_centered_justified(|ui| {
                                    ui.add(toggle("N", &mut state.view_toggles.node_inspector))
                                        .on_hover_text("Node Inspector");
                                });
                                ui.vertical_centered_justified(|ui| {
                                    ui.add(toggle("L", &mut state.view_toggles.logs))
                                        .on_hover_text("Log View");
                                });
                                ui.vertical_centered_justified(|ui| {
                                    ui.add(toggle("λ", &mut state.view_toggles.steel))
                                        .on_hover_text("Steel View");
                                });
                            });
                    });
            }
            Pane::GraphSelect => {
                let heads: Vec<_> = gantz.heads.iter().map(|(h, _)| h.clone()).collect();
                let res = graph_select(gantz.env, &heads, state.focused_head, ui);
                gantz_response.graph_select = Some(res.inner);
            }
            Pane::Logs => match &gantz.log_source {
                None => (),
                Some(LogSource::Logger(logger)) => {
                    log_view(logger, ui);
                }
                Some(LogSource::TraceCapture(trace_capture)) => {
                    trace_view(trace_capture, ui);
                }
            },
            Pane::NodeInspector => {
                // Use the focused head for the node inspector.
                let focused = state.focused_head;
                if let Some((head, root)) = gantz.heads.get_mut(focused) {
                    if let Some(vm) = vms.get_mut(focused) {
                        let head_state = state.open_heads.entry(head.clone()).or_default();
                        node_inspector(gantz.env, root, vm, head_state, ui);
                    }
                }
            }
            Pane::Steel => {
                // Use the focused head's compiled module.
                let focused = state.focused_head;
                let compiled_steel = compiled_modules.get(focused).map(|s| &s[..]).unwrap_or("");
                steel_view(compiled_steel, ui);
            }
        }
        egui_tiles::UiResponse::None
    }
}

/// The context passed to the inner graph `egui_tiles::Tree` widget.
struct GraphTreeBehaviour<'a, 'g, Env>
where
    Env: NodeTypeRegistry,
{
    env: &'a mut Env,
    heads: &'a mut [(
        gantz_ca::Head,
        &'g mut gantz_core::node::graph::Graph<Env::Node>,
    )],
    state: &'a mut GantzState,
    /// Per-head VMs, indexed to match heads.
    vms: &'a mut [Engine],
    /// Heads closed via the tab close button.
    closed_heads: &'a mut Vec<gantz_ca::Head>,
}

impl<'a, 'g, Env> egui_tiles::Behavior<GraphPane> for GraphTreeBehaviour<'a, 'g, Env>
where
    Env: widget::graph_select::GraphRegistry + NodeTypeRegistry,
    Env::Node: gantz_core::Node<Env> + NodeUi<Env> + graph_scene::ToGraphMut<Node = Env::Node>,
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
        self.heads.len() > 1
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
        // Render the tab using our custom widget.
        // Append a filled circle if this head is focused.
        let mut title = self.tab_title_for_tile(tiles, tile_id).text().to_string();
        if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
            let heads = self.heads.iter().map(|(h, _)| h);
            if crate::head_is_focused(heads, self.state.focused_head, head) {
                title.push_str(" ⚫");
            }
        }
        let res = widget::GraphTab::new(title, id)
            .active(state.active)
            .closable(state.closable)
            .show(ui);

        // Update focused_head when this tab is clicked.
        if res.tab.clicked() {
            if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
                if let Some(ix) = self.heads.iter().position(|(h, _)| h == head) {
                    self.state.focused_head = ix;
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
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut GraphPane,
    ) -> egui_tiles::UiResponse {
        let GraphPane(pane_head) = pane;
        // Find the index of this head in the heads slice.
        let ix = self
            .heads
            .iter()
            .position(|(h, _)| h == pane_head)
            .expect("pane head not found in heads");
        let (head, root) = &mut self.heads[ix];
        let vm = &mut self.vms[ix];

        // Destructure state to allow separate borrows.
        let GantzState {
            open_heads,
            focused_head,
            command_palette,
            auto_layout,
            layout_flow,
            center_view,
            ..
        } = &mut *self.state;

        let head_state = open_heads.entry(head.clone()).or_default();
        let graph_response = graph_scene(
            self.env,
            root,
            head,
            head_state,
            *auto_layout,
            *layout_flow,
            *center_view,
            vm,
            ui,
        );

        // Focus this head when clicking on the graph or any of its nodes.
        if let Some(response) = graph_response {
            if response.scene.clicked() || response.any_node_interacted() {
                *focused_head = ix;
            }
        }

        // Re-borrow head_state and vm after graph_scene.
        let head_state = open_heads.get_mut(head).unwrap();
        let vm = self.vms.get_mut(ix).unwrap();
        crate::widget::gantz::command_palette(self.env, root, head_state, command_palette, vm, ui);

        egui_tiles::UiResponse::None
    }
}

impl<'a, Env> widget::command_palette::Command for NodeTyCmd<'a, Env>
where
    Env: NodeTypeRegistry,
{
    fn text(&self) -> &str {
        self.name
    }

    fn tooltip(&self) -> &str {
        self.env.command_tooltip(self.name)
    }

    fn formatted_kb_shortcut(&self, ctx: &egui::Context) -> Option<String> {
        self.env.command_formatted_kb_shortcut(ctx, self.name)
    }
}

impl<'a, T> Clone for NodeTyCmd<'a, T> {
    fn clone(&self) -> Self {
        let Self { env, name } = self;
        Self { env, name }
    }
}

impl<'a, T> Copy for NodeTyCmd<'a, T> {}

impl Default for GantzState {
    fn default() -> Self {
        Self::new()
    }
}

/// Create the initial layout of the tree of tiles.
///
/// Roughly something like this:
///
/// -------------------------------------
/// |conf |scene                        |
/// |-----|                             |
/// |sel  |                             |
/// |     |                             |
/// |-----|-----------------------------|
/// |insp |logs          |steel         |
/// |     |              |              |
/// -------------------------------------
fn create_tree() -> egui_tiles::Tree<Pane> {
    let mut tiles = egui_tiles::Tiles::default();

    // The leaf panes.
    let graph_config = tiles.insert_pane(Pane::GraphConfig);
    let graph_scene = tiles.insert_pane(Pane::GraphScene);
    let graph_select = tiles.insert_pane(Pane::GraphSelect);
    let logs = tiles.insert_pane(Pane::Logs);
    let node_inspector = tiles.insert_pane(Pane::NodeInspector);
    let steel = tiles.insert_pane(Pane::Steel);

    // The left column.
    let mut shares = egui_tiles::Shares::default();
    shares.set_share(graph_config, 0.15);
    shares.set_share(graph_select, 0.4);
    shares.set_share(node_inspector, 0.45);
    let left_column = tiles.insert_container(egui_tiles::Linear {
        children: vec![graph_config, graph_select, node_inspector],
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

    // The root with both columns.
    let root = tiles.insert_container(egui_tiles::Linear::new_binary(
        egui_tiles::LinearDir::Horizontal,
        [left_column, right_column],
        0.22,
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
fn sync_graph_panes<N>(
    tree: &mut egui_tiles::Tree<GraphPane>,
    heads: &[(gantz_ca::Head, &mut gantz_core::node::graph::Graph<N>)],
) {
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
    let current: HashSet<gantz_ca::Head> = heads.iter().map(|(h, _)| h.clone()).collect();

    // Add missing panes for heads that don't have a pane yet.
    for (head, _) in heads {
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

/// Ensure the view toggles match the pane visibility.
fn set_tile_visibility(tree: &mut egui_tiles::Tree<Pane>, view: &ViewToggles) {
    let ids: Vec<_> = tree.tiles.tile_ids().collect();
    // Set visibility for panes.
    for &id in &ids {
        if let Some(pane) = tree.tiles.get_pane(&id) {
            match pane {
                Pane::GraphConfig => tree.set_visible(id, view.graph_config),
                Pane::GraphScene => (),
                Pane::GraphSelect => tree.set_visible(id, view.graph_select),
                Pane::Logs => tree.set_visible(id, view.logs),
                Pane::NodeInspector => tree.set_visible(id, view.node_inspector),
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

/// Provides a consistent frame and styling for the panes.
fn pane_ui<R>(ui: &mut egui::Ui, pane: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::CentralPanel::default().show_inside(ui, |ui| pane(ui))
}

fn graph_select<Env>(
    env: &mut Env,
    heads: &[gantz_ca::Head],
    focused_head: usize,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<widget::graph_select::GraphSelectResponse>
where
    Env: widget::graph_select::GraphRegistry,
{
    pane_ui(ui, |ui| {
        widget::GraphSelect::new(env, heads)
            .focused_head(focused_head)
            .show(ui)
    })
}

/// Returns the response from the graph scene if it was shown.
fn graph_scene<Env, N>(
    env: &Env,
    graph: &mut gantz_core::node::graph::Graph<N>,
    head: &gantz_ca::Head,
    head_state: &mut OpenHeadState,
    auto_layout: bool,
    layout_flow: egui::Direction,
    center_view: bool,
    vm: &mut Engine,
    ui: &mut egui::Ui,
) -> Option<graph_scene::GraphSceneResponse>
where
    N: Node<Env> + NodeUi<Env> + graph_scene::ToGraphMut<Node = N>,
{
    // We'll use this for positioning the fixed path labels window.
    let rect = ui.available_rect_before_wrap();

    // Show the `GraphScene` for the graph at the current path.
    let response = match graph_scene::index_path_graph_mut(graph, &head_state.path) {
        None => {
            log::error!("path {:?} is not a graph", head_state.path);
            None
        }
        Some(graph) => {
            // Retrieve the view associated with this graph.
            let graph_state = head_state
                .graphs
                .entry(head_state.path.to_vec())
                .or_insert_with(|| {
                    let mut view = egui_graph::View::default();
                    view.layout = widget::graph_scene::layout(graph, layout_flow, ui.ctx());
                    GraphState { view }
                });

            // Use both head and path for a unique ID per graph pane.
            let id = egui::Id::new(head).with(&head_state.path);
            let response = GraphScene::new(env, graph, &head_state.path)
                .with_id(id)
                .auto_layout(auto_layout)
                .layout_flow(layout_flow)
                .center_view(center_view)
                .show(&mut graph_state.view, &mut head_state.scene, vm, ui);
            Some(response)
        }
    };

    // Floating path index labels over the bottom-left corner of the scene.
    let space = ui.style().interaction.interact_radius * 3.0;
    egui::Window::new("path_label_window")
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(rect.left_bottom() + egui::vec2(space, -space))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .frame(egui::Frame::NONE)
        .show(ui.ctx(), |ui| {
            fn button<'a>(s: &str) -> widget::LabelButton {
                let text = egui::RichText::new(s).size(24.0);
                widget::LabelButton::new(text)
            }
            let col_w = ui.style().interaction.interact_radius * 4.0;
            egui::Grid::new("path_labels")
                .min_col_width(col_w)
                .max_col_width(col_w)
                .show(ui, |ui| {
                    ui.vertical_centered_justified(|ui| {
                        if ui.add(button("R")).on_hover_text("Root Graph").clicked() {
                            head_state.scene.cmds.push(Cmd::OpenGraph(vec![]));
                            head_state.scene.interaction.selection.clear();
                        }
                    });
                    for ix in 0..head_state.path.len() {
                        let id = head_state.path[ix];
                        ui.vertical_centered_justified(|ui| {
                            let s = format!("{}", id);
                            let path = &head_state.path[..ix + 1];
                            let current_path = path == head_state.path;
                            if ui
                                .add(button(&s))
                                .on_hover_text(format!("Graph at {path:?}"))
                                .clicked()
                            {
                                if !current_path {
                                    head_state.scene.cmds.push(Cmd::OpenGraph(path.to_vec()));
                                    head_state.scene.interaction.selection.clear();
                                }
                            }
                        });
                    }
                })
        });

    response
}

fn command_palette<Env>(
    env: &Env,
    root: &mut gantz_core::node::graph::Graph<Env::Node>,
    head_state: &mut OpenHeadState,
    cmd_palette: &mut widget::CommandPalette,
    vm: &mut Engine,
    ui: &mut egui::Ui,
) where
    Env: NodeTypeRegistry,
    Env::Node: gantz_core::Node<Env> + ToGraphMut<Node = Env::Node>,
{
    // If space is pressed, toggle command palette visibility.
    if !ui.ctx().wants_keyboard_input() {
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) {
            cmd_palette.toggle();
        }
    }

    // Map the node types to commands for the command palette.
    let cmds = env.node_types().map(|k| NodeTyCmd { env, name: &k[..] });

    // We'll only want to apply commands to the currently selected graph.
    let graph = graph_scene::index_path_graph_mut(root, &head_state.path).unwrap();

    // If a command was emitted, add the node.
    if let Some(cmd) = cmd_palette.show(ui.ctx(), cmds) {
        // Add a node of the selected type.
        let Some(node) = env.new_node(cmd.name) else {
            return;
        };
        let id = graph.add_node(node);
        let ix = id.index();

        // Determine the node's path and register it within the VM.
        let node_path: Vec<_> = head_state.path.iter().copied().chain(Some(ix)).collect();
        graph[id].register(&node_path, vm);
    }
}

fn graph_config(state: &mut GantzState, ui: &mut egui::Ui) -> egui::InnerResponse<egui::Response> {
    pane_ui(ui, |ui| {
        ui.horizontal(|ui| {
            ui.checkbox(&mut state.auto_layout, "Automatic Layout");
        });
        ui.checkbox(&mut state.center_view, "Center View");
        ui.horizontal(|ui| {
            ui.label("Flow:");
            ui.radio_value(
                &mut state.layout_flow,
                egui::Direction::LeftToRight,
                "Right",
            );
            ui.radio_value(&mut state.layout_flow, egui::Direction::TopDown, "Down");
        });
        ui.response()
    })
}

fn log_view(logger: &widget::log_view::Logger, ui: &mut egui::Ui) -> egui::InnerResponse<()> {
    pane_ui(ui, |ui| {
        widget::log_view::LogView::new("log-view".into(), logger.clone()).show(ui);
    })
}

fn trace_view(
    trace_capture: &widget::trace_view::TraceCapture,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<()> {
    pane_ui(ui, |ui| {
        widget::trace_view::TraceView::new("trace-view".into(), trace_capture.clone()).show(ui);
    })
}

fn node_inspector<Env, N>(
    env: &Env,
    root: &mut gantz_core::node::graph::Graph<N>,
    vm: &mut Engine,
    head_state: &mut OpenHeadState,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<()>
where
    N: Node<Env> + NodeUi<Env> + ToGraphMut<Node = N>,
{
    pane_ui(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink(egui::Vec2b::FALSE)
            .show(ui, |ui| {
                let graph = graph_scene::index_path_graph_mut(root, &head_state.path).unwrap();
                let ids: Vec<_> = graph.node_references().map(|n_ref| n_ref.id()).collect();
                // Collect the inlets and outlets.
                let (inlets, outlets) = crate::inlet_outlet_ids::<Env, _>(graph);
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
                        let path: Vec<_> =
                            head_state.path.iter().copied().chain(Some(ix)).collect();
                        let ctx = NodeCtx::new(
                            env,
                            &path[..],
                            &inlets,
                            &outlets,
                            vm,
                            &mut head_state.scene.cmds,
                        );
                        widget::NodeInspector::new(node, ctx).show(ui);
                    });
                }
            });
    })
}

fn steel_view(compiled_steel: &str, ui: &mut egui::Ui) -> egui::InnerResponse<()> {
    pane_ui(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink(egui::Vec2b::FALSE)
            .show(ui, |ui| {
                widget::steel_view(ui, &compiled_steel);
            });
    })
}
