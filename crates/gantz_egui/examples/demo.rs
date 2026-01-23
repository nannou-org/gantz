//! A simple demonstration of a pure `egui` setup for `gantz`.
//!
//! Includes a top-level `Node` trait with a minimal set of nodes, an
//! environment with a node registry, and a minimal default graph to demonstrate
//! how to use these with the top-level `Gantz` widget in an egui app.

use dyn_clone::DynClone;
use eframe::egui;
use gantz_core::steel::steel_vm::engine::Engine;
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
            .map(|commit_ca| {
                // Store CommitAddr directly (converted to ContentAddr).
                let ref_ = gantz_core::node::Ref::new((*commit_ca).into());
                let named = gantz_egui::node::NamedRef::new(node_type.to_string(), ref_);
                Box::new(named) as Box<_>
            })
            .or_else(|| self.primitives.get(node_type).map(|f| (f)()))
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

// Provide the `NodeRegistry` implementation required by `gantz_core::node::Ref`.
impl gantz_core::node::ref_::NodeRegistry for Environment {
    type Node = dyn gantz_core::Node<Self>;
    fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&Self::Node> {
        // Try commit lookup (for graph refs stored as CommitAddr).
        let commit_ca = gantz_ca::CommitAddr::from(*ca);
        self.registry
            .commit_graph_ref(&commit_ca)
            .map(|g| g as &dyn gantz_core::Node<Self>)
    }
}

// Provide the `NameRegistry` implementation required by `gantz_egui::node::NamedRef`.
impl gantz_egui::node::NameRegistry for Environment {
    fn name_ca(&self, name: &str) -> Option<gantz_ca::ContentAddr> {
        // Return CommitAddr (as ContentAddr) for graph nodes.
        self.registry
            .names()
            .get(name)
            .map(|commit_ca| (*commit_ca).into())
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
    register_primitive(&mut p, "inspect", || {
        Box::new(gantz_egui::node::Inspect::default()) as Box<_>
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
impl Node for gantz_egui::node::NamedRef {}

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

/// View state (layout + camera) for a graph and all its nested subgraphs, keyed by path.
type GraphViews = gantz_egui::GraphViews;

struct State {
    /// The currently open graphs/heads.
    /// Each entry is a head (branch or commit), its associated graph, and view state.
    heads: Vec<(gantz_ca::Head, Graph, GraphViews)>,
    /// Per-head compiled modules, indexed to match `heads`.
    compiled_modules: Vec<String>,
    /// Per-head VMs, indexed to match `heads`.
    vms: Vec<Engine>,
    logger: gantz_egui::widget::log_view::Logger,
    gantz: gantz_egui::widget::GantzState,
    env: Environment,
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
    /// The key at which the list of open heads is stored.
    const OPEN_HEADS_KEY: &str = "open-heads";

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Setup logging.
        let logger = gantz_egui::widget::log_view::Logger::default();
        log::set_boxed_logger(Box::new(logger.clone())).unwrap();
        log::set_max_level(log::LevelFilter::Info);

        // Load the graphs and mappings from storage.
        let (mut registry, open_heads, gantz) = cc
            .storage
            .as_ref()
            .map(|&storage| {
                let graph_addrs = load_graph_addrs(storage);
                let commit_addrs = load_commit_addrs(storage);
                let graphs = load_graphs(storage, graph_addrs.iter().copied());
                let commits = load_commits(storage, commit_addrs.iter().copied());
                let names = load_names(storage);
                let open_heads = load_open_heads(storage);
                let gantz = load_gantz_gui_state(storage);
                let registry = Registry::new(graphs, commits, names);
                (registry, open_heads, gantz)
            })
            .unwrap_or_else(|| {
                log::error!("Unable to access storage");
                (Default::default(), vec![], Default::default())
            });

        // Load all open heads, filtering out invalid ones.
        let heads: Vec<_> = open_heads
            .into_iter()
            .filter_map(|head| {
                let graph = clone_graph(registry.head_graph(&head)?);
                let views = GraphViews::default();
                Some((head, graph, views))
            })
            .collect();

        // If no valid heads remain, create a default one.
        let heads = if heads.is_empty() {
            let head = registry.init_head(timestamp());
            let graph = clone_graph(registry.head_graph(&head).unwrap());
            let views = GraphViews::default();
            vec![(head, graph, views)]
        } else {
            heads
        };

        // Setup the environment that will be provided to all nodes.
        let primitives = primitives();
        let mut env = Environment {
            registry,
            primitives,
        };

        // Prune unused graphs now that we have the environment for node lookups.
        let heads_for_prune = heads.iter().map(|(h, _, _)| h);
        let required = gantz_core::reg::required_commits(&env, &env.registry, heads_for_prune);
        env.registry.prune_unreachable(&required);

        // VM setup - initialize a VM for each open head.
        let mut vms = Vec::with_capacity(heads.len());
        let mut compiled_modules = Vec::with_capacity(heads.len());
        for (_, graph, _) in &heads {
            let (vm, compiled_module) = init_vm(&env, graph);
            vms.push(vm);
            compiled_modules.push(compiled_module);
        }

        // GUI setup.
        let ctx = &cc.egui_ctx;
        ctx.set_fonts(egui::FontDefinitions::default());

        let state = State {
            logger,
            gantz,
            heads,
            env,
            compiled_modules,
            vms,
        };

        App { state }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        gui(ctx, &mut self.state);

        // Check for changes to each open graph and commit/recompile them.
        // FIXME: Rather than checking changed CA to monitor changes, ideally
        // `Gantz` widget can tell us this in a custom response.
        for (ix, (head, graph, _)) in self.state.heads.iter_mut().enumerate() {
            let new_graph_ca = gantz_ca::graph_addr(&*graph);
            let head_commit = self.state.env.registry.head_commit(head).unwrap();
            if head_commit.graph != new_graph_ca {
                let old_head = head.clone();
                let old_commit_ca = self
                    .state
                    .env
                    .registry
                    .head_commit_ca(head)
                    .copied()
                    .unwrap();
                let new_commit_ca = self.state.env.registry.commit_graph_to_head(
                    timestamp(),
                    new_graph_ca,
                    || graph.clone(),
                    head,
                );
                log::debug!(
                    "Graph changed: {} -> {}",
                    old_commit_ca.display_short(),
                    new_commit_ca.display_short()
                );
                // Update the graph pane if the head's commit CA changed.
                gantz_egui::widget::update_graph_pane_head(ctx, &old_head, head);

                // Migrate open_heads entry from old key to new key.
                if let Some(state) = self.state.gantz.open_heads.remove(&old_head) {
                    self.state.gantz.open_heads.insert(head.clone(), state);
                }

                // Recompile this head's graph into its VM.
                let vm = &mut self.state.vms[ix];
                let module = compile_graph(&self.state.env, graph, vm);
                self.state.compiled_modules[ix] = fmt_compiled_module(&module);
            }
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

        // Save all open heads.
        let heads: Vec<_> = self.state.heads.iter().map(|(h, _, _)| h.clone()).collect();
        save_open_heads(storage, &heads);

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

/// Save all open heads to storage.
fn save_open_heads(storage: &mut dyn eframe::Storage, heads: &[gantz_ca::Head]) {
    let heads_str = match ron::to_string(heads) {
        Err(e) => {
            log::error!("Failed to serialize open heads: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(App::OPEN_HEADS_KEY, heads_str);
    log::debug!("Successfully persisted {} open heads", heads.len());
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

/// Load all open heads from storage.
fn load_open_heads(storage: &dyn eframe::Storage) -> Vec<gantz_ca::Head> {
    let Some(heads_str) = storage.get_string(App::OPEN_HEADS_KEY) else {
        log::debug!("No existing open heads to load");
        return vec![];
    };
    match ron::de::from_str(&heads_str) {
        Ok(heads) => {
            log::debug!("Successfully loaded open heads");
            heads
        }
        Err(e) => {
            log::error!("Failed to deserialize open heads: {e}");
            vec![]
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
    // Collect heads with their indices to process.
    let heads_to_process: Vec<_> = state
        .heads
        .iter()
        .enumerate()
        .map(|(ix, (h, _, _))| (ix, h.clone()))
        .collect();

    for (ix, head) in heads_to_process {
        let head_state = state.gantz.open_heads.entry(head.clone()).or_default();
        for cmd in std::mem::take(&mut head_state.scene.cmds) {
            log::debug!("{cmd:?}");
            match cmd {
                gantz_egui::Cmd::PushEval(path) => {
                    let fn_name = gantz_core::compile::push_eval_fn_name(&path);
                    if let Err(e) = state.vms[ix].call_function_by_name_with_args(&fn_name, vec![])
                    {
                        log::error!("{e}");
                    }
                }
                gantz_egui::Cmd::PullEval(path) => {
                    let fn_name = gantz_core::compile::pull_eval_fn_name(&path);
                    if let Err(e) = state.vms[ix].call_function_by_name_with_args(&fn_name, vec![])
                    {
                        log::error!("{e}");
                    }
                }
                gantz_egui::Cmd::OpenGraph(path) => {
                    // Re-borrow head_state to modify path.
                    let head_state = state.gantz.open_heads.get_mut(&head).unwrap();
                    head_state.path = path;
                }
                gantz_egui::Cmd::OpenNamedNode(name, content_ca) => {
                    // The content_ca represents a CommitAddr for graph nodes.
                    let commit_ca = gantz_ca::CommitAddr::from(content_ca);
                    if state.env.registry.names().get(&name) == Some(&commit_ca) {
                        open_head(state, gantz_ca::Head::Branch(name.to_string()));
                    } else {
                        log::debug!(
                            "Attempted to open named node, but the content address has changed"
                        );
                    }
                }
                gantz_egui::Cmd::ForkNamedNode { new_name, ca } => {
                    // The CA represents a CommitAddr for graph nodes.
                    let commit_ca = gantz_ca::CommitAddr::from(ca);
                    state.env.registry.insert_name(new_name.clone(), commit_ca);
                    log::info!("Forked node to new name: {new_name}");
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
            // Build a slice of (Head, &mut Graph, &mut GraphViews) for the Gantz widget.
            let mut heads: Vec<_> = state
                .heads
                .iter_mut()
                .map(|(h, g, l)| (h.clone(), g, l))
                .collect();
            let get_module = |ix| state.compiled_modules.get(ix).map(|s: &String| &s[..]);
            let response = gantz_egui::widget::Gantz::new(&mut state.env, &mut heads)
                .logger(state.logger.clone())
                .show(&mut state.gantz, &get_module, &mut state.vms, ui);

            // The given graph name was removed.
            if let Some(name) = response.graph_name_removed() {
                // Update any open heads that reference this name.
                for (head, _, _) in &mut state.heads {
                    if let gantz_ca::Head::Branch(head_name) = &*head {
                        if *head_name == name {
                            let commit_ca = *state.env.registry.head_commit_ca(head).unwrap();
                            *head = gantz_ca::Head::Commit(commit_ca);
                        }
                    }
                }
                state.env.registry.remove_name(&name);
            }

            // Single click: replace the focused head with the selected one.
            if let Some(new_head) = response.graph_replaced() {
                replace_head(ui.ctx(), state, new_head.clone());
            }

            // Open as a new tab (or focus if already open).
            if let Some(new_head) = response.graph_opened() {
                open_head(state, new_head.clone());
            }

            // Close head.
            if let Some(head) = response.graph_closed() {
                close_head(state, head);
            }

            // Create a new empty graph and open it.
            if response.new_graph() {
                let new_head = state.env.registry.init_head(timestamp());
                open_head(state, new_head);
            }

            // Handle closed heads from tab close buttons.
            for closed_head in &response.closed_heads {
                close_head(state, closed_head);
            }

            // Handle new branch created from tab double-click.
            if let Some((original_head, new_name)) = response.new_branch() {
                create_branch_from_head(ui.ctx(), state, original_head, new_name.clone());
            }
        });
}

/// Open a head as a new tab, or focus it if already open.
///
/// This is only used when selecting from GraphSelect.
fn open_head(state: &mut State, new_head: gantz_ca::Head) {
    // Check if the head is already open.
    if let Some(ix) = state.heads.iter().position(|(h, _, _)| *h == new_head) {
        // Just focus the existing tab.
        state.gantz.focused_head = ix;
        return;
    }

    // Head is not open - add it as a new tab.
    let graph = state.env.registry.head_graph(&new_head).unwrap();
    let new_graph = clone_graph(graph);
    let views = GraphViews::default();

    state
        .heads
        .push((new_head.clone(), new_graph.clone(), views));
    state.gantz.focused_head = state.heads.len() - 1;

    // Initialise the VM for the new graph and add to per-head collections.
    let (vm, compiled_module) = init_vm(&state.env, &new_graph);
    state.vms.push(vm);
    state.compiled_modules.push(compiled_module);

    // Initialize GUI state for the new head.
    state.gantz.open_heads.entry(new_head).or_default();
}

/// Replace the focused head with a new head in-place.
///
/// If the new head is already open elsewhere, focuses that instead.
fn replace_head(ctx: &egui::Context, state: &mut State, new_head: gantz_ca::Head) {
    // If the new head is already open, just focus it.
    if let Some(ix) = state.heads.iter().position(|(h, _, _)| *h == new_head) {
        state.gantz.focused_head = ix;
        return;
    }

    let ix = state.gantz.focused_head;
    let old_head = state.heads[ix].0.clone();

    // Load the new graph.
    let graph = state.env.registry.head_graph(&new_head).unwrap();
    let new_graph = clone_graph(graph);
    let views = GraphViews::default();

    // Replace at the focused index.
    state.heads[ix] = (new_head.clone(), new_graph.clone(), views);

    // Reinitialize the VM for the new graph.
    let (new_vm, new_module) = init_vm(&state.env, &new_graph);
    state.vms[ix] = new_vm;
    state.compiled_modules[ix] = new_module;

    // Update the graph pane to show the new head.
    gantz_egui::widget::update_graph_pane_head(ctx, &old_head, &new_head);

    // Move GUI state from old head to new head.
    if let Some(gui_state) = state.gantz.open_heads.remove(&old_head) {
        state.gantz.open_heads.insert(new_head, gui_state);
    } else {
        state.gantz.open_heads.entry(new_head).or_default();
    }
}

/// Close a head, removing it from the open tabs.
///
/// Does nothing if the head is not open or if it's the last open head.
fn close_head(state: &mut State, head: &gantz_ca::Head) {
    // Don't close if it's the last open head.
    // TODO: Consider opening default empty graph when closing last head.
    if state.heads.len() <= 1 {
        return;
    }
    if let Some(ix) = state.heads.iter().position(|(h, _, _)| h == head) {
        state.heads.remove(ix);
        state.vms.remove(ix);
        state.compiled_modules.remove(ix);
        state.gantz.open_heads.remove(head);

        // Update focused_head to remain valid.
        if ix <= state.gantz.focused_head {
            state.gantz.focused_head = state.gantz.focused_head.saturating_sub(1);
        }
    }
}

/// Create a new branch from an existing head and replace the open head with it.
fn create_branch_from_head(
    ctx: &egui::Context,
    state: &mut State,
    original_head: &gantz_ca::Head,
    new_name: String,
) {
    // Get the commit CA from the original head.
    let Some(commit_ca) = state.env.registry.head_commit_ca(original_head).copied() else {
        log::error!("Failed to get commit address for head: {:?}", original_head);
        return;
    };

    // Insert the new branch name into the registry.
    state.env.registry.insert_name(new_name.clone(), commit_ca);

    // Find the index of the original head and replace it.
    let new_head = gantz_ca::Head::Branch(new_name);
    if let Some(ix) = state.heads.iter().position(|(h, _, _)| h == original_head) {
        let old_head = state.heads[ix].0.clone();
        state.heads[ix].0 = new_head.clone();

        // Update the graph pane to show the new head.
        gantz_egui::widget::update_graph_pane_head(ctx, &old_head, &new_head);

        // Move GUI state from old head to new head.
        if let Some(gui_state) = state.gantz.open_heads.remove(&old_head) {
            state.gantz.open_heads.insert(new_head, gui_state);
        }
    }
}
