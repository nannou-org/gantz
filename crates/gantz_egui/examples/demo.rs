//! A simple demonstration of a pure `egui` setup for `gantz`.
//!
//! Includes a node codec over a minimal node set, an environment with a node
//! registry, and a minimal default graph to demonstrate how to use these with
//! the top-level `Gantz` widget in an egui app.

use eframe::egui;
use gantz_core::{compile::push_pull_entrypoints, steel::steel_vm::engine::Engine};
use gantz_egui::node::DynNode;
use gantz_egui::{HeadAccess, HeadDataMut};
use std::collections::{BTreeMap, HashMap};

// ----------------------------------------------

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
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
    /// The builtin (primitive) node palette as data.
    builtins: gantz_core::Builtins,
    /// One reified builtin instance per palette entry, for introspection.
    instances: gantz_egui::node::UiBuiltins,
    /// The registry of all nodes composed from other nodes, stored as data.
    registry: Registry,
    /// The registry's graphs reified through the demo's node codec, for typed
    /// node lookups. Kept in step with the registry via [`ensure_reified`].
    ///
    /// [`ensure_reified`]: Environment::ensure_reified
    reified: gantz_core::data::ReifiedGraphs<DynNode>,
}

impl Environment {
    /// Look up a node by content address.
    fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&dyn gantz_core::Node> {
        // Graph refs pin graph addresses: a graph in the registry IS a node.
        let graph_ca = gantz_ca::GraphAddr::from(*ca);
        self.reified
            .get(&graph_ca)
            .map(|g| g as &dyn gantz_core::Node)
    }

    /// The graph address at the tip of the named line of history: the
    /// address a `Ref` to the name should pin.
    fn head_graph_addr(&self, name: &gantz_ca::Name) -> Option<gantz_ca::GraphAddr> {
        let head_ca = self.registry.head(name)?;
        self.registry.commits().get(&head_ca).map(|c| c.graph)
    }

    /// The typed graph at the given head's tip, if reified.
    fn head_graph(&self, head: &gantz_ca::Head) -> Option<&Graph> {
        let commit = self.registry.head_commit(head)?;
        self.reified.get(&commit.graph)
    }

    /// Bring the reified cache up to date with the registry, logging graphs
    /// that fail to decode (they degrade like any missing node).
    fn ensure_reified(&mut self) {
        let codec = codec();
        let reify = |nd: &gantz_ca::NodeData| codec.reify_ui(nd).map(|inst| inst.node);
        for e in self.reified.ensure_all_with(&self.registry, reify) {
            log::error!("failed to reify registry graph: {e}");
        }
    }

    /// The borrowed [`gantz_egui::Env`] view over this environment's parts.
    fn as_env<'a>(&'a self, codec: &'a gantz_egui::node::NodeCodec) -> gantz_egui::Env<'a> {
        gantz_egui::Env {
            registry: &self.registry,
            builtins: &self.builtins,
            codec,
            graphs: &self.reified,
            instances: &self.instances,
        }
    }
}

/// Registry of graphs (in erased data form), commits and branch names.
type Registry = gantz_ca::Registry;

impl Environment {
    /// Create a node of the given type name, in its stored data form.
    fn new_node(&self, node_type: &str) -> Option<gantz_ca::NodeData> {
        let name: gantz_ca::Name = node_type.parse().expect("infallible");
        self.head_graph_addr(&name)
            .and_then(|graph_ca| {
                let ref_ = gantz_core::node::Ref::new(graph_ca.into());
                let named = gantz_egui::node::NamedRef::new(name.clone(), ref_);
                gantz_core::data::erase_node_typed(&named).ok()
            })
            .or_else(|| self.builtins.node_data(node_type).cloned())
    }

    /// The head's committed graph data cloned from the registry, for use as
    /// a working graph.
    fn head_data_graph(&self, head: &gantz_ca::Head) -> Option<gantz_ca::DataGraph> {
        let commit = self.registry.head_commit(head)?;
        self.registry.graph(&commit.graph).cloned()
    }
}

/// The set of all known primitive node types accessible to gantz, as data.
fn builtins() -> gantz_core::Builtins {
    use gantz_core::Builtin;
    gantz_core::Builtins::from_specs([
        Builtin::new("bang", &gantz_std::Bang::default()),
        Builtin::new("branch", &gantz_core::node::Branch::default()),
        Builtin::new("delay", &gantz_core::node::Delay::default()),
        Builtin::new("expr", &gantz_core::node::Expr::new("()").unwrap()),
        Builtin::new("inlet", &gantz_core::node::graph::Inlet::default()),
        Builtin::new("inspect", &gantz_egui::node::Inspect::default()),
        Builtin::new("outlet", &gantz_core::node::graph::Outlet::default()),
        Builtin::new("log", &gantz_std::Log::default()),
        Builtin::new("number", &gantz_std::Number::default()),
        Builtin::new("plot", &gantz_egui::node::Plot::default()),
    ])
}

/// The value-level codec for the demo's node set: THE node-set manifest.
fn codec() -> gantz_egui::node::NodeCodec {
    gantz_egui::ui_node_codec! {
        NodeSet {
            gantz_core::node::Branch,
            gantz_core::node::Delay,
            gantz_core::node::Expr,
            gantz_core::node::graph::Inlet,
            gantz_core::node::graph::Outlet,
            gantz_std::Bang,
            gantz_std::Log,
            gantz_std::Number,
            gantz_egui::node::Inspect,
            gantz_egui::node::NamedRef,
            gantz_egui::node::Plot,
        }
    }
}

/// The `.gantz` keyword sugar carrier for the demo's node set: the
/// `gantz_core`, `gantz_std` and `gantz_egui` node sugars (no bevy nodes here).
struct NodeSet;

impl gantz_format::NodeSugar for NodeSet {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![
            &gantz_format::CoreSugar,
            &gantz_std::StdSugar,
            &gantz_egui::EguiSugar,
        ])
    }
}

// ----------------------------------------------
// Graph
// ----------------------------------------------

type Graph = gantz_core::node::graph::Graph<DynNode>;

// ----------------------------------------------
// HeadAccess
// ----------------------------------------------

/// Provides [`HeadAccess`] implementation for the demo app's Vec-based storage.
struct DemoHeadAccess<'a> {
    /// Pre-collected heads for returning from `heads()`.
    head_keys: Vec<gantz_ca::Head>,
    /// Map from head to index for lookup.
    head_to_ix: HashMap<gantz_ca::Head, usize>,
    /// The underlying data.
    data: &'a mut Vec<(gantz_ca::Head, gantz_ca::DataGraph, gantz_egui::SceneView)>,
    modules: &'a [Option<gantz_core::vm::Compiled>],
    compile_errors: &'a [Option<String>],
    diagnostics: &'a [Vec<gantz_core::Diagnostic>],
    vms: &'a mut Vec<Engine>,
}

impl<'a> DemoHeadAccess<'a> {
    fn new(
        data: &'a mut Vec<(gantz_ca::Head, gantz_ca::DataGraph, gantz_egui::SceneView)>,
        modules: &'a [Option<gantz_core::vm::Compiled>],
        compile_errors: &'a [Option<String>],
        diagnostics: &'a [Vec<gantz_core::Diagnostic>],
        vms: &'a mut Vec<Engine>,
    ) -> Self {
        let head_keys: Vec<_> = data.iter().map(|(h, _, _)| h.clone()).collect();
        let head_to_ix: HashMap<_, _> = head_keys
            .iter()
            .enumerate()
            .map(|(ix, h)| (h.clone(), ix))
            .collect();
        Self {
            head_keys,
            head_to_ix,
            data,
            modules,
            compile_errors,
            diagnostics,
            vms,
        }
    }
}

impl<'a> HeadAccess for DemoHeadAccess<'a> {
    fn heads(&self) -> &[gantz_ca::Head] {
        &self.head_keys
    }

    fn with_head_mut<R>(
        &mut self,
        head: &gantz_ca::Head,
        f: impl FnOnce(HeadDataMut<'_>) -> R,
    ) -> Option<R> {
        let ix = *self.head_to_ix.get(head)?;
        let (_, graph, view) = &mut self.data[ix];
        let vm = &mut self.vms[ix];
        Some(f(HeadDataMut { graph, view, vm }))
    }

    fn module(&self, head: &gantz_ca::Head) -> Option<&gantz_core::vm::Compiled> {
        let ix = *self.head_to_ix.get(head)?;
        self.modules[ix].as_ref()
    }

    fn compile_error(&self, head: &gantz_ca::Head) -> Option<&str> {
        let ix = *self.head_to_ix.get(head)?;
        self.compile_errors[ix].as_deref()
    }

    fn diagnostics(&self, head: &gantz_ca::Head) -> &[gantz_core::Diagnostic] {
        self.head_to_ix
            .get(head)
            .map(|&ix| &self.diagnostics[ix][..])
            .unwrap_or(&[])
    }
}

/// The per-head artifacts of one compile attempt: the module artifact (kept
/// even when steel rejected it, for display and span resolution), the
/// rendered error chain on failure, and compile diagnostics.
fn compile_results(
    result: Result<gantz_core::vm::Compiled, gantz_core::vm::CompileError>,
) -> (
    Option<gantz_core::vm::Compiled>,
    Option<String>,
    Vec<gantz_core::Diagnostic>,
) {
    match result {
        Ok(module) => (Some(module), None, vec![]),
        Err(e) => {
            let error = gantz_core::vm::error_chain(&e);
            log::error!("Failed to compile graph: {error}");
            let diags = gantz_core::diagnostic::from_compile_error(&e);
            (e.into_module(), Some(error), diags)
        }
    }
}

// ----------------------------------------------
// Model
// ----------------------------------------------

struct App {
    state: State,
}

struct State {
    /// The currently open graphs/heads.
    /// Each entry is a head (branch or commit), its working graph (in its
    /// stored data form), and view state.
    heads: Vec<(gantz_ca::Head, gantz_ca::DataGraph, gantz_egui::SceneView)>,
    /// Per-head compiled modules, indexed to match `heads`.
    compile_errors: Vec<Option<String>>,
    /// Per-head module artifacts for span resolution, indexed to match `heads`.
    modules: Vec<Option<gantz_core::vm::Compiled>>,
    /// Per-head diagnostics, indexed to match `heads`.
    diagnostics: Vec<Vec<gantz_core::Diagnostic>>,
    /// Per-head VMs, indexed to match `heads`.
    vms: Vec<Engine>,
    /// Index of the currently focused head.
    focused_head: usize,
    /// The compile config used for all heads (session-only, not persisted).
    compile_config: gantz_core::compile::Config,
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
    /// The key at which the registry's metadata sections (heads, views,
    /// descriptions, demos) are stored.
    const SECTIONS_KEY: &str = "registry-sections";
    /// The key at which the list of open heads is stored.
    const OPEN_HEADS_KEY: &str = "open-heads";

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Setup logging.
        let logger = gantz_egui::widget::log_view::Logger::default();
        log::set_boxed_logger(Box::new(logger.clone())).unwrap();
        log::set_max_level(log::LevelFilter::Info);

        // Load the graphs and mappings from storage.
        let (registry, open_heads, gantz) = cc
            .storage
            .as_ref()
            .map(|&storage| {
                let graph_addrs = load_graph_addrs(storage);
                let commit_addrs = load_commit_addrs(storage);
                let graphs = load_graphs(storage, graph_addrs.iter().copied());
                let commits = load_commits(storage, commit_addrs.iter().copied());
                let open_heads = load_open_heads(storage);
                let gantz = load_gantz_gui_state(storage);
                let mut registry = Registry::from_parts(graphs, commits, BTreeMap::new());
                for (id, section) in load_sections(storage) {
                    for (key, value) in section.entries {
                        registry.set_section_value(
                            id.clone(),
                            section.policy,
                            section.liveness,
                            key,
                            value,
                        );
                    }
                }
                (registry, open_heads, gantz)
            })
            .unwrap_or_else(|| {
                log::error!("Unable to access storage");
                (Default::default(), vec![], Default::default())
            });

        // Setup the environment that will be provided to all nodes, reifying
        // the stored graphs through the demo's node set.
        let builtins = builtins();
        let (instances, errs) = gantz_egui::node::UiBuiltins::reify(&builtins, &codec());
        for e in errs {
            log::error!("failed to reify builtin: {e}");
        }
        let mut env = Environment {
            registry,
            builtins,
            instances,
            reified: Default::default(),
        };
        env.ensure_reified();

        // Load all open heads, filtering out invalid ones. Working graphs
        // are the stored data cloned straight from the registry.
        let heads: Vec<_> = open_heads
            .into_iter()
            .filter_map(|head| {
                let graph = env.head_data_graph(&head)?;
                let view = gantz_egui::SceneView::default();
                Some((head, graph, view))
            })
            .collect();

        // If no valid heads remain, create a default one.
        let heads = if heads.is_empty() {
            let head = env.registry.init_head(timestamp());
            env.ensure_reified();
            let graph = env.head_data_graph(&head).unwrap();
            let view = gantz_egui::SceneView::default();
            vec![(head, graph, view)]
        } else {
            heads
        };

        // Prune unused content: reachability is a pure data walk over the
        // stored graphs' refs columns.
        let live = {
            let seeds = heads
                .iter()
                .filter_map(|(h, _, _)| env.registry.head_commit_ca(h));
            gantz_ca::closure(&env.registry, seeds)
        };
        gantz_ca::prune(&mut env.registry, &live);
        env.reified.retain_live(&live);

        // VM setup - initialize a VM for each open head.
        let compile_config = gantz_core::compile::Config::default();
        let mut vms = Vec::with_capacity(heads.len());
        let mut compile_errors = Vec::with_capacity(heads.len());
        let mut modules = Vec::with_capacity(heads.len());
        let mut diagnostics = Vec::with_capacity(heads.len());
        for (head, _, _) in &heads {
            // Compile from the reified cache at the head's committed address
            // (the working graph equals it).
            let get_node = |ca: &gantz_ca::ContentAddr| env.node(ca);
            let graph = env.head_graph(head).expect("head graph reified above");
            let eps = push_pull_entrypoints(&get_node, graph);
            // A default engine keeps indices aligned on failure.
            let (vm, result) = match gantz_core::vm::init(&get_node, graph, &eps, &compile_config) {
                Ok((vm, module)) => (vm, Ok(module)),
                Err(e) => (Engine::new_base(), Err(e)),
            };
            let (module, error, diags) = compile_results(result);
            vms.push(vm);
            compile_errors.push(error);
            modules.push(module);
            diagnostics.push(diags);
        }

        // GUI setup.
        let ctx = &cc.egui_ctx;
        ctx.set_fonts(egui::FontDefinitions::default());

        let state = State {
            logger,
            gantz,
            heads,
            env,
            compile_errors,
            modules,
            diagnostics,
            vms,
            focused_head: 0,
            compile_config,
        };

        App { state }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let responses = gui(ui, &mut self.state);

        // Check for changes to each open graph and commit them.
        // FIXME: Rather than checking changed CA to monitor changes, ideally
        // `Gantz` widget can tell us this in a custom response.
        let mut committed_ixs = Vec::new();
        for (ix, (head, graph, _)) in self.state.heads.iter_mut().enumerate() {
            // The working graph IS the stored form: address it directly.
            let new_graph_ca = gantz_ca::graph_addr(&*graph);
            let head_commit = self.state.env.registry.head_commit(head).unwrap();
            if head_commit.graph != new_graph_ca {
                let old_head = head.clone();
                let old_commit_ca = self.state.env.registry.head_commit_ca(head).unwrap();
                let data_graph = graph.clone();
                let new_commit_ca = self.state.env.registry.commit_graph_to_head(
                    timestamp(),
                    new_graph_ca,
                    || data_graph,
                    head,
                );
                log::debug!(
                    "Graph changed: {} -> {}",
                    old_commit_ca.display_short(),
                    new_commit_ca.display_short()
                );
                // Update the graph pane if the head's commit CA changed.
                gantz_egui::widget::update_graph_pane_head(&ctx, &old_head, head);
                self.state.gantz.migrate_head(&old_head, head, true);
                committed_ixs.push(ix);
            }
        }

        // Recompile the committed heads (from the freshly reified cache) and
        // propagate the edits to referrers (e.g. nested graph -> parent).
        if !committed_ixs.is_empty() {
            self.state.env.ensure_reified();
            for ix in committed_ixs {
                recompile_head(&mut self.state, ix);
            }
            resync_and_refresh(&mut self.state);
        }

        // Process any pending response payloads generated from the UI.
        process_responses(&ctx, &mut self.state, responses);
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

        save_sections(storage, self.state.env.registry.sections());

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
fn save_graphs(
    storage: &mut dyn eframe::Storage,
    graphs: &HashMap<gantz_ca::GraphAddr, gantz_ca::DataGraph>,
) {
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
fn save_graph(
    storage: &mut dyn eframe::Storage,
    ca: gantz_ca::GraphAddr,
    graph: &gantz_ca::DataGraph,
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

/// Save the registry's metadata sections to storage.
fn save_sections(
    storage: &mut dyn eframe::Storage,
    sections: &BTreeMap<gantz_ca::SectionId, gantz_ca::Section>,
) {
    let sections_str = match ron::to_string(sections) {
        Err(e) => {
            log::error!("Failed to serialize registry sections: {e}");
            return;
        }
        Ok(s) => s,
    };
    storage.set_string(App::SECTIONS_KEY, sections_str);
    log::debug!("Successfully persisted registry sections");
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
) -> HashMap<gantz_ca::GraphAddr, gantz_ca::DataGraph> {
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
fn load_graph(
    storage: &dyn eframe::Storage,
    ca: gantz_ca::GraphAddr,
) -> Option<gantz_ca::DataGraph> {
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

/// Load the registry's metadata sections from storage.
fn load_sections(
    storage: &dyn eframe::Storage,
) -> BTreeMap<gantz_ca::SectionId, gantz_ca::Section> {
    let Some(sections_str) = storage.get_string(App::SECTIONS_KEY) else {
        log::debug!("No existing registry sections to load");
        return BTreeMap::default();
    };
    match ron::de::from_str(&sections_str) {
        Ok(sections) => {
            log::debug!("Successfully loaded registry sections from storage");
            sections
        }
        Err(e) => {
            log::error!("Failed to deserialize registry sections: {e}");
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

/// Resolve a payload's head tag to the head and its index in `state.heads`.
fn tagged_head(state: &State, head: Option<gantz_ca::Head>) -> Option<(gantz_ca::Head, usize)> {
    let head = head?;
    let ix = state.heads.iter().position(|(h, _, _)| *h == head)?;
    Some((head, ix))
}

// Drain the response payloads emitted by the UI and process them.
fn process_responses(ctx: &egui::Context, state: &mut State, mut responses: gantz_egui::Responses) {
    for (head, gantz_egui::EvalEntry(ep)) in responses.take() {
        let Some((_, ix)) = tagged_head(state, head) else {
            continue;
        };
        let fn_name = gantz_core::compile::entry_fn_name(&ep.id());
        let result = state.vms[ix].call_function_by_name_with_args(&fn_name, vec![]);
        // Runtime diagnostics reflect the latest evaluation only.
        let diags = &mut state.diagnostics[ix];
        diags.retain(|d| d.severity != gantz_core::diagnostic::Severity::Runtime);
        if let Err(e) = result {
            if let Some(compiled) = &state.modules[ix] {
                let vm = &state.vms[ix];
                diags.push(gantz_core::diagnostic::from_eval_error(&e, vm, compiled));
            }
            log::error!("{e}");
        }
    }

    for (_, gantz_egui::OpenHead(target)) in responses.take() {
        open_head(state, target);
    }

    for (_, gantz_egui::ReplaceHead(target)) in responses.take() {
        replace_head(ctx, state, target);
    }

    for (head, branch) in responses.take::<gantz_egui::BranchNode>() {
        let Some((_, ix)) = tagged_head(state, head) else {
            continue;
        };
        let gantz_egui::BranchNode { new_name, ca, path } = branch;
        let (_, graph, _) = &mut state.heads[ix];
        gantz_egui::ops::branch_node(
            &mut state.env.registry,
            timestamp(),
            graph,
            new_name,
            ca,
            &path,
        );
    }

    for (head, inspect) in responses.take::<gantz_egui::InspectEdge>() {
        let Some((_, ix)) = tagged_head(state, head) else {
            continue;
        };
        let env = &state.env;
        let get_node = |ca: &gantz_ca::ContentAddr| env.node(ca);
        let (_, graph, view) = &mut state.heads[ix];
        gantz_egui::ops::inspect_edge(
            &codec(),
            &get_node,
            || env.new_node("inspect"),
            graph,
            view,
            &mut state.vms[ix],
            inspect,
        );
    }

    for (head, create) in responses.take::<gantz_egui::CreateNode>() {
        let Some((head, ix)) = tagged_head(state, head) else {
            continue;
        };
        let editing = match &head {
            gantz_ca::Head::Branch(name) => Some(name.to_string()),
            gantz_ca::Head::Commit(_) => None,
        };
        let head_state = state.gantz.open_heads.entry(head).or_default();
        let env = &state.env;
        let get_node = |ca: &gantz_ca::ContentAddr| env.node(ca);
        let (_, graph, view) = &mut state.heads[ix];
        gantz_egui::ops::create_node(
            &state.env.registry,
            editing.as_deref(),
            &codec(),
            &get_node,
            |node_type| env.new_node(node_type),
            graph,
            view,
            head_state,
            &mut state.vms[ix],
            create,
        );
    }

    for (head, create) in responses.take::<gantz_egui::CreateNestedGraph>() {
        let Some((head, ix)) = tagged_head(state, head) else {
            continue;
        };
        let gantz_ca::Head::Branch(parent) = head else {
            log::warn!("CreateNestedGraph: name the graph before adding a nested graph");
            continue;
        };
        let head_state = state
            .gantz
            .open_heads
            .entry(gantz_ca::Head::Branch(parent.clone()))
            .or_default();
        let (_, graph, view) = &mut state.heads[ix];
        gantz_egui::ops::create_nested_graph(
            &mut state.env.registry,
            timestamp(),
            graph,
            view,
            head_state,
            create.pos,
            &parent,
        );
        // The fresh nested graph must be reified before its NamedRef resolves.
        state.env.ensure_reified();
    }

    for (head, gantz_egui::CopyNodes(nodes)) in responses.take() {
        let Some((_, ix)) = tagged_head(state, head) else {
            continue;
        };
        let (_, graph, gv) = &mut state.heads[ix];
        let text = gantz_egui::ops::copy_nodes(&state.env.registry, graph, gv, &nodes, &codec());
        if let Some(text) = text {
            ctx.copy_text(text);
        }
    }

    for (head, gantz_egui::Paste { text, pos }) in responses.take() {
        let Some((head, ix)) = tagged_head(state, head) else {
            continue;
        };
        // In eframe, Event::Paste provides text directly.
        let Some(text) = text else { continue };
        let editing = match &head {
            gantz_ca::Head::Branch(name) => Some(name.to_string()),
            gantz_ca::Head::Commit(_) => None,
        };
        let head_state = state.gantz.open_heads.entry(head).or_default();
        let (_, graph, gv) = &mut state.heads[ix];
        let pasted = gantz_egui::ops::paste(
            &mut state.env.registry,
            editing.as_deref(),
            graph,
            gv,
            head_state,
            &text,
            &pos,
            &codec(),
        );
        // Re-register the full root graph so pasted nodes get their state
        // initialized. Idempotent for existing nodes; registration reifies
        // the graph transiently.
        if pasted {
            let vm = &mut state.vms[ix];
            let get_node = |ca: &gantz_ca::ContentAddr| state.env.node(ca);
            match codec().reify_graph(graph) {
                Ok(g) => gantz_core::graph::register(&get_node, &g, &[], vm),
                Err(e) => log::error!("Paste: cannot re-register the pasted graph: {e}"),
            }
        }
    }

    for (head, gantz_egui::CutNodes(nodes)) in responses.take() {
        let Some((head, ix)) = tagged_head(state, head) else {
            continue;
        };
        let head_state = state.gantz.open_heads.entry(head).or_default();
        let (_, graph, gv) = &mut state.heads[ix];
        let vm = &mut state.vms[ix];
        let text = gantz_egui::ops::cut_nodes(
            &state.env.registry,
            graph,
            vm,
            gv,
            &mut head_state.scene.interaction.selection,
            &nodes,
            &codec(),
        );
        if let Some(text) = text {
            ctx.copy_text(text);
        }
    }

    for (head, gantz_egui::DuplicateNodes(nodes)) in responses.take() {
        let Some((head, ix)) = tagged_head(state, head) else {
            continue;
        };
        let editing = match &head {
            gantz_ca::Head::Branch(name) => Some(name.to_string()),
            gantz_ca::Head::Commit(_) => None,
        };
        let head_state = state.gantz.open_heads.entry(head).or_default();
        let (_, graph, gv) = &mut state.heads[ix];
        let duplicated = gantz_egui::ops::duplicate_nodes(
            &mut state.env.registry,
            editing.as_deref(),
            graph,
            gv,
            head_state,
            &nodes,
            &codec(),
        );
        // Re-register the full root graph so the new nodes get their state
        // initialized. Idempotent for existing nodes; registration reifies
        // the graph transiently.
        if duplicated {
            let vm = &mut state.vms[ix];
            let get_node = |ca: &gantz_ca::ContentAddr| state.env.node(ca);
            match codec().reify_graph(graph) {
                Ok(g) => gantz_core::graph::register(&get_node, &g, &[], vm),
                Err(e) => log::error!("DuplicateNodes: cannot re-register the graph: {e}"),
            }
        }
    }

    for (
        head,
        gantz_egui::MergeHead {
            source,
            resolutions,
            auto_resolve,
        },
    ) in responses.take()
    {
        let Some((head, ix)) = tagged_head(state, head) else {
            continue;
        };
        let outcome = {
            let head_state = state.gantz.open_heads.entry(head.clone()).or_default();
            let (h, graph, view) = &mut state.heads[ix];
            gantz_egui::ops::merge_head(
                &mut state.env.registry,
                timestamp(),
                h,
                graph,
                &mut state.vms[ix],
                view,
                &mut head_state.scene.interaction.selection,
                &source,
                resolutions,
                auto_resolve,
            )
        };
        match outcome {
            gantz_egui::ops::MergeHeadOutcome::FastForward(target) => {
                navigate_head(ctx, state, &head, target);
            }
            gantz_egui::ops::MergeHeadOutcome::Merged { .. } => {
                // The op already committed (with both parents), so the commit
                // loop sees a clean graph; do its bookkeeping here: clear the
                // redo stack, recompile (this also re-registers, initializing
                // merged-in nodes' state), and bring referrers up to date.
                state.gantz.migrate_head(&head, &head, true);
                state.env.ensure_reified();
                recompile_heads(state);
                resync_and_refresh(state);
            }
            gantz_egui::ops::MergeHeadOutcome::Refused(reasons) => {
                // Defensive: the UI disables conflicted/blocked candidates.
                log::warn!(
                    "MergeHead: refused to merge '{source}': {}",
                    reasons.join("; ")
                );
            }
            gantz_egui::ops::MergeHeadOutcome::Noop => (),
        }
    }

    for (head, gantz_egui::Undo) in responses.take() {
        let Some((head, _)) = tagged_head(state, head) else {
            continue;
        };
        let parent =
            gantz_egui::ops::undo(&state.env.registry, &mut state.gantz.redo_stacks, &head);
        if let Some(parent) = parent {
            navigate_head(ctx, state, &head, parent);
        }
    }

    for (head, gantz_egui::Redo) in responses.take() {
        let Some((head, _)) = tagged_head(state, head) else {
            continue;
        };
        let redo_ca = gantz_egui::ops::redo(&mut state.gantz.redo_stacks, &head);
        if let Some(redo_ca) = redo_ca {
            navigate_head(ctx, state, &head, redo_ca);
        }
    }

    for (head, gantz_egui::ExportHead) in responses.take() {
        let Some(head) = head else { continue };
        let text =
            match gantz_egui::export::export_heads_sexpr(&state.env.registry, [&head], &codec()) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("ExportHead: failed to serialize: {e}");
                    continue;
                }
            };
        let default_name = gantz_egui::export::default_filename(&head);
        let ext = gantz_egui::export::FILE_EXTENSION;
        let dialog = rfd::AsyncFileDialog::new()
            .set_title("Export Graph")
            .set_file_name(&default_name)
            .add_filter("Gantz Export", &[ext]);
        if let Some(handle) = pollster::block_on(dialog.save_file()) {
            if let Err(e) = pollster::block_on(handle.write(text.as_bytes())) {
                log::error!("ExportHead: failed to write: {e}");
            } else {
                log::info!("Exported graph to {}", handle.file_name());
            }
        }
    }

    for (_, gantz_egui::ExportAllNamed) in responses.take() {
        let named_heads: Vec<gantz_ca::Head> = state
            .env
            .registry
            .heads()
            .map(|(name, _)| gantz_ca::Head::Branch(name.clone()))
            .collect();
        if named_heads.is_empty() {
            log::info!("ExportAllNamed: no named graphs to export");
            continue;
        }
        let text = match gantz_egui::export::export_heads_sexpr(
            &state.env.registry,
            named_heads.iter(),
            &codec(),
        ) {
            Ok(s) => s,
            Err(e) => {
                log::error!("ExportAllNamed: failed to serialize: {e}");
                continue;
            }
        };
        let ext = gantz_egui::export::FILE_EXTENSION;
        let dialog = rfd::AsyncFileDialog::new()
            .set_title("Export All Named Graphs")
            .set_file_name(&format!("gantz.{ext}"))
            .add_filter("Gantz Export", &[ext]);
        if let Some(handle) = pollster::block_on(dialog.save_file()) {
            if let Err(e) = pollster::block_on(handle.write(text.as_bytes())) {
                log::error!("ExportAllNamed: failed to write: {e}");
            } else {
                log::info!("Exported all named graphs to {}", handle.file_name());
            }
        }
    }

    // Any remaining payloads are unhandled - report rather than silently drop.
    for name in responses.type_names() {
        log::warn!("unhandled response payload: {name}");
    }
}

fn gui(ui: &mut egui::Ui, state: &mut State) -> gantz_egui::Responses {
    let compile_config = state.compile_config;
    let ctx = ui.ctx().clone();
    let response = egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show_inside(ui, |ui| {
            // Create the head access adapter.
            let mut access = DemoHeadAccess::new(
                &mut state.heads,
                &state.modules,
                &state.compile_errors,
                &state.diagnostics,
                &mut state.vms,
            );

            let no_base_names = Default::default();
            let codec = codec();
            let env = state.env.as_env(&codec);
            gantz_egui::widget::Gantz::new(&env, &no_base_names)
                .logger(state.logger.clone())
                .compile_config(compile_config)
                .show(&mut state.gantz, state.focused_head, &mut access, ui)
        })
        .inner;

    // Update focused head from the widget's response.
    state.focused_head = response.focused_head;

    // The given graph name was removed.
    if let Some(name) = response.graph_name_removed() {
        // Update any open heads that reference this name.
        for (head, _, _) in &mut state.heads {
            if let gantz_ca::Head::Branch(head_name) = &*head {
                if *head_name == name {
                    let commit_ca = state.env.registry.head_commit_ca(head).unwrap();
                    *head = gantz_ca::Head::Commit(commit_ca);
                }
            }
        }
        state.env.registry.remove_head(&name);
    }

    // Single click: replace the focused head with the selected one.
    if let Some(new_head) = response.graph_replaced() {
        replace_head(&ctx, state, new_head.clone());
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
        create_branch_from_head(&ctx, state, original_head, new_name.clone());
    }

    // Handle a graph description edit (keyed by the graph's name).
    if let Some((gantz_ca::Head::Branch(name), description)) = &response.description_changed {
        gantz_egui::section::set_description(
            &mut state.env.registry,
            name.clone(),
            description.clone(),
        );
    }

    // Handle import button click.
    if response.import() {
        let ext = gantz_egui::export::FILE_EXTENSION;
        let dialog = rfd::AsyncFileDialog::new()
            .set_title("Import")
            .add_filter("Gantz Export", &[ext]);
        if let Some(handle) = pollster::block_on(dialog.pick_file()) {
            let bytes = pollster::block_on(handle.read());
            import_bytes(state, bytes, true);
        }
    }

    // Handle file drops.
    for drop in response.file_drops {
        let open_head = drop.target == gantz_egui::widget::gantz::FileDropTarget::GraphScene;
        import_bytes(state, drop.bytes, open_head);
    }

    // Handle compile config change: recompile all open heads.
    if let Some(cfg) = response.compile_config {
        state.compile_config = cfg;
        recompile_heads(state);
    }

    response.responses
}

/// Import a `.gantz` file from raw bytes.
///
/// Deserializes the export, merges into the registry, and optionally opens
/// the unique root head.
fn import_bytes(state: &mut State, bytes: Vec<u8>, open_head: bool) {
    let imported = match gantz_egui::export::parse_export(&bytes, &codec()) {
        Ok(reg) => reg,
        Err(e) => {
            log::error!("Import: {e}");
            return;
        }
    };

    let root_name = if open_head {
        gantz_egui::export::unique_root_name(&imported)
    } else {
        None
    };

    let report = state.env.registry.merge(imported);
    state.env.ensure_reified();
    log::info!(
        "Imported: {} names added, {} replaced",
        report.heads_added.len(),
        report.heads_replaced.len(),
    );

    if let Some(name) = root_name {
        self::open_head(state, gantz_ca::Head::Branch(name));
    }
}

/// Open a head as a new tab, or focus it if already open.
///
/// This is only used when selecting from GraphSelect.
fn open_head(state: &mut State, new_head: gantz_ca::Head) {
    // Check if the head is already open.
    if let Some(ix) = state.heads.iter().position(|(h, _, _)| *h == new_head) {
        // Just focus the existing tab.
        state.focused_head = ix;
        return;
    }

    // Head is not open - add it as a new tab (the working graph is the
    // stored data cloned from the registry).
    let new_graph = state.env.head_data_graph(&new_head).unwrap();
    let view = gantz_egui::SceneView::default();

    state.heads.push((new_head.clone(), new_graph, view));
    state.focused_head = state.heads.len() - 1;

    // Initialise the VM from the reified cache at the committed address.
    let get_node = |ca: &gantz_ca::ContentAddr| state.env.node(ca);
    let graph = state.env.head_graph(&new_head).unwrap();
    let eps = push_pull_entrypoints(&get_node, graph);
    // A default engine keeps indices aligned on failure.
    let (vm, result) = match gantz_core::vm::init(&get_node, graph, &eps, &state.compile_config) {
        Ok((vm, module)) => (vm, Ok(module)),
        Err(e) => (Engine::new_base(), Err(e)),
    };
    let (module, error, diags) = compile_results(result);
    state.vms.push(vm);
    state.compile_errors.push(error);
    state.modules.push(module);
    state.diagnostics.push(diags);

    // Initialize GUI state for the new head.
    state.gantz.open_heads.entry(new_head).or_default();
}

/// Replace the focused head with a new head in-place.
///
/// If the new head is already open elsewhere, focuses that instead.
fn replace_head(ctx: &egui::Context, state: &mut State, new_head: gantz_ca::Head) {
    // If the new head is already open, just focus it.
    if let Some(ix) = state.heads.iter().position(|(h, _, _)| *h == new_head) {
        state.focused_head = ix;
        return;
    }

    let ix = state.focused_head;
    let old_head = state.heads[ix].0.clone();

    // Load the new graph (the stored data cloned from the registry).
    let new_graph = state.env.head_data_graph(&new_head).unwrap();
    let view = gantz_egui::SceneView::default();

    // Replace at the focused index.
    state.heads[ix] = (new_head.clone(), new_graph, view);

    // Reinitialize the VM from the reified cache at the committed address.
    let get_node = |ca: &gantz_ca::ContentAddr| state.env.node(ca);
    let graph = state.env.head_graph(&new_head).unwrap();
    let eps = push_pull_entrypoints(&get_node, graph);
    let result = match gantz_core::vm::init(&get_node, graph, &eps, &state.compile_config) {
        Ok((new_vm, module)) => {
            state.vms[ix] = new_vm;
            Ok(module)
        }
        Err(e) => Err(e),
    };
    let (module, error, diags) = compile_results(result);
    state.compile_errors[ix] = error;
    state.modules[ix] = module;
    state.diagnostics[ix] = diags;

    // Update the graph pane to show the new head.
    gantz_egui::widget::update_graph_pane_head(ctx, &old_head, &new_head);
    state.gantz.migrate_head(&old_head, &new_head, false);
    // Ensure an entry exists even if there was no old state to migrate.
    state.gantz.open_heads.entry(new_head).or_default();
}

/// Move a head to a target commit.
///
/// Branch heads update the registry name mapping and refresh in-place.
/// Commit heads replace the focused head entirely.
fn navigate_head(
    ctx: &egui::Context,
    state: &mut State,
    head: &gantz_ca::Head,
    target: gantz_ca::CommitAddr,
) {
    match head {
        gantz_ca::Head::Commit(_) => {
            replace_head(ctx, state, gantz_ca::Head::Commit(target));
        }
        gantz_ca::Head::Branch(name) => {
            state.env.registry.set_head(name.clone(), target);
            refresh_branch_head(state);
        }
    }
}

/// Refresh the focused branch head after its commit pointer has been moved.
///
/// Reloads the graph, view, and VM from the registry for the focused head.
fn refresh_branch_head(state: &mut State) {
    let ix = state.focused_head;
    let (ref head, ref mut graph, ref mut view) = state.heads[ix];
    *graph = state.env.head_data_graph(head).unwrap();
    *view = gantz_egui::SceneView::default();
    let get_node = |ca: &gantz_ca::ContentAddr| state.env.node(ca);
    let typed = state.env.head_graph(head).unwrap();
    let eps = push_pull_entrypoints(&get_node, typed);
    let result = match gantz_core::vm::init(&get_node, typed, &eps, &state.compile_config) {
        Ok((new_vm, module)) => {
            state.vms[ix] = new_vm;
            Ok(module)
        }
        Err(e) => Err(e),
    };
    let (module, error, diags) = compile_results(result);
    state.compile_errors[ix] = error;
    state.modules[ix] = module;
    state.diagnostics[ix] = diags;
}

/// Reload any open head whose commit moved (to its new registry graph) and
/// recompile. A no-op when there are no moves.
fn apply_moves(state: &mut State, moves: &[gantz_egui::sync::Moved]) {
    if moves.is_empty() {
        return;
    }
    // The moves committed fresh graphs: reify them before lookups.
    state.env.ensure_reified();
    for m in moves {
        let Some(new_graph) = state
            .env
            .registry
            .commits()
            .get(&m.new_commit)
            .and_then(|c| state.env.registry.graph(&c.graph))
            .cloned()
        else {
            continue;
        };
        for (head, graph, _) in state.heads.iter_mut() {
            if matches!(head, gantz_ca::Head::Branch(name) if *name == m.name) {
                *graph = new_graph;
                break;
            }
        }
    }
    recompile_heads(state);
}

/// After committing edited heads, bring referrers up to date: resync all
/// sync-enabled `NamedRef`s, reload any open head whose commit moved, and
/// recompile. This is how editing a nested graph propagates to its parents.
fn resync_and_refresh(state: &mut State) {
    let moves = gantz_egui::sync::resync(&mut state.env.registry, timestamp());
    apply_moves(state, &moves);
}

/// Recompile every open head's graph into its existing VM (no commit).
///
/// Used when the compile config changes: the graph content is unchanged, and
/// compiling into the existing VM preserves node state.
fn recompile_heads(state: &mut State) {
    for ix in 0..state.heads.len() {
        recompile_head(state, ix);
    }
}

/// Recompile one head's graph - read from the reified cache at its committed
/// address - into its existing VM (no commit), preserving node state.
fn recompile_head(state: &mut State, ix: usize) {
    let head = state.heads[ix].0.clone();
    let vm = &mut state.vms[ix];
    let env = &state.env;
    let get_node = |ca: &gantz_ca::ContentAddr| env.node(ca);
    let Some(graph) = env.head_graph(&head) else {
        log::error!("recompile: no reified graph for head {head}");
        return;
    };
    gantz_core::graph::register(&get_node, graph, &[], vm);
    let eps = push_pull_entrypoints(&get_node, graph);
    let result = gantz_core::vm::compile(&get_node, graph, vm, &eps, &state.compile_config);
    let (module, error, diags) = compile_results(result);
    state.compile_errors[ix] = error;
    state.modules[ix] = module;
    state.diagnostics[ix] = diags;
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
        state.compile_errors.remove(ix);
        state.modules.remove(ix);
        state.diagnostics.remove(ix);
        state.gantz.open_heads.remove(head);
        state.gantz.redo_stacks.remove(head);

        // Update focused_head to remain valid.
        if ix <= state.focused_head {
            state.focused_head = state.focused_head.saturating_sub(1);
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
    let Some(commit_ca) = state.env.registry.head_commit_ca(original_head) else {
        log::error!("Failed to get commit address for head: {:?}", original_head);
        return;
    };

    // Create a new commit pointing to the same graph so the new branch gets
    // its own independent `CommitAddr` (and therefore its own view/layout).
    let graph_addr = state.env.registry.commits()[&commit_ca].graph;
    let new_commit_ca =
        state
            .env
            .registry
            .commit_graph(timestamp(), Some(commit_ca), graph_addr, || {
                unreachable!("graph already exists in registry")
            });

    // Insert the new branch name pointing to the fresh commit.
    let new_name: gantz_ca::Name = new_name.parse().expect("infallible");
    state.env.registry.set_head(new_name.clone(), new_commit_ca);

    // Inherit the original graph's description, if any.
    if let gantz_ca::Head::Branch(orig_name) = original_head {
        if let Some(desc) = gantz_egui::section::description(&state.env.registry, orig_name) {
            gantz_egui::section::set_description(&mut state.env.registry, new_name.clone(), desc);
        }
    }

    // Find the index of the original head and replace it.
    let new_head = gantz_ca::Head::Branch(new_name);
    if let Some(ix) = state.heads.iter().position(|(h, _, _)| h == original_head) {
        let old_head = state.heads[ix].0.clone();
        state.heads[ix].0 = new_head.clone();

        // Update the graph pane to show the new head.
        gantz_egui::widget::update_graph_pane_head(ctx, &old_head, &new_head);
        state.gantz.migrate_head(&old_head, &new_head, false);
    }

    // Give the fork independent nested children, then (when a nested graph was
    // renamed to a root name) repoint its parent's references to it.
    if let (gantz_ca::Head::Branch(old), gantz_ca::Head::Branch(new)) = (original_head, &new_head) {
        let ts = timestamp();
        let mut moves = gantz_egui::sync::fork_nested(&mut state.env.registry, ts, old, new);
        moves.extend(gantz_egui::sync::promote_nested(
            &mut state.env.registry,
            ts,
            old,
            new,
        ));
        apply_moves(state, &moves);
    }
}
