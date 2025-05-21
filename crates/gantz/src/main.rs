use eframe::egui;
use egui_graph::node::{EdgeEvent, SocketKind};
use gantz::{
    Node,
    core::{Edge, Node as CoreNode, steel::steel_vm::engine::Engine},
};
use petgraph::visit::EdgeRef;
use petgraph::{
    graph::{EdgeIndex, NodeIndex},
    visit::{IntoEdgeReferences, IntoNodeReferences, NodeRef},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::{Hash, Hasher},
};
use steel::{SteelVal, parser::ast::ExprKind};

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions::default();
    let name = "g a n t z";
    eframe::run_native(name, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

struct App {
    state: State,
    view: egui_graph::View,
}

struct State {
    graph: Graph,
    graph_hash: u64,
    module: Vec<ExprKind>,
    logger: gantz::egui::widget::log_view::Logger,
    node_ty_reg: NodeTypeRegistry,
    vm: Engine,
    cmds: Vec<gantz::egui::Cmd>,
    interaction: Interaction,
    flow: egui::Direction,
    auto_layout: bool,
    node_id_map: HashMap<egui::Id, NodeIndex>,
    center_view: bool,
    views: Views,
    command_palette: gantz::egui::widget::command_palette::CommandPalette,
}

#[derive(Default)]
struct Views {
    node_inspector: bool,
    logs: bool,
    steel: bool,
    graph_config: bool,
}

#[derive(Default)]
struct Interaction {
    selection: Selection,
    edge_in_progress: Option<(NodeIndex, SocketKind, usize)>,
}

#[derive(Default)]
struct Selection {
    nodes: HashSet<NodeIndex>,
    edges: HashSet<EdgeIndex>,
}

type Graph = petgraph::stable_graph::StableGraph<Box<dyn Node>, Edge>;

/// The set of all known node types accessible to gantz.
#[derive(Default)]
pub struct NodeTypeRegistry(BTreeMap<String, Box<dyn Fn() -> Box<dyn Node>>>);

impl std::ops::Deref for NodeTypeRegistry {
    type Target = BTreeMap<String, Box<dyn Fn() -> Box<dyn Node>>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for NodeTypeRegistry {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl NodeTypeRegistry {
    /// A convenience generic method around `NodeTypeRegistry::insert`.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        new: impl 'static + Fn() -> Box<dyn Node>,
    ) -> Option<Box<dyn Fn() -> Box<dyn Node>>> {
        self.insert(name.into(), Box::new(new) as Box<_>)
    }
}

/// The set of all known node types accessible to gantz.
pub fn node_type_registry() -> NodeTypeRegistry {
    let mut reg = NodeTypeRegistry::default();
    reg.register("add", || Box::new(gantz_std::ops::Add::default()) as Box<_>);
    reg.register("bang", || Box::new(gantz_std::Bang::default()) as Box<_>);
    reg.register("expr", || {
        Box::new(gantz_core::node::Expr::new("()").unwrap()) as Box<_>
    });
    reg.register("log", || Box::new(gantz_std::Log::default()) as Box<_>);
    reg.register("number", || {
        Box::new(gantz_std::Number::default()) as Box<_>
    });
    reg
}

#[derive(Clone, Copy)]
struct Cmd<'a>(&'a str);

impl<'a> gantz::egui::widget::command_palette::Command for Cmd<'a> {
    fn text(&self) -> &str {
        self.0
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let logger = gantz::egui::widget::log_view::setup_logging();
        let ctx = &cc.egui_ctx;
        ctx.set_fonts(egui::FontDefinitions::default());

        let graph = new_graph();
        let graph_hash = graph_hash(&graph);

        let mut vm = Engine::new();
        vm.register_value(gantz::core::ROOT_STATE, SteelVal::empty_hashmap());
        gantz::core::node::state::register_graph(&graph, &mut vm);

        let module = compile_graph(&graph, &mut vm);

        let state = State {
            graph,
            graph_hash,
            module,
            logger,
            node_ty_reg: node_type_registry(),
            vm,
            cmds: vec![],
            interaction: Default::default(),
            flow: egui::Direction::TopDown,
            auto_layout: false,
            node_id_map: Default::default(),
            center_view: false,
            views: Views::default(),
            command_palette: gantz::egui::widget::command_palette::CommandPalette::default(),
        };

        let mut view = egui_graph::View::default();
        view.layout = layout(&state.graph, state.flow, ctx);

        App { view, state }
    }
}

fn compile_graph(graph: &Graph, vm: &mut Engine) -> Vec<ExprKind> {
    // Generate the steel module.
    let module = gantz::core::codegen::module(graph, &[], &[]);
    // Compile the eval fns.
    for expr in &module {
        if let Err(e) = vm.run(expr.to_pretty(80)) {
            log::error!("{e}");
        }
    }
    module
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.state.auto_layout {
            self.view.layout = layout(&self.state.graph, self.state.flow, ctx);
        }
        gui(ctx, &mut self.view, &mut self.state);

        // Check for changes to the graph.
        let new_graph_hash = graph_hash(&self.state.graph);
        if self.state.graph_hash != new_graph_hash {
            self.state.graph_hash = new_graph_hash;
            self.state.module = compile_graph(&self.state.graph, &mut self.state.vm);
        }

        // Process any pending commands.
        for cmd in self.state.cmds.drain(..) {
            match cmd {
                gantz::egui::Cmd::PushEval(path) => {
                    let fn_name = gantz::core::codegen::push_eval_fn_name(path[0]);
                    if let Err(e) = self
                        .state
                        .vm
                        .call_function_by_name_with_args(&fn_name, vec![])
                    {
                        log::error!("{e}");
                    }
                }
                gantz::egui::Cmd::PullEval(path) => {
                    let fn_name = gantz::core::codegen::pull_eval_fn_name(path[0]);
                    if let Err(e) = self
                        .state
                        .vm
                        .call_function_by_name_with_args(&fn_name, vec![])
                    {
                        log::error!("{e}");
                    }
                }
            }
        }
    }
}

fn new_graph() -> Graph {
    let mut graph = Graph::new();

    let button = graph.add_node(Box::new(gantz::std::Bang::default()) as Box<dyn gantz::Node>);
    let n0 = graph.add_node(Box::new(gantz::std::Number::default()) as Box<_>);
    let n1 = graph.add_node(Box::new(gantz::std::Number::default()) as Box<_>);
    let add = graph.add_node(Box::new(gantz::std::ops::Add::default()) as Box<_>);
    let n2 = graph.add_node(Box::new(gantz::std::Number::default()) as Box<_>);
    let log = graph.add_node(Box::new(gantz::std::log::Log::default()) as Box<_>);

    graph.add_edge(button, n0, Edge::from((0, 0)));
    graph.add_edge(button, n1, Edge::from((0, 0)));
    graph.add_edge(n0, add, Edge::from((0, 0)));
    graph.add_edge(n1, add, Edge::from((0, 1)));
    graph.add_edge(add, n2, Edge::from((0, 0)));
    graph.add_edge(n2, log, Edge::from((0, 0)));

    graph
}

/// Determine the graph hash. Used between updates to check for changes.
fn graph_hash(g: &Graph) -> u64 {
    let mut h = std::hash::DefaultHasher::default();
    for n in g.node_references() {
        n.id().hash(&mut h);
        n.weight().hash(&mut h);
    }
    for e in g.edge_references() {
        e.id().hash(&mut h);
        e.weight().hash(&mut h);
    }
    h.finish()
}

fn layout(graph: &Graph, flow: egui::Direction, ctx: &egui::Context) -> egui_graph::Layout {
    ctx.memory(|m| {
        let nodes = graph.node_indices().map(|n| {
            let id = egui::Id::new(n);
            let size = m
                .area_rect(id)
                .map(|a| a.size())
                .unwrap_or([200.0, 50.0].into());
            (id, size)
        });
        let edges = graph
            .edge_indices()
            .filter_map(|e| graph.edge_endpoints(e))
            .map(|(a, b)| (egui::Id::new(a), egui::Id::new(b)));
        egui_graph::layout(nodes, edges, flow)
    })
}

fn gui(ctx: &egui::Context, view: &mut egui_graph::View, state: &mut State) {
    egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctx, |ui| {
            graph(ui, view, state);
            command_palette(ui, state);
            if state.views.graph_config {
                graph_config(ui, view, state);
            }
            if state.views.steel {
                steel_view(ui, state);
            }
            if state.views.logs {
                log_view(ui, state);
            }
            if state.views.node_inspector {
                node_inspector(ui, state);
            }
        });
}

fn command_palette(ui: &mut egui::Ui, state: &mut State) {
    if !ui.ctx().wants_keyboard_input() {
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) {
            state.command_palette.toggle();
        }
    }
    let cmds = state.node_ty_reg.keys().map(|k| Cmd(&k[..]));
    if let Some(Cmd(node_ty)) = state.command_palette.show(ui.ctx(), cmds) {
        // Add a node of the selected type.
        let new_fn = &state.node_ty_reg[node_ty];
        let node = (new_fn)();
        let id = state.graph.add_node(node);
        let ix = id.index();
        state.graph[id].register(&[ix], &mut state.vm);
    }
}

fn graph(ui: &mut egui::Ui, view: &mut egui_graph::View, state: &mut State) {
    egui_graph::Graph::new("gantz")
        .center_view(state.center_view)
        .show(view, ui, |ui, show| {
            show.nodes(ui, |nctx, ui| nodes(nctx, ui, state))
                .edges(ui, |ectx, ui| edges(ectx, ui, state));
        });

    // FIXME: This should be floating on top of the Graph widget.
    let space = ui.style().interaction.interact_radius * 3.0;
    egui::Window::new("label_toggle_window")
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-space, -space))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .frame(egui::Frame::NONE)
        .show(ui.ctx(), |ui| {
            fn toggle<'a>(s: &str, b: &'a mut bool) -> gantz::egui::widget::LabelToggle<'a> {
                let text = egui::RichText::new(s).size(24.0);
                gantz::egui::widget::LabelToggle::new(text, b)
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

fn nodes(nctx: &mut egui_graph::NodesCtx, ui: &mut egui::Ui, state: &mut State) {
    let indices: Vec<_> = state.graph.node_indices().collect();
    for n in indices {
        let inputs = state.graph[n].n_inputs();
        let outputs = state.graph[n].n_outputs();
        let node = &mut state.graph[n];
        let egui_id = egui::Id::new(n);
        state.node_id_map.insert(egui_id, n);
        let mut node_changed = false;
        let response = egui_graph::node::Node::from_id(egui_id)
            .inputs(inputs)
            .outputs(outputs)
            .flow(state.flow)
            .show(nctx, ui, |ui| {
                let path = vec![n.index()];
                let node_ctx = gantz::egui::NodeCtx::new(&path, &mut state.vm, &mut state.cmds);

                // Instantiate the node's UI.
                node_changed |= node.ui(node_ctx, ui).changed();
            });

        if response.changed() {
            // Update the selected nodes.
            if egui_graph::is_node_selected(ui, nctx.graph_id, egui_id) {
                state.interaction.selection.nodes.insert(n);
            } else {
                state.interaction.selection.nodes.remove(&n);
            }

            // Check for an edge event.
            if let Some(ev) = response.edge_event() {
                match ev {
                    EdgeEvent::Started { kind, index } => {
                        state.interaction.edge_in_progress = Some((n, kind, index));
                    }
                    EdgeEvent::Ended { kind, index } => {
                        // Create the edge.
                        if let Some((src, _, ix)) = state.interaction.edge_in_progress.take() {
                            let (index, ix) = (index as u16, ix as u16);
                            let (a, b, w) = match kind {
                                SocketKind::Input => (src, n, Edge::from((ix, index))),
                                SocketKind::Output => (n, src, Edge::from((index, ix))),
                            };
                            // Check that this edge doesn't already exist.
                            if !state
                                .graph
                                .edges(a)
                                .any(|e| e.target() == b && *e.weight() == w)
                            {
                                state.graph.add_edge(a, b, w);
                            }
                        }
                    }
                    EdgeEvent::Cancelled => {
                        state.interaction.edge_in_progress = None;
                    }
                }
            }

            // If the delete key was pressed while selected, remove it.
            if response.removed() {
                state.graph.remove_node(n);
                state.node_id_map.remove(&egui_id);
            }
        }
    }
}

fn edges(ectx: &mut egui_graph::EdgesCtx, ui: &mut egui::Ui, state: &mut State) {
    // Instantiate all edges.
    for e in state.graph.edge_indices().collect::<Vec<_>>() {
        let (na, nb) = state.graph.edge_endpoints(e).unwrap();
        let edge = *state.graph.edge_weight(e).unwrap();
        let (input, output) = (edge.input.0.into(), edge.output.0.into());
        let a = egui::Id::new(na);
        let b = egui::Id::new(nb);
        let mut selected = state.interaction.selection.edges.contains(&e);
        let response =
            egui_graph::edge::Edge::new((a, output), (b, input), &mut selected).show(ectx, ui);

        if response.deleted() {
            state.graph.remove_edge(e);
            state.interaction.selection.edges.remove(&e);
        } else if response.changed() {
            if selected {
                state.interaction.selection.edges.insert(e);
            } else {
                state.interaction.selection.edges.remove(&e);
            }
        }
    }

    // Draw the in-progress edge if there is one.
    if let Some(edge) = ectx.in_progress(ui) {
        edge.show(ui);
    }
}

fn node_inspector(ui: &mut egui::Ui, state: &mut State) {
    // In your egui update loop:
    egui::Window::new("Node Inspector").show(ui.ctx(), |ui| {
        let mut ids = state
            .interaction
            .selection
            .nodes
            .iter()
            .copied()
            .collect::<Vec<_>>();
        ids.sort();
        for id in ids {
            ui.group(|ui| {
                let node = &mut state.graph[id];
                let path = &[id.index()];
                let ctx = gantz::egui::NodeCtx::new(&path[..], &mut state.vm, &mut state.cmds);
                gantz::egui::widget::NodeInspector::new(node, ctx).show(ui);
            });
        }
    });
}

fn log_view(ui: &mut egui::Ui, state: &State) {
    // In your egui update loop:
    egui::Window::new("Logs").show(ui.ctx(), |ui| {
        gantz::egui::widget::log_view::LogView::new("log-view".into(), state.logger.clone())
            .show(ui);
    });
}

fn steel_view(ui: &mut egui::Ui, state: &mut State) {
    egui::Window::new("Module").show(ui.ctx(), |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let s = state
                .module
                .iter()
                .map(|expr| expr.to_pretty(80))
                .collect::<Vec<String>>()
                .join("\n\n");
            gantz::egui::widget::steel_view(ui, &s);
        });
    });
}

fn graph_config(ui: &mut egui::Ui, view: &mut egui_graph::View, state: &mut State) {
    egui::Window::new("Graph Config")
        .auto_sized()
        .show(ui.ctx(), |ui| {
            ui.label("GRAPH CONFIG");
            ui.horizontal(|ui| {
                ui.checkbox(&mut state.auto_layout, "Automatic Layout");
                ui.separator();
                ui.add_enabled_ui(!state.auto_layout, |ui| {
                    if ui.button("Layout Once").clicked() {
                        view.layout = layout(&state.graph, state.flow, ui.ctx());
                    }
                });
            });
            ui.checkbox(&mut state.center_view, "Center View");
            ui.horizontal(|ui| {
                ui.label("Flow:");
                ui.radio_value(&mut state.flow, egui::Direction::LeftToRight, "Right");
                ui.radio_value(&mut state.flow, egui::Direction::TopDown, "Down");
            });
            ui.label(format!("Scene: {:?}", view.scene_rect));
        });
}
