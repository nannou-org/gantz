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
    reg
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
            auto_layout: true,
            node_id_map: Default::default(),
            center_view: false,
        };
        let view = Default::default();
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
            graph_config(ui, view, state);
            log_view(ui, state);
            steel_view(ui, state);
            graph(ui, view, state);
        });
}

fn graph(ui: &mut egui::Ui, view: &mut egui_graph::View, state: &mut State) {
    egui_graph::Graph::new("gantz")
        .center_view(state.center_view)
        .show(view, ui, |ui, show| {
            show.nodes(ui, |nctx, ui| nodes(nctx, ui, state))
                .edges(ui, |ectx, ui| edges(ectx, ui, state));
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
            let s = state.module.iter().map(|expr| expr.to_pretty(80)).collect::<Vec<String>>().join("\n\n");
            gantz::egui::widget::steel_view(ui, &s);
        });
    });
}

fn graph_config(ui: &mut egui::Ui, view: &mut egui_graph::View, state: &mut State) {
    let mut frame = egui::Frame::window(ui.style());
    frame.shadow.spread = 0;
    frame.shadow.offset = [0, 0];
    egui::Window::new("Graph Config")
        .frame(frame)
        .anchor(
            egui::Align2::LEFT_TOP,
            ui.spacing().window_margin.left_top(),
        )
        .collapsible(false)
        .title_bar(false)
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
