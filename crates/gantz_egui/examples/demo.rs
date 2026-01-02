//! A simple demonstration of a pure `egui` setup for `gantz`.
//!
//! Includes a top-level `Node` trait with a minimal set of nodes, an
//! environment with a node registry, and a minimal default graph to demonstrate
//! how to use these with the top-level `Gantz` widget in an egui app.

use dyn_clone::DynClone;
use eframe::egui;
use gantz_core::steel::steel_vm::engine::Engine;
use petgraph::visit::{IntoNodeReferences, NodeRef};
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
    registry: Registry,
}

/// Registry of graphs, commits and branch names.
type Registry = gantz_ca::Registry<Graph>;

/// Constructors for all primitive nodes.
type Primitives = BTreeMap<String, Box<dyn Fn() -> Box<dyn Node>>>;

// Provide the `NodeTypeRegistry` implementation required by `gantz_egui`.
impl gantz_egui::widget::gantz::NodeTypeRegistry for Environment {
    type Node = Box<dyn Node>;

    fn node_types(&self) -> impl Iterator<Item = &str> {
        let mut types = vec![];
        types.extend(self.primitives.keys().map(|s| &s[..]));
        types.extend(self.registry.names().keys().map(|s| &s[..]));
        types.sort();
        types.into_iter()
    }

    fn new_node(&self, node_type: &str) -> Option<Self::Node> {
        self.registry
            .names()
            .get(node_type)
            .and_then(|commit_ca| {
                let graph_ca = self.registry.commits().get(commit_ca)?.graph;
                let named = gantz_egui::node::NamedGraph::new(node_type.to_string(), graph_ca);
                Some(Box::new(named) as Box<_>)
            })
            .or_else(|| self.primitives.get(node_type).map(|f| (f)()))
    }
}

// Provide the `GraphRegistry` implementation required by `gantz_egui`.
impl gantz_egui::node::graph::GraphRegistry for Environment {
    type Node = Box<dyn Node>;
    fn graph(
        &self,
        ca: gantz_ca::GraphAddr,
    ) -> Option<&gantz_core::node::graph::Graph<Self::Node>> {
        self.registry.graphs().get(&ca)
    }
}

// Provide the `GraphRegistry` implementation required by the `GraphSelect` widget.
impl gantz_egui::widget::graph_select::GraphRegistry for Environment {
    fn commits(&self) -> Vec<(&gantz_ca::CommitAddr, &gantz_ca::Commit)> {
        // Sort commits by newest to oldest.
        let mut commits: Vec<_> = self.registry.commits().iter().collect();
        commits.sort_by(|(_, a), (_, b)| b.timestamp.cmp(&a.timestamp));
        commits
    }

    fn names(&self) -> &BTreeMap<String, gantz_ca::CommitAddr> {
        &self.registry.names()
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
    Any + DynClone + gantz_ca::CaHash + gantz_core::Node<Environment> + gantz_egui::NodeUi<Environment>
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
    head: gantz_ca::Head,
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
                let mut registry = Registry::new(graphs, commits, names);
                registry.prune_unnamed_graphs(head.as_ref(), graph_contains);
                (registry, head, gantz)
            })
            .unwrap_or_else(|| {
                log::error!("Unable to access storage");
                (Default::default(), None, Default::default())
            });

        // Lookup the active graph or fallback to an empty default.
        let head = match head {
            None => registry.init_head(timestamp()),
            Some(head) => match registry.head_graph(&head) {
                None => registry.init_head(timestamp()),
                Some(_) => head.clone(),
            },
        };
        let graph = clone_graph(registry.head_graph(&head).unwrap());

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
        let new_graph_ca = gantz_ca::graph_addr(&self.state.graph);
        let head_commit = self
            .state
            .env
            .registry
            .head_commit(&self.state.head)
            .unwrap();
        if head_commit.graph != new_graph_ca {
            let graph = &self.state.graph;
            self.state.env.registry.commit_graph_to_head(
                timestamp(),
                new_graph_ca,
                || graph.clone(),
                &mut self.state.head,
            );
            let module = compile_graph(&self.state.env, &self.state.graph, &mut self.state.vm);
            self.state.compiled_module = fmt_compiled_module(&module);
        }

        // Process any pending commands generated from the UI.
        process_cmds(&mut self.state);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let mut addrs: Vec<_> = self.state.env.registry.graphs().keys().copied().collect();
        addrs.sort();
        save_graph_addrs(storage, &addrs);
        save_graphs(storage, &self.state.env.registry.graphs());

        let mut addrs: Vec<_> = self.state.env.registry.commits().keys().copied().collect();
        addrs.sort();
        save_commit_addrs(storage, &addrs);
        save_commits(storage, &self.state.env.registry.commits());

        save_names(storage, &self.state.env.registry.names());
        save_head(storage, &self.state.head);
        save_gantz_gui_state(storage, &self.state.gantz);
    }

    // Persist GUI state.
    fn persist_egui_memory(&self) -> bool {
        true
    }
}

/// Create a timestamp for a commit.
fn timestamp() -> std::time::Duration {
    let now = web_time::SystemTime::now();
    now.duration_since(web_time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
}

/// Short-hand for using `dyn-clone` to clone the graph.
fn clone_graph(graph: &Graph) -> Graph {
    graph.map(|_, n| dyn_clone::clone_box(&**n), |_, e| e.clone())
}

/// Save the list of known graph addresses to storage.
fn save_graph_addrs(storage: &mut dyn eframe::Storage, addrs: &[gantz_ca::GraphAddr]) {
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
fn save_commit_addrs(storage: &mut dyn eframe::Storage, addrs: &[gantz_ca::CommitAddr]) {
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
fn save_graphs(storage: &mut dyn eframe::Storage, graphs: &HashMap<gantz_ca::GraphAddr, Graph>) {
    for (&ca, graph) in graphs {
        save_graph(storage, ca, graph);
    }
}

/// Save all commits to storage, keyed via their content address.
fn save_commits(
    storage: &mut dyn eframe::Storage,
    commits: &HashMap<gantz_ca::CommitAddr, gantz_ca::Commit>,
) {
    for (&ca, commit) in commits {
        save_commit(storage, ca, commit);
    }
}

/// Save the given graph to storage.
fn save_graph(storage: &mut dyn eframe::Storage, ca: gantz_ca::GraphAddr, graph: &Graph) {
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
fn save_commit(
    storage: &mut dyn eframe::Storage,
    ca: gantz_ca::CommitAddr,
    commit: &gantz_ca::Commit,
) {
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
fn save_names(storage: &mut dyn eframe::Storage, names: &BTreeMap<String, gantz_ca::CommitAddr>) {
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
fn save_head(storage: &mut dyn eframe::Storage, head: &gantz_ca::Head) {
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
fn load_graph_addrs(storage: &dyn eframe::Storage) -> Vec<gantz_ca::GraphAddr> {
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
fn load_commit_addrs(storage: &dyn eframe::Storage) -> Vec<gantz_ca::CommitAddr> {
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
    addrs: impl IntoIterator<Item = gantz_ca::GraphAddr>,
) -> HashMap<gantz_ca::GraphAddr, Graph> {
    addrs
        .into_iter()
        .filter_map(|ca| Some((ca, load_graph(storage, ca)?)))
        .collect()
}

/// Given access to storage and an iterator yielding known commit addresses,
/// load those commits into memory.
fn load_commits(
    storage: &dyn eframe::Storage,
    addrs: impl IntoIterator<Item = gantz_ca::CommitAddr>,
) -> HashMap<gantz_ca::CommitAddr, gantz_ca::Commit> {
    addrs
        .into_iter()
        .filter_map(|ca| Some((ca, load_commit(storage, ca)?)))
        .collect()
}

/// Load the graph with the given address from storage.
fn load_graph(storage: &dyn eframe::Storage, ca: gantz_ca::GraphAddr) -> Option<Graph> {
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
fn load_commit(
    storage: &dyn eframe::Storage,
    ca: gantz_ca::CommitAddr,
) -> Option<gantz_ca::Commit> {
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
fn load_names(storage: &dyn eframe::Storage) -> BTreeMap<String, gantz_ca::CommitAddr> {
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
fn load_head(storage: &dyn eframe::Storage) -> Option<gantz_ca::Head> {
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
fn graph_key(ca: gantz_ca::GraphAddr) -> String {
    format!("{ca}")
}

/// The key for a particular commit in storage.
fn commit_key(ca: gantz_ca::CommitAddr) -> String {
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
    // Process pending commands for the active head.
    let head_state = state.gantz.open_heads.entry(state.head.clone()).or_default();
    for cmd in std::mem::take(&mut head_state.scene.cmds) {
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
                // Re-borrow head_state to modify path.
                let head_state = state.gantz.open_heads.get_mut(&state.head).unwrap();
                head_state.path = path;
            }
            gantz_egui::Cmd::OpenNamedGraph(name, ca) => {
                if let Some(commit) = state.env.registry.named_commit(&name) {
                    if ca == commit.graph {
                        set_head(state, gantz_ca::Head::Branch(name.to_string()));
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

fn gui(ctx: &egui::Context, state: &mut State) {
    egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctx, |ui| {
            let commit_ca = *state.env.registry.head_commit_ca(&state.head).unwrap();
            // Create a single-element slice for the Gantz widget.
            let mut heads = [(state.head.clone(), &mut state.graph)];
            let response =
                gantz_egui::widget::Gantz::new(&mut state.env, &mut heads)
                    .logger(state.logger.clone())
                    .show(&mut state.gantz, &state.compiled_module, &mut state.vm, ui);

            // The given graph name was removed.
            if let Some(name) = response.graph_name_removed() {
                if let gantz_ca::Head::Branch(ref head_name) = state.head {
                    if *head_name == name {
                        state.head = gantz_ca::Head::Commit(commit_ca);
                    }
                }
                state.env.registry.remove_name(&name);
            }

            // A graph was selected.
            if let Some(new_head) = response.graph_selected() {
                // TODO: Load state for named graphs?
                set_head(state, new_head.clone());
            }

            // Create a new empty graph and select it.
            if response.new_graph() {
                // Set the head to a new commit.
                let new_head = state.env.registry.init_head(timestamp());
                set_head(state, new_head);
            }
        });
}

/// Set the active graph as the graph with the given CA.
///
/// Panics if the given head does not exist.
fn set_head(state: &mut State, new_head: gantz_ca::Head) {
    let graph = state.env.registry.head_graph(&new_head).unwrap();

    // Clone the graph.
    state.graph = clone_graph(graph);
    state.head = new_head.clone();

    // Initialise the VM.
    let (vm, compiled_module) = init_vm(&state.env, &state.graph);
    state.vm = vm;
    state.compiled_module = compiled_module;

    // Clear the head's GUI state (layout, etc), or create default if not present.
    let head_state = state.gantz.open_heads.entry(new_head).or_default();
    head_state.path.clear();
    head_state.graphs.clear();
    head_state.scene.interaction.selection.clear();
}

/// Whether or not the graph contains a subgraph with the given CA.
fn graph_contains(g: &Graph, ca: &gantz_ca::GraphAddr) -> bool {
    g.node_references().any(|n_ref| {
        let node = n_ref.weight();
        ((&**node) as &dyn Any)
            .downcast_ref::<GraphNode>()
            .map(|graph| {
                let graph_ca = gantz_ca::graph_addr(&graph.graph);
                *ca == graph_ca || graph_contains(&graph.graph, ca)
            })
            .unwrap_or(false)
    })
}
