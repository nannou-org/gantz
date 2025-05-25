use gantz_core::Node;
use steel::steel_vm::engine::Engine;

use crate::{
    NodeCtx, NodeUi,
    widget::{self, GraphScene, GraphSceneState, graph_scene},
};

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
    graph: &'a mut graph_scene::Graph<T::Node>,
}

pub struct GantzState {
    pub graph_scene: GraphSceneState,
    pub view: egui_graph::View,
    pub views: Views,
    pub command_palette: widget::CommandPalette,
    pub auto_layout: bool,
    pub layout_flow: egui::Direction,
    pub center_view: bool,
    pub logger: widget::log_view::Logger,
}

#[derive(Default)]
pub struct Views {
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
    T::Node: NodeUi,
{
    /// Instantiate the full top-level gantz widget.
    pub fn new(node_ty_reg: &'a T, graph: &'a mut graph_scene::Graph<T::Node>) -> Self {
        Self { node_ty_reg, graph }
    }

    /// Present the gantz UI.
    pub fn show(
        self,
        state: &mut GantzState,
        compiled_steel: &str,
        vm: &mut Engine,
        ui: &mut egui::Ui,
    ) {
        graph_scene(self.graph, state, vm, ui);
        command_palette(self.node_ty_reg, self.graph, state, vm, ui);
        if state.views.graph_config {
            graph_config(self.graph, state, ui);
        }
        if state.views.logs {
            log_view(&state.logger, ui);
        }
        if state.views.node_inspector {
            node_inspector(self.graph, vm, state, ui);
        }
        if state.views.steel {
            steel_view(compiled_steel, ui);
        }
    }
}

impl GantzState {
    pub const DEFAULT_DIRECTION: egui::Direction = egui::Direction::TopDown;

    /// Shorthand for initialising graph state with the initial layout
    /// automatically determined for the given graph.
    // TODO: This layout is currently estimated but doesn't actually run the UI.
    // Should consider dry-running the UI once if possible.
    pub fn new_with_layout<N>(graph: &graph_scene::Graph<N>, ctx: &egui::Context) -> Self {
        let mut view = egui_graph::View::default();
        view.layout = widget::graph_scene::layout(graph, Self::DEFAULT_DIRECTION, ctx);
        Self::from_view(view)
    }

    pub fn from_view(view: egui_graph::View) -> Self {
        let logger = widget::log_view::setup_logging();
        Self {
            auto_layout: false,
            center_view: false,
            command_palette: widget::CommandPalette::default(),
            graph_scene: Default::default(),
            layout_flow: Self::DEFAULT_DIRECTION,
            logger,
            view,
            views: Views::default(),
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
    graph: &mut graph_scene::Graph<N>,
    state: &mut GantzState,
    vm: &mut Engine,
    ui: &mut egui::Ui,
) where
    N: Node + NodeUi,
{
    GraphScene::new(graph)
        .auto_layout(state.auto_layout)
        .layout_flow(state.layout_flow)
        .center_view(state.center_view)
        .show(&mut state.view, &mut state.graph_scene, vm, ui);

    // FIXME: This should be floating on top of the Graph widget.
    let space = ui.style().interaction.interact_radius * 3.0;
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
                        ui.add(toggle("N", &mut state.views.node_inspector))
                            .on_hover_text("Node Inspector");
                    });
                    ui.vertical_centered_justified(|ui| {
                        ui.add(toggle("L", &mut state.views.logs))
                            .on_hover_text("Log View");
                    });
                    ui.vertical_centered_justified(|ui| {
                        ui.add(toggle("λ", &mut state.views.steel))
                            .on_hover_text("Steel View");
                    });
                    ui.vertical_centered_justified(|ui| {
                        ui.add(toggle("G", &mut state.views.graph_config))
                            .on_hover_text("Graph Configuration");
                    });
                });
        });
}

fn command_palette<T>(
    node_ty_reg: &T,
    graph: &mut graph_scene::Graph<T::Node>,
    state: &mut GantzState,
    vm: &mut Engine,
    ui: &mut egui::Ui,
) where
    T: NodeTypeRegistry,
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

    // If a command was emitted, add the node.
    if let Some(cmd) = state.command_palette.show(ui.ctx(), cmds) {
        // Add a node of the selected type.
        let Some(node) = node_ty_reg.new_node(cmd.name) else {
            return;
        };
        let id = graph.add_node(node);
        let ix = id.index();
        graph[id].register(&[ix], vm);
    }
}

fn graph_config<N>(graph: &mut graph_scene::Graph<N>, state: &mut GantzState, ui: &mut egui::Ui) {
    egui::Window::new("Graph Config")
        .auto_sized()
        .show(ui.ctx(), |ui| {
            ui.label("GRAPH CONFIG");
            ui.horizontal(|ui| {
                ui.checkbox(&mut state.auto_layout, "Automatic Layout");
                ui.separator();
                ui.add_enabled_ui(!state.auto_layout, |ui| {
                    if ui.button("Layout Once").clicked() {
                        state.view.layout = graph_scene::layout(graph, state.layout_flow, ui.ctx());
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
            ui.label(format!("Scene: {:?}", state.view.scene_rect));
        });
}

fn log_view(logger: &widget::log_view::Logger, ui: &mut egui::Ui) {
    // In your egui update loop:
    egui::Window::new("Logs").show(ui.ctx(), |ui| {
        widget::log_view::LogView::new("log-view".into(), logger.clone()).show(ui);
    });
}

fn node_inspector<N>(
    graph: &mut graph_scene::Graph<N>,
    vm: &mut Engine,
    state: &mut GantzState,
    ui: &mut egui::Ui,
) where
    N: Node + NodeUi,
{
    // In your egui update loop:
    egui::Window::new("Node Inspector").show(ui.ctx(), |ui| {
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
                let node = &mut graph[id];
                let path = &[id.index()];
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
