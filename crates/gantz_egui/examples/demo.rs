//! A simple demonstration of a pure `egui` setup for `gantz`.
//!
//! Includes a top-level `Node` trait with a minimal set of nodes, a node
//! registry, and a minimal default graph to demonstrate how to use these with
//! the top-level `Gantz` widget in an egui app.

use dyn_hash::DynHash;
use eframe::egui;
use gantz_core::steel::steel_vm::engine::Engine;
use petgraph::visit::EdgeRef;
use petgraph::visit::{IntoEdgeReferences, IntoNodeReferences, NodeRef};
use std::{
    any::Any,
    collections::BTreeMap,
    hash::{Hash, Hasher},
};
use steel::{SteelVal, parser::ast::ExprKind};

// ----------------------------------------------

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions::default();
    let name = "g a n t z";
    eframe::run_native(name, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

// ----------------------------------------------
// Top-level `Node` trait
// ----------------------------------------------

/// A top-level blanket trait providing trait object serialization.
#[typetag::serde(tag = "type")]
trait Node: Any + DynHash + gantz_core::Node + gantz_egui::NodeUi {}

dyn_hash::hash_trait_object!(Node);

#[typetag::serde]
impl Node for gantz_core::node::Expr {}
#[typetag::serde]
impl Node for gantz_core::node::GraphNode<Box<dyn Node>> {}
#[typetag::serde]
impl Node for gantz_core::node::graph::Inlet {}
#[typetag::serde]
impl Node for gantz_core::node::graph::Outlet {}

#[typetag::serde]
impl Node for gantz_std::ops::Add {}
#[typetag::serde]
impl Node for gantz_std::Bang {}
#[typetag::serde]
impl Node for gantz_std::Log {}
#[typetag::serde]
impl Node for gantz_std::Number {}

#[typetag::serde]
impl Node for Box<dyn Node> {}

// To allow for navigating between nested graphs in a graph scene, we need to be
// able to downcast a node to a graph node.
impl gantz_egui::widget::graph_scene::ToGraphMut for Box<dyn Node> {
    type Node = Self;
    fn to_graph_mut(&mut self) -> Option<&mut gantz_core::node::GraphNode<Self::Node>> {
        ((&mut **self) as &mut dyn Any).downcast_mut()
    }
}

// ----------------------------------------------
// Node Registry
// ----------------------------------------------

/// The set of all known node types accessible to gantz.
#[derive(Default)]
struct NodeTypeRegistry(BTreeMap<String, Box<dyn Fn() -> Box<dyn Node>>>);

impl NodeTypeRegistry {
    /// A convenience generic method around `NodeTypeRegistry::insert`.
    fn register(
        &mut self,
        name: impl Into<String>,
        new: impl 'static + Fn() -> Box<dyn Node>,
    ) -> Option<Box<dyn Fn() -> Box<dyn Node>>> {
        self.0.insert(name.into(), Box::new(new) as Box<_>)
    }
}

impl gantz_egui::widget::gantz::NodeTypeRegistry for NodeTypeRegistry {
    type Node = Box<dyn Node>;

    fn node_types(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(|s| &s[..])
    }

    fn new_node(&self, node_type: &str) -> Option<Self::Node> {
        self.0.get(node_type).map(|f| (f)())
    }
}

/// The set of all known node types accessible to gantz.
fn node_type_registry() -> NodeTypeRegistry {
    let mut reg = NodeTypeRegistry::default();
    reg.register("add", || Box::new(gantz_std::ops::Add::default()) as Box<_>);
    reg.register("bang", || Box::new(gantz_std::Bang::default()) as Box<_>);
    reg.register("expr", || {
        Box::new(gantz_core::node::Expr::new("()").unwrap()) as Box<_>
    });
    reg.register("graph", || Box::new(GraphNode::default()) as Box<_>);
    reg.register("inlet", || {
        Box::new(gantz_core::node::graph::Inlet::default()) as Box<_>
    });
    reg.register("outlet", || {
        Box::new(gantz_core::node::graph::Outlet::default()) as Box<_>
    });
    reg.register("log", || Box::new(gantz_std::Log::default()) as Box<_>);
    reg.register("number", || {
        Box::new(gantz_std::Number::default()) as Box<_>
    });
    reg
}

// ----------------------------------------------
// Graph
// ----------------------------------------------

type Graph = gantz_core::node::graph::Graph<Box<dyn Node>>;
type GraphNode = gantz_core::node::GraphNode<Box<dyn Node>>;

// ----------------------------------------------
// Model
// ----------------------------------------------

struct App {
    state: State,
}

struct State {
    graph: GraphNode,
    graph_hash: u64,
    compiled_module: String,
    logger: gantz_egui::widget::log_view::Logger,
    gantz: gantz_egui::widget::GantzState,
    node_ty_reg: NodeTypeRegistry,
    vm: Engine,
}

// ----------------------------------------------
// Implementation
// ----------------------------------------------

impl App {
    /// The key at which the graph is to be saved/loaded.
    const GRAPH_KEY: &str = "graph";
    /// The key at which the gantz widget state is to be saved/loaded.
    const GANTZ_GUI_STATE_KEY: &str = "gantz-widget-state";

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Setup logging.
        let logger = gantz_egui::widget::log_view::Logger::default();
        log::set_boxed_logger(Box::new(logger.clone())).unwrap();
        log::set_max_level(log::LevelFilter::Info);

        // Load the graph or fallback to a default empty one.
        let graph = cc
            .storage
            .as_ref()
            .and_then(|storage| {
                let Some(graph_str) = storage.get_string(Self::GRAPH_KEY) else {
                    log::debug!("No existing graph to load");
                    return None;
                };
                match ron::de::from_str(&graph_str) {
                    Ok(graph) => {
                        log::debug!("Successfully loaded graph from storage");
                        Some(graph)
                    }
                    Err(e) => {
                        log::error!("Failed to deserialize graph: {e}");
                        None
                    }
                }
            })
            .unwrap_or_else(|| {
                log::debug!("Initialising default graph");
                GraphNode::default()
            });
        let graph_hash = graph_hash(&graph);

        // VM setup
        let mut vm = Engine::new();
        vm.register_value(gantz_core::ROOT_STATE, SteelVal::empty_hashmap());
        gantz_core::graph::register(&graph.graph, &[], &mut vm);
        let module = compile_graph(&graph, &mut vm);
        let compiled_module = fmt_compiled_module(&module);

        // GUI setup
        let ctx = &cc.egui_ctx;
        ctx.set_fonts(egui::FontDefinitions::default());

        // Load the gantz GUI state or fallback to default.
        let gantz = cc
            .storage
            .as_ref()
            .and_then(|storage| {
                let Some(gantz_str) = storage.get_string(Self::GANTZ_GUI_STATE_KEY) else {
                    log::debug!("No existing gantz GUI state to load");
                    return None;
                };
                match ron::de::from_str(&gantz_str) {
                    Ok(gantz) => {
                        log::debug!("Successfully loaded gantz GUI state from storage");
                        Some(gantz)
                    }
                    Err(e) => {
                        log::error!("Failed to deserialize gantz GUI state: {e}");
                        None
                    }
                }
            })
            .unwrap_or_else(|| {
                log::debug!("Initialising default gantz GUI state");
                gantz_egui::widget::GantzState::new()
            });

        let state = State {
            logger,
            gantz,
            graph,
            graph_hash,
            node_ty_reg: node_type_registry(),
            compiled_module,
            vm,
        };

        App { state }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        gui(ctx, &mut self.state);

        // Check for changes to the graph.
        let new_graph_hash = graph_hash(&self.state.graph);
        if self.state.graph_hash != new_graph_hash {
            self.state.graph_hash = new_graph_hash;
            let module = compile_graph(&self.state.graph, &mut self.state.vm);
            self.state.compiled_module = fmt_compiled_module(&module);
        }

        // Process any pending commands generated from the UI.
        process_cmds(&mut self.state.gantz, &mut self.state.vm);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Save the graph.
        let graph_str = match ron::to_string(&self.state.graph) {
            Err(e) => {
                log::error!("Failed to serialize and save graph: {e}");
                return;
            }
            Ok(s) => s,
        };
        storage.set_string(Self::GRAPH_KEY, graph_str);
        log::debug!("Successfully persisted graph");

        // Save the gantz GUI state.
        let gantz_str = match ron::to_string(&self.state.gantz) {
            Err(e) => {
                log::error!("Failed to serialize and save gantz GUI state: {e}");
                return;
            }
            Ok(s) => s,
        };
        storage.set_string(Self::GANTZ_GUI_STATE_KEY, gantz_str);
        log::debug!("Successfully persisted gantz GUI state");
    }

    // Persist GUI state.
    fn persist_egui_memory(&self) -> bool {
        true
    }
}

// Drain the commands provided by the UI and process them.
fn process_cmds(state: &mut gantz_egui::widget::GantzState, vm: &mut Engine) {
    // Process any pending commands.
    for cmd in state.graph_scene.cmds.drain(..) {
        log::debug!("{cmd:?}");
        match cmd {
            gantz_egui::Cmd::PushEval(path) => {
                let fn_name = gantz_core::codegen::push_eval_fn_name(&path);
                if let Err(e) = vm.call_function_by_name_with_args(&fn_name, vec![]) {
                    log::error!("{e}");
                }
            }
            gantz_egui::Cmd::PullEval(path) => {
                let fn_name = gantz_core::codegen::pull_eval_fn_name(&path);
                if let Err(e) = vm.call_function_by_name_with_args(&fn_name, vec![]) {
                    log::error!("{e}");
                }
            }
            gantz_egui::Cmd::OpenGraph(path) => {
                state.path = path;
            }
        }
    }
}

fn compile_graph(graph: &Graph, vm: &mut Engine) -> Vec<ExprKind> {
    // Generate the steel module.
    let module = gantz_core::codegen::module(graph);
    // Compile the eval fns.
    for expr in &module {
        if let Err(e) = vm.run(expr.to_pretty(80)) {
            log::error!("{e}");
        }
    }
    module
}

fn fmt_compiled_module(module: &[ExprKind]) -> String {
    module
        .iter()
        .map(|expr| expr.to_pretty(80))
        .collect::<Vec<String>>()
        .join("\n\n")
}

/// Determine the graph hash. Used between updates to check for changes.
// FIXME: Ideally `Gantz` widget can tell us this in a custom response.
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

fn gui(ctx: &egui::Context, state: &mut State) {
    egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctx, |ui| {
            gantz_egui::widget::Gantz::new(&state.node_ty_reg, &mut state.graph).show(
                &mut state.gantz,
                &state.logger,
                &state.compiled_module,
                &mut state.vm,
                ui,
            );
        });
}
