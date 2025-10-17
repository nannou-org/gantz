//! A simple demonstration of a pure `egui` setup for `gantz`.
//!
//! Includes a top-level `Node` trait with a minimal set of nodes, an
//! environment with a node registry, and a minimal default graph to demonstrate
//! how to use these with the top-level `Gantz` widget in an egui app.

use dyn_clone::DynClone;
use dyn_hash::DynHash;
use eframe::egui;
use gantz_core::steel::steel_vm::engine::Engine;
use gantz_egui::ContentAddr;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::{any::Any, collections::BTreeMap};
use steel::{SteelVal, parser::ast::ExprKind};

// ----------------------------------------------

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions::default();
    let name = "g a n t z";
    eframe::run_native(name, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

// ----------------------------------------------
// Environment
// ----------------------------------------------

/// The type used to track mappings between node names, content addresses and
/// graphs. Also provides access to the node registry. This can be thought of as
/// a shared immutable input to all nodes.
struct Environment {
    /// Constructors for all primitive nodes.
    primitives: Primitives,
    /// The registry of all nodes composed from other nodes.
    registry: NodeTypeRegistry,
}

/// The registry for all named graphs, i.e. nodes composed from other nodes.
#[derive(Default, Deserialize, Serialize)]
struct NodeTypeRegistry {
    /// A mapping from content addresses to graphs.
    graphs: HashMap<ContentAddr, Graph>,
    /// A mapping from names to graph content addresses.
    names: BTreeMap<String, ContentAddr>,
}

/// Constructors for all primitive nodes.
type Primitives = BTreeMap<String, Box<dyn Fn() -> Box<dyn Node>>>;

// Provide the `NodeTypeRegistry` implementation required by `gantz_egui`.
impl gantz_egui::widget::gantz::NodeTypeRegistry for Environment {
    type Node = Box<dyn Node>;

    fn node_types(&self) -> impl Iterator<Item = &str> {
        let mut types = vec![];
        types.extend(self.primitives.keys().map(|s| &s[..]));
        types.extend(self.registry.names.keys().map(|s| &s[..]));
        types.sort();
        types.into_iter()
    }

    fn new_node(&self, node_type: &str) -> Option<Self::Node> {
        self.registry
            .names
            .get(node_type)
            .map(|&ca| {
                let named = gantz_egui::node::NamedGraph::new(node_type.to_string(), ca);
                Box::new(named) as Box<_>
            })
            .or_else(|| self.primitives.get(node_type).map(|f| (f)()))
    }
}

// Provide the `NodeNameRegistry` implementation required by `gantz_egui`.
impl gantz_egui::node::graph::GraphRegistry for Environment {
    type Node = Box<dyn Node>;
    fn graph(&self, ca: ContentAddr) -> Option<&gantz_core::node::graph::Graph<Self::Node>> {
        self.registry.graphs.get(&ca)
    }
}

/// The set of all known node types accessible to gantz.
fn primitives() -> Primitives {
    let mut p = Primitives::default();
    register_primitive(&mut p, "add", || {
        Box::new(gantz_std::ops::Add::default()) as Box<_>
    });
    register_primitive(&mut p, "bang", || {
        Box::new(gantz_std::Bang::default()) as Box<_>
    });
    register_primitive(&mut p, "expr", || {
        Box::new(gantz_core::node::Expr::new("()").unwrap()) as Box<_>
    });
    register_primitive(&mut p, "graph", || Box::new(GraphNode::default()) as Box<_>);
    register_primitive(&mut p, "inlet", || {
        Box::new(gantz_core::node::graph::Inlet::default()) as Box<_>
    });
    register_primitive(&mut p, "outlet", || {
        Box::new(gantz_core::node::graph::Outlet::default()) as Box<_>
    });
    register_primitive(&mut p, "log", || {
        Box::new(gantz_std::Log::default()) as Box<_>
    });
    register_primitive(&mut p, "number", || {
        Box::new(gantz_std::Number::default()) as Box<_>
    });
    p
}

fn register_primitive(
    primitives: &mut Primitives,
    name: impl Into<String>,
    new: impl 'static + Fn() -> Box<dyn Node>,
) -> Option<Box<dyn Fn() -> Box<dyn Node>>> {
    primitives.insert(name.into(), Box::new(new) as Box<_>)
}

// ----------------------------------------------
// Top-level `Node` trait
// ----------------------------------------------

/// A top-level blanket trait providing trait object cloning, hashing, and serialization.
#[typetag::serde(tag = "type")]
trait Node:
    Any + DynClone + DynHash + gantz_core::Node<Environment> + gantz_egui::NodeUi<Environment>
{
}

dyn_clone::clone_trait_object!(Node);
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
impl Node for gantz_egui::node::NamedGraph {}

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
    graph_ca: ContentAddr,
    compiled_module: String,
    logger: gantz_egui::widget::log_view::Logger,
    gantz: gantz_egui::widget::GantzState,
    env: Environment,
    vm: Engine,
}

// ----------------------------------------------
// Implementation
// ----------------------------------------------

impl App {
    /// The key at which the gantz widget state is to be saved/loaded.
    const GANTZ_GUI_STATE_KEY: &str = "gantz-widget-state";
    /// All known graph content addresses.
    const GRAPH_ADDRS_KEY: &str = "graph-addrs";
    /// The key at which the mapping from names to graph CAs is stored.
    const GRAPH_NAMES_KEY: &str = "graph-names";
    /// The key at which the content address of the active graph is stored.
    const ACTIVE_GRAPH_KEY: &str = "active-graph";

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Setup logging.
        let logger = gantz_egui::widget::log_view::Logger::default();
        log::set_boxed_logger(Box::new(logger.clone())).unwrap();
        log::set_max_level(log::LevelFilter::Info);

        // Load the graphs and mappings from storage.
        let (graphs, names, active_graph) = cc
            .storage
            .as_ref()
            .map(|&storage| {
                let graph_addrs = load_graph_addrs(storage);
                let graphs = load_graphs(storage, graph_addrs.iter().copied());
                let graph_names = load_graph_names(storage);
                let active_graph = load_active_graph(storage);
                (graphs, graph_names, active_graph)
            })
            .unwrap_or_else(|| (Default::default(), Default::default(), None));

        // Lookup the active graph or fallback to an empty default.
        let graph = match active_graph {
            None => GraphNode::default(),
            Some(ca) => {
                let graph = graphs.get(&ca).map(|g| clone_graph(g)).unwrap_or_default();
                GraphNode { graph }
            }
        };
        let graph_ca = gantz_egui::graph_content_addr(&graph);

        // Setup the environment that will be provided to all nodes.
        let registry = NodeTypeRegistry { graphs, names };
        let primitives = primitives();
        let env = Environment {
            registry,
            primitives,
        };

        // VM setup
        let mut vm = Engine::new();
        // TODO: Load state from storage?
        vm.register_value(gantz_core::ROOT_STATE, SteelVal::empty_hashmap());
        gantz_core::graph::register(&env, &graph.graph, &[], &mut vm);
        let module = compile_graph(&env, &graph, &mut vm);
        let compiled_module = fmt_compiled_module(&module);

        // GUI setup.
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
            graph_ca,
            env,
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
        // FIXME: Rather than checking changed CA to monitor changes, ideally
        // `Gantz` widget can tell us this in a custom response.
        let new_graph_ca = gantz_egui::graph_content_addr(&self.state.graph);
        if self.state.graph_ca != new_graph_ca {
            self.state.graph_ca = new_graph_ca;
            let module = compile_graph(&self.state.env, &self.state.graph, &mut self.state.vm);
            self.state.compiled_module = fmt_compiled_module(&module);
        }

        // Process any pending commands generated from the UI.
        process_cmds(&mut self.state.gantz, &mut self.state.vm);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Ensure the active graph is registered.
        let active_ca = gantz_egui::graph_content_addr(&self.state.graph.graph);
        self.state
            .env
            .registry
            .graphs
            .entry(active_ca)
            .or_insert_with(|| clone_graph(&self.state.graph.graph));

        // Save the graph addresses, the graphs and the graph names.
        let mut addrs: Vec<_> = self.state.env.registry.graphs.keys().copied().collect();
        addrs.sort();
        save_graph_addrs(storage, &addrs);
        save_graphs(storage, &self.state.env.registry.graphs);
        save_graph_names(storage, &self.state.env.registry.names);

        // Save the active graph.
        save_active_graph(storage, active_ca);

        // Save the gantz GUI state.
        save_gantz_gui_state(storage, &self.state.gantz);
    }

    // Persist GUI state.
    fn persist_egui_memory(&self) -> bool {
        true
    }
}

/// Short-hand for using `dyn-clone` to clone the graph.
fn clone_graph(graph: &Graph) -> Graph {
    graph.map(|_, n| dyn_clone::clone_box(&**n), |_, e| e.clone())
}

/// Save the list of known content addresses to storage.
fn save_graph_addrs(storage: &mut dyn eframe::Storage, addrs: &[ContentAddr]) {
    let graph_addrs_str = match ron::to_string(addrs) {
        Err(e) => {
            log::error!("Failed to serialize graph content addresses: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(App::GRAPH_ADDRS_KEY, graph_addrs_str);
    log::debug!("Successfully persisted known graph content addresses");
}

/// Save all graphs to storage, keyed via their content address.
fn save_graphs(
    storage: &mut dyn eframe::Storage,
    graphs: &HashMap<ContentAddr, gantz_core::node::graph::Graph<Box<dyn Node>>>,
) {
    for (&ca, graph) in graphs {
        save_graph(storage, ca, graph);
    }
}

/// Save the list of known content addresses to storage.
fn save_graph(
    storage: &mut dyn eframe::Storage,
    ca: ContentAddr,
    graph: &gantz_core::node::graph::Graph<Box<dyn Node>>,
) {
    let key = graph_key(ca);
    let graph_str = match ron::to_string(graph) {
        Err(e) => {
            log::error!("Failed to serialize graph: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(&key, graph_str);
    log::debug!("Successfully persisted graph {key}");
}

/// Save the graph names to storage.
fn save_graph_names(storage: &mut dyn eframe::Storage, names: &BTreeMap<String, ContentAddr>) {
    let graph_names_str = match ron::to_string(names) {
        Err(e) => {
            log::error!("Failed to serialize graph names: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(App::GRAPH_NAMES_KEY, graph_names_str);
    log::debug!("Successfully persisted graph names");
}

/// Save the gantz GUI state.
fn save_gantz_gui_state(storage: &mut dyn eframe::Storage, state: &gantz_egui::widget::GantzState) {
    let gantz_str = match ron::to_string(state) {
        Err(e) => {
            log::error!("Failed to serialize and save gantz GUI state: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(App::GANTZ_GUI_STATE_KEY, gantz_str);
    log::debug!("Successfully persisted gantz GUI state");
}

/// Save the active graph to storage.
fn save_active_graph(storage: &mut dyn eframe::Storage, ca: ContentAddr) {
    // TODO: Use hex formatter rather than `ron`.
    let active_graph_str = match ron::to_string(&ca) {
        Err(e) => {
            log::error!("Failed to serialize active graph CA: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(App::ACTIVE_GRAPH_KEY, active_graph_str);
    log::debug!("Successfully persisted active graph CA");
}

/// Load the graph addresses from storage.
fn load_graph_addrs(storage: &dyn eframe::Storage) -> Vec<ContentAddr> {
    let Some(graph_addrs_str) = storage.get_string(App::GRAPH_ADDRS_KEY) else {
        log::debug!("No existing graph address list to load");
        return vec![];
    };
    match ron::de::from_str(&graph_addrs_str) {
        Ok(addrs) => {
            log::debug!("Successfully loaded graph addresses from storage");
            addrs
        }
        Err(e) => {
            log::error!("Failed to deserialize graph addresses: {e}");
            vec![]
        }
    }
}

/// Given access to storage and an iterator yielding known graph content
/// addresses, load those graphs into memory.
fn load_graphs(
    storage: &dyn eframe::Storage,
    addrs: impl IntoIterator<Item = ContentAddr>,
) -> HashMap<ContentAddr, gantz_core::node::graph::Graph<Box<dyn Node>>> {
    addrs
        .into_iter()
        .filter_map(|ca| Some((ca, load_graph(storage, ca)?)))
        .collect()
}

/// Load the graph with the given content address from storage.
fn load_graph(
    storage: &dyn eframe::Storage,
    ca: ContentAddr,
) -> Option<gantz_core::node::graph::Graph<Box<dyn Node>>> {
    let key = graph_key(ca);
    let Some(graph_str) = storage.get_string(&key) else {
        log::debug!("No graph found for content address {key}");
        return None;
    };
    match ron::de::from_str(&graph_str) {
        Ok(graph) => {
            log::debug!("Successfully loaded graph {key} from storage");
            Some(graph)
        }
        Err(e) => {
            log::error!("Failed to deserialize graph {key}: {e}");
            None
        }
    }
}

/// Load the graph names from storage.
fn load_graph_names(storage: &dyn eframe::Storage) -> BTreeMap<String, ContentAddr> {
    let Some(graph_names_str) = storage.get_string(App::GRAPH_NAMES_KEY) else {
        log::debug!("No existing graph names list to load");
        return BTreeMap::default();
    };
    match ron::de::from_str(&graph_names_str) {
        Ok(names) => {
            log::debug!("Successfully loaded graph names from storage");
            names
        }
        Err(e) => {
            log::error!("Failed to deserialize graph names: {e}");
            BTreeMap::default()
        }
    }
}

/// Load the CA of the active graph if there is one.
fn load_active_graph(storage: &dyn eframe::Storage) -> Option<ContentAddr> {
    let active_graph_str = storage.get_string(App::ACTIVE_GRAPH_KEY)?;
    // TODO: Use from_hex instead of `ron`.
    ron::de::from_str(&active_graph_str).ok()
}

/// The key for a particular graph in storage.
fn graph_key(ca: ContentAddr) -> String {
    format!("{}", gantz_egui::fmt_content_addr(ca))
}

// Drain the commands provided by the UI and process them.
fn process_cmds(state: &mut gantz_egui::widget::GantzState, vm: &mut Engine) {
    // Process any pending commands.
    for cmd in state.graph_scene.cmds.drain(..) {
        log::debug!("{cmd:?}");
        match cmd {
            gantz_egui::Cmd::PushEval(path) => {
                let fn_name = gantz_core::compile::push_eval_fn_name(&path);
                if let Err(e) = vm.call_function_by_name_with_args(&fn_name, vec![]) {
                    log::error!("{e}");
                }
            }
            gantz_egui::Cmd::PullEval(path) => {
                let fn_name = gantz_core::compile::pull_eval_fn_name(&path);
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

fn compile_graph(env: &Environment, graph: &Graph, vm: &mut Engine) -> Vec<ExprKind> {
    // Generate the steel module.
    let module = gantz_core::compile::module(env, graph);
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

fn gui(ctx: &egui::Context, state: &mut State) {
    egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctx, |ui| {
            gantz_egui::widget::Gantz::new(&state.env, &mut state.graph).show(
                &mut state.gantz,
                &state.logger,
                &state.compiled_module,
                &mut state.vm,
                ui,
            );
        });
}
