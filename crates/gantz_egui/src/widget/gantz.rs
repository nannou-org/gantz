use super::graph_scene::ToGraphMut;
use crate::{
    Cmd, NodeCtx, NodeUi,
    widget::{self, GraphScene, GraphSceneState, graph_scene},
};
use gantz_core::{Node, node};
use std::collections::HashMap;
use steel::steel_vm::engine::Engine;

/// A registry of available nodes.
pub trait NodeTypeRegistry {
    /// The gantz node type that can be produced by the registry.
    type Node: Node;

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
pub struct Gantz<'a, T>
where
    T: NodeTypeRegistry,
{
    node_ty_reg: &'a T,
    root: &'a mut gantz_core::node::GraphNode<T::Node>,
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

/// UI state relevant to each nested graph within the tree.
pub type Graphs = HashMap<Vec<node::Id>, GraphState>;

/// UI state relevant to a graph at a certain path within the root.
#[derive(serde::Deserialize, serde::Serialize)]
pub struct GraphState {
    pub view: egui_graph::View,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct ViewToggles {
    pub node_inspector: bool,
    pub logs: bool,
    pub steel: bool,
    pub graph_config: bool,
}

struct NodeTyCmd<'a, T> {
    reg: &'a T,
    name: &'a str,
}

impl<'a, T> Gantz<'a, T>
where
    T: NodeTypeRegistry,
    T::Node: NodeUi + graph_scene::ToGraphMut<Node = T::Node>,
{
    /// Instantiate the full top-level gantz widget.
    pub fn new(node_ty_reg: &'a T, root: &'a mut gantz_core::node::GraphNode<T::Node>) -> Self {
        Self { node_ty_reg, root }
    }

    /// Present the gantz UI.
    pub fn show(
        self,
        state: &mut GantzState,
        logger: &widget::log_view::Logger,
        compiled_steel: &str,
        vm: &mut Engine,
        ui: &mut egui::Ui,
    ) {
        graph_scene(self.root, state, vm, ui);
        command_palette(self.node_ty_reg, self.root, state, vm, ui);
        if state.view_toggles.graph_config {
            graph_config(self.root, state, ui);
        }
        if state.view_toggles.logs {
            log_view(logger, ui);
        }
        if state.view_toggles.node_inspector {
            node_inspector(self.root, vm, state, ui);
        }
        if state.view_toggles.steel {
            steel_view(compiled_steel, ui);
        }
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

impl<'a, T: NodeTypeRegistry> widget::command_palette::Command for NodeTyCmd<'a, T> {
    fn text(&self) -> &str {
        self.name
    }

    fn tooltip(&self) -> &str {
        self.reg.command_tooltip(self.name)
    }

    fn formatted_kb_shortcut(&self, ctx: &egui::Context) -> Option<String> {
        self.reg.command_formatted_kb_shortcut(ctx, self.name)
    }
}

impl<'a, T> Clone for NodeTyCmd<'a, T> {
    fn clone(&self) -> Self {
        let Self { reg, name } = self;
        Self { reg, name }
    }
}

impl<'a, T> Copy for NodeTyCmd<'a, T> {}

fn graph_scene<N>(
    graph: &mut gantz_core::node::GraphNode<N>,
    state: &mut GantzState,
    vm: &mut Engine,
    ui: &mut egui::Ui,
) where
    N: Node + NodeUi + graph_scene::ToGraphMut<Node = N>,
{
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

            GraphScene::new(graph, &state.path)
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
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(space, -space))
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
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-space, -space))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .frame(egui::Frame::NONE)
        .show(ui.ctx(), |ui| {
            fn toggle<'a>(s: &str, b: &'a mut bool) -> widget::LabelToggle<'a> {
                let text = egui::RichText::new(s).size(24.0);
                widget::LabelToggle::new(text, b)
            }
            let grid_w = 120.0;
            let n_cols = 4;
            let gap_space = ui.spacing().item_spacing.x * (n_cols as f32 - 1.0);
            let col_w = (grid_w - gap_space) / n_cols as f32;
            egui::Grid::new("view_toggles")
                .min_col_width(col_w)
                .max_col_width(col_w)
                .show(ui, |ui| {
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
                    ui.vertical_centered_justified(|ui| {
                        ui.add(toggle("G", &mut state.view_toggles.graph_config))
                            .on_hover_text("Graph Configuration");
                    });
                });
        });
}

fn command_palette<T>(
    node_ty_reg: &T,
    root: &mut gantz_core::node::GraphNode<T::Node>,
    state: &mut GantzState,
    vm: &mut Engine,
    ui: &mut egui::Ui,
) where
    T: NodeTypeRegistry,
    T::Node: ToGraphMut<Node = T::Node>,
{
    // If space is pressed, toggle command palette visibility.
    if !ui.ctx().wants_keyboard_input() {
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) {
            state.command_palette.toggle();
        }
    }

    // Map the node types to commands for the command palette.
    let cmds = node_ty_reg.node_types().map(|k| NodeTyCmd {
        reg: node_ty_reg,
        name: &k[..],
    });

    // We'll only want to apply commands to the currently selected graph.
    let graph = graph_scene::index_path_graph_mut(root, &state.path).unwrap();

    // If a command was emitted, add the node.
    if let Some(cmd) = state.command_palette.show(ui.ctx(), cmds) {
        // Add a node of the selected type.
        let Some(node) = node_ty_reg.new_node(cmd.name) else {
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
    root: &mut gantz_core::node::GraphNode<N>,
    state: &mut GantzState,
    ui: &mut egui::Ui,
) where
    N: ToGraphMut<Node = N>,
{
    egui::Window::new("Graph Config")
        .auto_sized()
        .show(ui.ctx(), |ui| {
            ui.label("GRAPH CONFIG");
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
            let graph_state = &state.graphs[&state.path];
            ui.label(format!("Scene: {:?}", graph_state.view.scene_rect));
        });
}

fn log_view(logger: &widget::log_view::Logger, ui: &mut egui::Ui) {
    // In your egui update loop:
    egui::Window::new("Logs").show(ui.ctx(), |ui| {
        widget::log_view::LogView::new("log-view".into(), logger.clone()).show(ui);
    });
}

fn node_inspector<N>(
    root: &mut gantz_core::node::GraphNode<N>,
    vm: &mut Engine,
    state: &mut GantzState,
    ui: &mut egui::Ui,
) where
    N: Node + NodeUi + ToGraphMut<Node = N>,
{
    // In your egui update loop:
    egui::Window::new("Node Inspector").show(ui.ctx(), |ui| {
        let graph = graph_scene::index_path_graph_mut(root, &state.path).unwrap();
        let mut ids = state
            .graph_scene
            .interaction
            .selection
            .nodes
            .iter()
            .copied()
            .collect::<Vec<_>>();
        ids.sort();
        for id in ids {
            ui.group(|ui| {
                let Some(node) = graph.node_weight_mut(id) else {
                    return;
                };
                let ix = id.index();
                let path: Vec<_> = state.path.iter().copied().chain(Some(ix)).collect();
                let ctx = NodeCtx::new(&path[..], vm, &mut state.graph_scene.cmds);
                widget::NodeInspector::new(node, ctx).show(ui);
            });
        }
    });
}

fn steel_view(compiled_steel: &str, ui: &mut egui::Ui) {
    egui::Window::new("Module").show(ui.ctx(), |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            widget::steel_view(ui, &compiled_steel);
        });
    });
}
