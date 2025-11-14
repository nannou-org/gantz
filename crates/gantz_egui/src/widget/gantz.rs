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
    root: &'a mut gantz_core::node::graph::Graph<Env::Node>,
    head: &'a gantz_ca::Head,
    log_source: Option<LogSource>,
}

enum LogSource {
    Logger(widget::log_view::Logger),
    #[cfg(feature = "tracing")]
    TraceCapture(widget::trace_view::TraceCapture),
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct GantzState {
    pub graph_scene: GraphSceneState,
    pub path: Vec<node::Id>,
    pub graphs: Graphs,
    pub view_toggles: ViewToggles,
    pub command_palette: widget::CommandPalette,
    pub auto_layout: bool,
    pub layout_flow: egui::Direction,
    pub center_view: bool,
}

/// A pane within the tree.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Pane {
    GraphConfig,
    GraphScene,
    GraphSelect,
    Logs,
    NodeInspector,
    Steel,
}

/// The context passed to the `egui_tiles::Tree` widget.
struct TreeBehaviour<'a, 'g, Env>
where
    Env: NodeTypeRegistry,
{
    gantz: &'a mut Gantz<'g, Env>,
    state: &'a mut GantzState,
    compiled_steel: &'a str,
    vm: &'a mut Engine,
    gantz_response: &'a mut GantzResponse,
}

/// Response from the top-level gantz widget.
#[derive(Debug, Default)]
pub struct GantzResponse {
    pub graph_select: Option<widget::graph_select::GraphSelectResponse>,
}

/// UI state relevant to each nested graph within the tree.
pub type Graphs = HashMap<Vec<node::Id>, GraphState>;

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

    /// If a graph was selected this is its content address and name (if named).
    pub fn graph_selected(&self) -> Option<&gantz_ca::Head> {
        self.graph_select.as_ref().and_then(|g| g.selected.as_ref())
    }

    /// If `Some` indicates, the root graph name was updated.
    ///
    /// If `Some(None)`, the head graph's name was cleared.
    pub fn graph_name_updated(&self) -> Option<Option<String>> {
        self.graph_select
            .as_ref()
            .and_then(|g| g.name_updated.clone())
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
        root: &'a mut gantz_core::node::graph::Graph<Env::Node>,
        head: &'a gantz_ca::Head,
    ) -> Self {
        Self {
            env,
            root,
            head,
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
    pub fn show(
        mut self,
        state: &mut GantzState,
        compiled_steel: &str,
        vm: &mut Engine,
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
            compiled_steel,
            vm,
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
        Self::from_graphs(Default::default())
    }

    pub fn from_graphs(graphs: Graphs) -> Self {
        Self {
            graph_scene: Default::default(),
            path: vec![],
            graphs,
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
            Pane::GraphScene => "Graph Scene".into(),
            Pane::GraphSelect => "Graph Select".into(),
            Pane::Logs => match self.gantz.log_source {
                None => "Logs (No Source)".into(),
                Some(LogSource::Logger(_)) => "Logs".into(),
                Some(LogSource::TraceCapture(_)) => "Tracing".into(),
            },
            Pane::NodeInspector => "Node Inspector".into(),
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
        // We will manually simplify before calling `tree.ui`. See `simplify_tree`
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
            compiled_steel,
            ref mut vm,
            ref mut gantz_response,
        } = *self;
        match pane {
            Pane::GraphConfig => {
                graph_config(gantz.root, state, ui);
            }
            Pane::GraphScene => {
                graph_scene(gantz.env, gantz.root, state, vm, ui);
                command_palette(gantz.env, gantz.root, state, vm, ui);
            }
            Pane::GraphSelect => {
                let res = graph_select(gantz.env, gantz.head, ui);
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
                node_inspector(gantz.env, gantz.root, vm, state, ui);
            }
            Pane::Steel => {
                steel_view(compiled_steel, ui);
            }
        }
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
    // Find it's parent. This must be `Tabs` after the `simplify` pass above.
    let parent_id = tree
        .tiles
        .parent_of(graph_scene_id)
        .expect("parent must be `Tabs`");
    // If the parent has one child, replace it with the graph scene.
    let parent = tree
        .tiles
        .get_container(parent_id)
        .expect("parent must be a container");
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
    head: &gantz_ca::Head,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<widget::graph_select::GraphSelectResponse>
where
    Env: widget::graph_select::GraphRegistry,
{
    pane_ui(ui, |ui| widget::GraphSelect::new(env, head).show(ui))
}

fn graph_scene<Env, N>(
    env: &Env,
    graph: &mut gantz_core::node::graph::Graph<N>,
    state: &mut GantzState,
    vm: &mut Engine,
    ui: &mut egui::Ui,
) where
    N: Node<Env> + NodeUi<Env> + graph_scene::ToGraphMut<Node = N>,
{
    // We'll use this for positioning the fixed path and toggle windows.
    let rect = ui.available_rect_before_wrap();

    // Show the `GraphScene` for the graph at the current path.
    match graph_scene::index_path_graph_mut(graph, &state.path) {
        None => log::error!("path {:?} is not a graph", state.path),
        Some(graph) => {
            // Retrieve the view associated with this graph.
            let graph_state = state.graphs.entry(state.path.to_vec()).or_insert_with(|| {
                let mut view = egui_graph::View::default();
                view.layout = widget::graph_scene::layout(graph, state.layout_flow, ui.ctx());
                GraphState { view }
            });

            GraphScene::new(env, graph, &state.path)
                .with_id(egui::Id::new(&state.path))
                .auto_layout(state.auto_layout)
                .layout_flow(state.layout_flow)
                .center_view(state.center_view)
                .show(&mut graph_state.view, &mut state.graph_scene, vm, ui);
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
                            state.graph_scene.cmds.push(Cmd::OpenGraph(vec![]));
                            state.graph_scene.interaction.selection.clear();
                        }
                    });
                    for ix in 0..state.path.len() {
                        let id = state.path[ix];
                        ui.vertical_centered_justified(|ui| {
                            let s = format!("{}", id);
                            let path = &state.path[..ix + 1];
                            let current_path = path == state.path;
                            if ui
                                .add(button(&s))
                                .on_hover_text(format!("Graph at {path:?}"))
                                .clicked()
                            {
                                if !current_path {
                                    state.graph_scene.cmds.push(Cmd::OpenGraph(path.to_vec()));
                                    state.graph_scene.interaction.selection.clear();
                                }
                            }
                        });
                    }
                })
        });

    // Floating toggles over the bottom right corner of the graph scene.
    egui::Window::new("label_toggle_window")
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

fn command_palette<Env>(
    env: &Env,
    root: &mut gantz_core::node::graph::Graph<Env::Node>,
    state: &mut GantzState,
    vm: &mut Engine,
    ui: &mut egui::Ui,
) where
    Env: NodeTypeRegistry,
    Env::Node: gantz_core::Node<Env> + ToGraphMut<Node = Env::Node>,
{
    // If space is pressed, toggle command palette visibility.
    if !ui.ctx().wants_keyboard_input() {
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) {
            state.command_palette.toggle();
        }
    }

    // Map the node types to commands for the command palette.
    let cmds = env.node_types().map(|k| NodeTyCmd { env, name: &k[..] });

    // We'll only want to apply commands to the currently selected graph.
    let graph = graph_scene::index_path_graph_mut(root, &state.path).unwrap();

    // If a command was emitted, add the node.
    if let Some(cmd) = state.command_palette.show(ui.ctx(), cmds) {
        // Add a node of the selected type.
        let Some(node) = env.new_node(cmd.name) else {
            return;
        };
        let id = graph.add_node(node);
        let ix = id.index();

        // Determine the node's path and register it within the VM.
        let node_path: Vec<_> = state.path.iter().copied().chain(Some(ix)).collect();
        graph[id].register(&node_path, vm);
    }
}

fn graph_config<N>(
    root: &mut gantz_core::node::graph::Graph<N>,
    state: &mut GantzState,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<egui::Response>
where
    N: ToGraphMut<Node = N>,
{
    pane_ui(ui, |ui| {
        ui.horizontal(|ui| {
            ui.checkbox(&mut state.auto_layout, "Automatic Layout");
            ui.separator();
            ui.add_enabled_ui(!state.auto_layout, |ui| {
                if ui.button("Layout Once").clicked() {
                    let graph = graph_scene::index_path_graph_mut(root, &state.path).unwrap();
                    let graph_state = state.graphs.get_mut(&state.path).unwrap();
                    graph_state.view.layout =
                        graph_scene::layout(graph, state.layout_flow, ui.ctx());
                }
            });
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
        if let Some(graph_state) = state.graphs.get(&state.path) {
            ui.label(format!("Scene: {:?}", graph_state.view.scene_rect));
        }
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
    state: &mut GantzState,
    ui: &mut egui::Ui,
) -> egui::InnerResponse<()>
where
    N: Node<Env> + NodeUi<Env> + ToGraphMut<Node = N>,
{
    pane_ui(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink(egui::Vec2b::FALSE)
            .show(ui, |ui| {
                let graph = graph_scene::index_path_graph_mut(root, &state.path).unwrap();
                let ids: Vec<_> = graph.node_references().map(|n_ref| n_ref.id()).collect();
                // Collect the inlets and outlets.
                let (inlets, outlets) = crate::inlet_outlet_ids::<Env, _>(graph);
                for id in ids {
                    let mut frame = egui::Frame::group(ui.style());
                    if state.graph_scene.interaction.selection.nodes.contains(&id) {
                        frame.stroke.color = ui.visuals().selection.stroke.color;
                    }
                    frame.show(ui, |ui| {
                        let Some(node) = graph.node_weight_mut(id) else {
                            return;
                        };
                        let ix = id.index();
                        let path: Vec<_> = state.path.iter().copied().chain(Some(ix)).collect();
                        let ctx = NodeCtx::new(
                            env,
                            &path[..],
                            &inlets,
                            &outlets,
                            vm,
                            &mut state.graph_scene.cmds,
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
