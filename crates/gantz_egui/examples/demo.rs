//! A simple demonstration of a pure `egui` setup for `gantz`.
//!
//! Includes a top-level `Node` trait with a minimal set of nodes, an
//! environment with a node registry, and a minimal default graph to demonstrate
//! how to use these with the top-level `Gantz` widget in an egui app.

use dyn_clone::DynClone;
use eframe::egui;
use gantz_core::steel::steel_vm::engine::Engine;
use gantz_egui::ca;
use petgraph::visit::{IntoNodeReferences, NodeRef};
use serde::{Deserialize, Serialize};
use std::{
    any::Any,
    collections::{BTreeMap, HashMap},
};
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
    graphs: HashMap<ca::GraphAddr, Graph>,
    /// A mapping from addresses to commits.
    commits: HashMap<ca::CommitAddr, ca::Commit>,
    /// A mapping from names to graph content addresses.
    names: BTreeMap<String, ca::CommitAddr>,
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
            .and_then(|commit_ca| {
                let graph_ca = self.registry.commits.get(commit_ca)?.graph;
                let named = gantz_egui::node::NamedGraph::new(node_type.to_string(), graph_ca);
                Some(Box::new(named) as Box<_>)
            })
            .or_else(|| self.primitives.get(node_type).map(|f| (f)()))
    }
}

// Provide the `GraphRegistry` implementation required by `gantz_egui`.
impl gantz_egui::node::graph::GraphRegistry for Environment {
    type Node = Box<dyn Node>;
    fn graph(&self, ca: ca::GraphAddr) -> Option<&gantz_core::node::graph::Graph<Self::Node>> {
        self.registry.graphs.get(&ca)
    }
}

// Provide the `GraphRegistry` implementation required by the `GraphSelect` widget.
impl gantz_egui::widget::graph_select::GraphRegistry for Environment {
    fn commits(&self) -> Vec<(&ca::CommitAddr, &ca::Commit)> {
        // Sort commits by newest to oldest.
        let mut commits: Vec<_> = self.registry.commits.iter().collect();
        commits.sort_by(|(_, a), (_, b)| b.timestamp.cmp(&a.timestamp));
        commits
    }

    fn names(&self) -> &BTreeMap<String, ca::CommitAddr> {
        &self.registry.names
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
    Any
    + DynClone
    + gantz_core::ca::CaHash
    + gantz_core::Node<Environment>
    + gantz_egui::NodeUi<Environment>
{
}

dyn_clone::clone_trait_object!(Node);

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
    fn to_graph_mut(&mut self) -> Option<&mut gantz_core::node::graph::Graph<Self::Node>> {
        ((&mut **self) as &mut dyn Any)
            .downcast_mut::<gantz_core::node::GraphNode<Self::Node>>()
            .map(|node| &mut node.graph)
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
    head: gantz_egui::ca::Head,
    graph: Graph,
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
    /// All known graph addresses.
    const GRAPH_ADDRS_KEY: &str = "graph-addrs";
    /// All known graph addresses.
    const COMMIT_ADDRS_KEY: &str = "commit-addrs";
    /// The key at which the mapping from names to graph CAs is stored.
    const NAMES_KEY: &str = "graph-names";
    /// The key at which the active head is stored.
    const HEAD_KEY: &str = "head";

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Setup logging.
        let logger = gantz_egui::widget::log_view::Logger::default();
        log::set_boxed_logger(Box::new(logger.clone())).unwrap();
        log::set_max_level(log::LevelFilter::Info);

        // Load the graphs and mappings from storage.
        let (mut registry, head, gantz) = cc
            .storage
            .as_ref()
            .map(|&storage| {
                let graph_addrs = load_graph_addrs(storage);
                let commit_addrs = load_commit_addrs(storage);
                let graphs = load_graphs(storage, graph_addrs.iter().copied());
                let commits = load_commits(storage, commit_addrs.iter().copied());
                let names = load_names(storage);
                let head = load_head(storage);
                let gantz = load_gantz_gui_state(storage);
                let mut registry = NodeTypeRegistry {
                    graphs,
                    names,
                    commits,
                };
                prune_unused_graphs(&mut registry);
                prune_graphless_commits(&mut registry);
                (registry, head, gantz)
            })
            .unwrap_or_else(|| {
                log::error!("Unable to access storage");
                (Default::default(), None, Default::default())
            });

        // Lookup the active graph or fallback to an empty default.
        let head = match head {
            None => init_head(&mut registry),
            Some(head) => match head_graph(&registry, &head) {
                None => init_head(&mut registry),
                Some(_) => head.clone(),
            },
        };
        let graph = clone_graph(head_graph(&registry, &head).unwrap());

        // Setup the environment that will be provided to all nodes.
        let primitives = primitives();
        let env = Environment {
            registry,
            primitives,
        };

        // VM setup.
        let (vm, compiled_module) = init_vm(&env, &graph);

        // GUI setup.
        let ctx = &cc.egui_ctx;
        ctx.set_fonts(egui::FontDefinitions::default());

        let state = State {
            logger,
            gantz,
            graph,
            head,
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

        // Check for changes to the graph and auto-commit them.
        // FIXME: Rather than checking changed CA to monitor changes, ideally
        // `Gantz` widget can tell us this in a custom response.
        let new_graph_ca = ca::graph_addr(&self.state.graph);
        let head_commit = head_commit(&self.state.env.registry, &self.state.head).unwrap();
        if head_commit.graph != new_graph_ca {
            commit_graph_to_head(
                &mut self.state.env.registry,
                &mut self.state.head,
                &self.state.graph,
                new_graph_ca,
            );
            let module = compile_graph(&self.state.env, &self.state.graph, &mut self.state.vm);
            self.state.compiled_module = fmt_compiled_module(&module);
        }

        // Process any pending commands generated from the UI.
        process_cmds(&mut self.state);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Ensure the active graph is registered.
        let active_ca = ca::graph_addr(&self.state.graph);
        self.state
            .env
            .registry
            .graphs
            .entry(active_ca)
            .or_insert_with(|| clone_graph(&self.state.graph));

        let mut addrs: Vec<_> = self.state.env.registry.graphs.keys().copied().collect();
        addrs.sort();
        save_graph_addrs(storage, &addrs);
        save_graphs(storage, &self.state.env.registry.graphs);

        let mut addrs: Vec<_> = self.state.env.registry.commits.keys().copied().collect();
        addrs.sort();
        save_commit_addrs(storage, &addrs);
        save_commits(storage, &self.state.env.registry.commits);

        save_names(storage, &self.state.env.registry.names);
        save_head(storage, &self.state.head);
        save_gantz_gui_state(storage, &self.state.gantz);
    }

    // Persist GUI state.
    fn persist_egui_memory(&self) -> bool {
        true
    }
}

/// Initialise head to an initial commit pointing to an empty graph.
fn init_head(registry: &mut NodeTypeRegistry) -> ca::Head {
    // Register an empty graph.
    let graph = Graph::default();
    let graph_ca = ca::graph_addr(&graph);
    registry.graphs.insert(graph_ca, graph);

    // Register an initial commit.
    let commit = ca::Commit::timestamped(None, graph_ca);
    let commit_ca = ca::commit_addr(&commit);
    registry.commits.insert(commit_ca, commit);

    ca::Head::Commit(commit_ca)
}

/// Commit the given graph to the given head.
fn commit_graph_to_head(
    reg: &mut NodeTypeRegistry,
    head: &mut ca::Head,
    graph: &Graph,
    graph_ca: ca::GraphAddr,
) {
    // Ensure the graph is registerd.
    reg.graphs
        .entry(graph_ca)
        .or_insert_with(|| clone_graph(graph));

    // Create a new commit.
    let parent_ca = *head_commit_ca(&reg.names, head).unwrap();
    let commit = ca::Commit::timestamped(Some(parent_ca), graph_ca);
    let commit_ca = ca::commit_addr(&commit);
    reg.commits.insert(commit_ca, commit);

    // Update head, or insure the name mapping is up-to-date.
    match *head {
        ca::Head::Commit(ref mut ca) => *ca = commit_ca,
        ca::Head::Branch(ref name) => {
            reg.names.insert(name.to_string(), commit_ca);
        }
    }
}

/// Short-hand for using `dyn-clone` to clone the graph.
fn clone_graph(graph: &Graph) -> Graph {
    graph.map(|_, n| dyn_clone::clone_box(&**n), |_, e| e.clone())
}

/// Save the list of known graph addresses to storage.
fn save_graph_addrs(storage: &mut dyn eframe::Storage, addrs: &[ca::GraphAddr]) {
    let graph_addrs_str = match ron::to_string(addrs) {
        Err(e) => {
            log::error!("Failed to serialize graph addresses: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(App::GRAPH_ADDRS_KEY, graph_addrs_str);
    log::debug!("Successfully persisted known graph addresses");
}

/// Save the list of known commit addresses to storage.
fn save_commit_addrs(storage: &mut dyn eframe::Storage, addrs: &[ca::CommitAddr]) {
    let commit_addrs_str = match ron::to_string(addrs) {
        Err(e) => {
            log::error!("Failed to serialize commit addresses: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(App::COMMIT_ADDRS_KEY, commit_addrs_str);
    log::debug!("Successfully persisted known commit addresses");
}

/// Save all graphs to storage, keyed via their content address.
fn save_graphs(storage: &mut dyn eframe::Storage, graphs: &HashMap<ca::GraphAddr, Graph>) {
    for (&ca, graph) in graphs {
        save_graph(storage, ca, graph);
    }
}

/// Save all commits to storage, keyed via their content address.
fn save_commits(storage: &mut dyn eframe::Storage, commits: &HashMap<ca::CommitAddr, ca::Commit>) {
    for (&ca, commit) in commits {
        save_commit(storage, ca, commit);
    }
}

/// Save the given graph to storage.
fn save_graph(storage: &mut dyn eframe::Storage, ca: ca::GraphAddr, graph: &Graph) {
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

/// Save the given commit to storage.
fn save_commit(storage: &mut dyn eframe::Storage, ca: ca::CommitAddr, commit: &ca::Commit) {
    let key = commit_key(ca);
    let commit_str = match ron::to_string(commit) {
        Err(e) => {
            log::error!("Failed to serialize commit: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(&key, commit_str);
    log::debug!("Successfully persisted commit {key}");
}

/// Save the graph names to storage.
fn save_names(storage: &mut dyn eframe::Storage, names: &BTreeMap<String, ca::CommitAddr>) {
    let graph_names_str = match ron::to_string(names) {
        Err(e) => {
            log::error!("Failed to serialize graph names: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(App::NAMES_KEY, graph_names_str);
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

/// Save the head to storage.
fn save_head(storage: &mut dyn eframe::Storage, head: &ca::Head) {
    let head_str = match ron::to_string(head) {
        Err(e) => {
            log::error!("Failed to serialize and save head: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(App::HEAD_KEY, head_str);
    log::debug!("Successfully persisted head: {head:?}");
}

/// Load the graph addresses from storage.
fn load_graph_addrs(storage: &dyn eframe::Storage) -> Vec<ca::GraphAddr> {
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

/// Load the commit addresses from storage.
fn load_commit_addrs(storage: &dyn eframe::Storage) -> Vec<ca::CommitAddr> {
    let Some(commit_addrs_str) = storage.get_string(App::COMMIT_ADDRS_KEY) else {
        log::debug!("No existing commit address list to load");
        return vec![];
    };
    match ron::de::from_str(&commit_addrs_str) {
        Ok(addrs) => {
            log::debug!("Successfully loaded commit addresses from storage");
            addrs
        }
        Err(e) => {
            log::error!("Failed to deserialize commit addresses: {e}");
            vec![]
        }
    }
}

/// Given access to storage and an iterator yielding known graph addresses, load
/// those graphs into memory.
fn load_graphs(
    storage: &dyn eframe::Storage,
    addrs: impl IntoIterator<Item = ca::GraphAddr>,
) -> HashMap<ca::GraphAddr, Graph> {
    addrs
        .into_iter()
        .filter_map(|ca| Some((ca, load_graph(storage, ca)?)))
        .collect()
}

/// Given access to storage and an iterator yielding known commit addresses,
/// load those commits into memory.
fn load_commits(
    storage: &dyn eframe::Storage,
    addrs: impl IntoIterator<Item = ca::CommitAddr>,
) -> HashMap<ca::CommitAddr, ca::Commit> {
    addrs
        .into_iter()
        .filter_map(|ca| Some((ca, load_commit(storage, ca)?)))
        .collect()
}

/// Load the graph with the given address from storage.
fn load_graph(storage: &dyn eframe::Storage, ca: ca::GraphAddr) -> Option<Graph> {
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

/// Load the commit with the given address from storage.
fn load_commit(storage: &dyn eframe::Storage, ca: ca::CommitAddr) -> Option<ca::Commit> {
    let key = commit_key(ca);
    let Some(commit_str) = storage.get_string(&key) else {
        log::debug!("No commit found for address {key}");
        return None;
    };
    match ron::de::from_str(&commit_str) {
        Ok(commit) => {
            log::debug!("Successfully loaded commit {key} from storage");
            Some(commit)
        }
        Err(e) => {
            log::error!("Failed to deserialize commit {key}: {e}");
            None
        }
    }
}

/// Load the graph names from storage.
fn load_names(storage: &dyn eframe::Storage) -> BTreeMap<String, ca::CommitAddr> {
    let Some(graph_names_str) = storage.get_string(App::NAMES_KEY) else {
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

/// Load the active head.
fn load_head(storage: &dyn eframe::Storage) -> Option<ca::Head> {
    let Some(head_str) = storage.get_string(App::HEAD_KEY) else {
        log::debug!("No existing head to load");
        return None;
    };
    match ron::de::from_str(&head_str) {
        Ok(head) => {
            log::debug!("Successfully loaded head");
            Some(head)
        }
        Err(e) => {
            log::error!("Failed to deserialize head: {e}");
            None
        }
    }
}

/// Load the state of the gantz GUI from storage.
fn load_gantz_gui_state(storage: &dyn eframe::Storage) -> gantz_egui::widget::GantzState {
    storage
        .get_string(App::GANTZ_GUI_STATE_KEY)
        .or_else(|| {
            log::debug!("No existing gantz GUI state to load");
            None
        })
        .and_then(|gantz_str| match ron::de::from_str(&gantz_str) {
            Ok(gantz) => {
                log::debug!("Successfully loaded gantz GUI state from storage");
                Some(gantz)
            }
            Err(e) => {
                log::error!("Failed to deserialize gantz GUI state: {e}");
                None
            }
        })
        .unwrap_or_else(|| {
            log::debug!("Initialising default gantz GUI state");
            gantz_egui::widget::GantzState::new()
        })
}

/// The key for a particular graph in storage.
fn graph_key(ca: ca::GraphAddr) -> String {
    format!("{ca}")
}

/// The key for a particular commit in storage.
fn commit_key(ca: ca::CommitAddr) -> String {
    format!("{ca}")
}

/// Initialise the VM for the given environment and graph.
///
/// Also returns the compiled module for the initial state.
///
/// TODO: Allow loading state from storage.
fn init_vm(env: &Environment, graph: &Graph) -> (Engine, String) {
    let mut vm = Engine::new_base();
    vm.register_value(gantz_core::ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(env, graph, &[], &mut vm);
    let module = compile_graph(env, graph, &mut vm);
    let compiled_module = fmt_compiled_module(&module);
    (vm, compiled_module)
}

// Drain the commands provided by the UI and process them.
fn process_cmds(state: &mut State) {
    // Process any pending commands.
    for cmd in std::mem::take(&mut state.gantz.graph_scene.cmds) {
        log::debug!("{cmd:?}");
        match cmd {
            gantz_egui::Cmd::PushEval(path) => {
                let fn_name = gantz_core::compile::push_eval_fn_name(&path);
                if let Err(e) = state.vm.call_function_by_name_with_args(&fn_name, vec![]) {
                    log::error!("{e}");
                }
            }
            gantz_egui::Cmd::PullEval(path) => {
                let fn_name = gantz_core::compile::pull_eval_fn_name(&path);
                if let Err(e) = state.vm.call_function_by_name_with_args(&fn_name, vec![]) {
                    log::error!("{e}");
                }
            }
            gantz_egui::Cmd::OpenGraph(path) => {
                state.gantz.path = path;
            }
            gantz_egui::Cmd::OpenNamedGraph(name, ca) => {
                if let Some(commit_ca) = state.env.registry.names.get(&name) {
                    let commit = &state.env.registry.commits[commit_ca];
                    if ca == commit.graph {
                        set_head(state, ca::Head::Branch(name.to_string()));
                    } else {
                        log::debug!(
                            "Attempted to open named graph, but the graph address has changed"
                        );
                    }
                }
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

/// Look-up the commit address pointed to by the given head.
fn head_commit_ca<'a>(
    names: &'a BTreeMap<String, ca::CommitAddr>,
    head: &'a ca::Head,
) -> Option<&'a ca::CommitAddr> {
    match head {
        ca::Head::Branch(name) => names.get(name),
        ca::Head::Commit(ca) => Some(ca),
    }
}

/// Look-up the commit pointed to by the given head.
fn head_commit<'a>(reg: &'a NodeTypeRegistry, head: &'a ca::Head) -> Option<&'a ca::Commit> {
    head_commit_ca(&reg.names, head).and_then(|ca| reg.commits.get(&ca))
}

/// Look-up the graph pointed to by the head.
fn head_graph<'a>(reg: &'a NodeTypeRegistry, head: &'a ca::Head) -> Option<&'a Graph> {
    head_commit(reg, head).and_then(|commit| reg.graphs.get(&commit.graph))
}

fn gui(ctx: &egui::Context, state: &mut State) {
    egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctx, |ui| {
            let commit_ca = *head_commit_ca(&state.env.registry.names, &state.head).unwrap();
            let response =
                gantz_egui::widget::Gantz::new(&mut state.env, &mut state.graph, &state.head)
                    .logger(state.logger.clone())
                    .show(&mut state.gantz, &state.compiled_module, &mut state.vm, ui);

            // The graph name was updated, ensure a mapping exists if necessary.
            if let Some(name_opt) = response.graph_name_updated() {
                match name_opt {
                    // If a name was given, ensure it maps to the CA.
                    Some(name) => {
                        state.env.registry.names.insert(name.to_string(), commit_ca);
                        state.head = ca::Head::Branch(name.to_string());
                    }
                    // Otherwise the name was cleared, so just point to the commit.
                    None => {
                        state.head = ca::Head::Commit(commit_ca);
                    }
                }
            // The given graph name was removed.
            } else if let Some(name) = response.graph_name_removed() {
                if let ca::Head::Branch(ref head_name) = state.head {
                    if *head_name == name {
                        state.head = ca::Head::Commit(commit_ca);
                    }
                }
                state.env.registry.names.remove(&name);
            }

            // A graph was selected.
            if let Some(new_head) = response.graph_selected() {
                // TODO: Load state for named graphs?
                set_head(state, new_head.clone());
            }

            // Create a new empty graph and select it.
            if response.new_graph() {
                // Add the empty graph.
                let graph = Graph::default();
                let graph_ca = ca::graph_addr(&graph);
                state.env.registry.graphs.insert(graph_ca, graph);

                // Create a fresh commit.
                let parent = None;
                let commit = ca::Commit::timestamped(parent, graph_ca);
                let commit_ca = ca::commit_addr(&commit);
                state.env.registry.commits.insert(commit_ca, commit);

                // Set the head to the new commit.
                set_head(state, ca::Head::Commit(commit_ca));
            }
        });
}

/// Set the active graph as the graph with the given CA.
///
/// Panics if the given head does not exist.
fn set_head(state: &mut State, new_head: ca::Head) {
    let new_head_commit_ca = head_commit_ca(&state.env.registry.names, &new_head).unwrap();
    let new_head_graph_ca = state.env.registry.commits[&new_head_commit_ca].graph;
    let graph = &state.env.registry.graphs[&new_head_graph_ca];

    // Clone the graph.
    state.graph = clone_graph(graph);
    state.head = new_head;

    // Initialise the VM.
    let (vm, compiled_module) = init_vm(&state.env, &state.graph);
    state.vm = vm;
    state.compiled_module = compiled_module;

    // Clear the graph GUI state (layout, etc).
    state.gantz.path.clear();
    state.gantz.graphs.clear();
    state.gantz.graph_scene.interaction.selection.clear();
}

/// Prune all unused graph entries from the registry.
fn prune_unused_graphs(reg: &mut NodeTypeRegistry) {
    let to_remove: Vec<_> = reg
        .graphs
        .keys()
        .copied()
        .filter(|&ca| !graph_in_use(reg, ca))
        .collect();
    for ca in to_remove {
        reg.graphs.remove(&ca);
    }
}

/// Tests whether or not the graph with the given content address is in use
/// within the registry.
///
/// This is used to determine whether or not to remove unused graphs.
fn graph_in_use(reg: &NodeTypeRegistry, ca: ca::GraphAddr) -> bool {
    reg.names
        .values()
        .any(|commit_ca| ca == reg.commits[commit_ca].graph)
        || reg.graphs.values().any(|g| graph_contains_ca(g, ca))
}

/// Whether or not the graph contains a subgraph with the given CA.
fn graph_contains_ca(g: &Graph, ca: ca::GraphAddr) -> bool {
    g.node_references().any(|n_ref| {
        let node = n_ref.weight();
        ((&**node) as &dyn Any)
            .downcast_ref::<GraphNode>()
            .map(|graph| {
                let graph_ca = ca::graph_addr(&graph.graph);
                ca == graph_ca || graph_contains_ca(&graph.graph, ca)
            })
            .unwrap_or(false)
    })
}

/// Prunes all commits point to graphs that no longer exist.
///
/// Intended for running after `prune_unused_graphs`.
fn prune_graphless_commits(reg: &mut NodeTypeRegistry) {
    reg.commits
        .retain(|_ca, commit| reg.graphs.contains_key(&commit.graph));
}
