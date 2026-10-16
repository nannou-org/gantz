//! A minimal `egui` host for one `gantz` graph.
//!
//! It shows what a host gives the top-level [`Gantz`] widget: a node codec,
//! an environment over the registry, [`HeadAccess`] over the open graph, and
//! handling for the payloads the widget emits. It keeps one graph in memory,
//! with no persistence. `bevy_gantz_egui` is the complete host, with
//! persistence, many open graphs, history and collaboration.
//!
//! [`Gantz`]: gantz_egui::widget::Gantz

use eframe::egui;
use gantz_ca::{ContentAddr, DataGraph, Head, Registry};
use gantz_core::steel::steel_vm::engine::Engine;
use gantz_egui::node::{DynNode, NodeCodec};
use gantz_egui::{HeadAccess, HeadDataMut};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let name = "g a n t z";
    eframe::run_native(name, options, Box::new(|_cc| Ok(Box::new(App::new()))))
}

/// The `.gantz` keyword sugar for the demo's node set.
struct NodeSet;

struct App {
    codec: NodeCodec,
    registry: Registry,
    builtins: gantz_core::Builtins,
    /// One reified instance per builtin, for the node palette.
    instances: gantz_egui::node::UiBuiltins,
    /// Reified registry graphs. They serve graph references, which this demo
    /// does not create, so this stays empty.
    graphs: gantz_core::data::ReifiedGraphs<DynNode>,
    open: Open,
    gantz: gantz_egui::widget::GantzState,
    logger: gantz_egui::widget::log_view::Logger,
    config: gantz_core::compile::Config,
}

/// The open graph and everything compiled from it.
struct Open {
    /// A branch head, so commits keep the head and its per-head UI state.
    head: Head,
    /// The working graph in its stored data form.
    graph: DataGraph,
    view: gantz_egui::SceneView,
    vm: Engine,
    nodes: gantz_egui::node::NodeInstances,
    module: Option<gantz_core::vm::Compiled>,
    compile_error: Option<String>,
    diagnostics: Vec<gantz_core::Diagnostic>,
}

impl App {
    fn new() -> Self {
        let logger = gantz_egui::widget::log_view::Logger::default();
        log::set_boxed_logger(Box::new(logger.clone())).unwrap();
        log::set_max_level(log::LevelFilter::Info);

        let codec = codec();
        let builtins = builtins();
        let (instances, errs) = gantz_egui::node::UiBuiltins::reify(&builtins, &codec);
        for e in errs {
            log::error!("failed to reify a builtin: {e}");
        }

        // Start from an empty graph committed to the `demo` branch.
        let mut registry = Registry::default();
        let graph = DataGraph::default();
        let name: gantz_ca::Name = "demo".parse().expect("a valid name");
        let addr = gantz_ca::graph_addr(&graph);
        registry.commit_graph_to_name(timestamp(), addr, || graph.clone(), &name);

        let mut open = Open {
            head: Head::Branch(name),
            graph,
            view: Default::default(),
            vm: gantz_core::vm::new_engine(&[]),
            nodes: Default::default(),
            module: None,
            compile_error: None,
            diagnostics: vec![],
        };
        let config = gantz_core::compile::Config::default();
        compile(&codec, &config, &mut open);

        App {
            codec,
            registry,
            builtins,
            instances,
            graphs: Default::default(),
            open,
            gantz: Default::default(),
            logger,
            config,
        }
    }
}

impl gantz_format::NodeSugar for NodeSet {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![
            &gantz_format::CoreSugar,
            &gantz_io::IoSugar,
            &gantz_egui::EguiSugar,
        ])
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let env = gantz_egui::Env {
            registry: &self.registry,
            builtins: &self.builtins,
            codec: &self.codec,
            graphs: &self.graphs,
            instances: &self.instances,
        };
        let base_names = Default::default();
        let response = egui::CentralPanel::default()
            .frame(egui::Frame::default())
            .show_inside(ui, |ui| {
                gantz_egui::widget::Gantz::new(&env, &base_names)
                    .logger(self.logger.clone())
                    .compile_config(self.config)
                    .show(&mut self.gantz, 0, &mut self.open, ui)
            })
            .inner;
        let mut responses = response.responses;

        // Edit the graph first, so evaluations below run the latest graph.
        let head_state = self
            .gantz
            .open_heads
            .entry(self.open.head.clone())
            .or_default();
        let editing = Some("demo");
        let builtins = &self.builtins;
        let new_node = |node_type: &str| builtins.node_data(node_type).cloned();
        for (_, create) in responses.take::<gantz_egui::CreateNode>() {
            let open = &mut self.open;
            gantz_egui::ops::create_node(
                &self.registry,
                editing,
                &self.codec,
                &no_refs,
                new_node,
                &mut open.graph,
                &mut open.view,
                head_state,
                &mut open.vm,
                create,
            );
        }
        for (_, replace) in responses.take::<gantz_egui::ReplaceNode>() {
            let open = &mut self.open;
            gantz_egui::ops::replace_node(
                &self.registry,
                editing,
                &self.codec,
                &no_refs,
                new_node,
                &mut open.graph,
                head_state,
                &mut open.vm,
                replace,
            );
        }
        for (_, drop) in responses.take::<gantz_egui::DropUnknownData>() {
            gantz_egui::ops::drop_unknown_data(&self.codec, &mut self.open.graph, drop);
        }

        // Commit and recompile when the working graph changed.
        let addr = gantz_ca::graph_addr(&self.open.graph);
        if self.registry.head_commit(&self.open.head).map(|c| c.graph) != Some(addr) {
            let graph = self.open.graph.clone();
            let head = &mut self.open.head;
            self.registry
                .commit_graph_to_head(timestamp(), addr, || graph, head);
            compile(&self.codec, &self.config, &mut self.open);
        }

        for (_, gantz_egui::EvalEntry(ep)) in responses.take() {
            eval(&mut self.open, &ep);
        }

        // VM state writes matter only to a collaborative session.
        responses.take::<gantz_egui::StateWritten>();
        for name in responses.type_names() {
            log::warn!("the demo does not handle `{name}`");
        }
    }
}

impl HeadAccess for Open {
    fn heads(&self) -> &[Head] {
        std::slice::from_ref(&self.head)
    }

    fn with_head_mut<R>(&mut self, head: &Head, f: impl FnOnce(HeadDataMut<'_>) -> R) -> Option<R> {
        (*head == self.head).then(|| {
            f(HeadDataMut {
                graph: &mut self.graph,
                view: &mut self.view,
                vm: &mut self.vm,
                instances: &mut self.nodes,
            })
        })
    }

    fn module(&self, _head: &Head) -> Option<&gantz_core::vm::Compiled> {
        self.module.as_ref()
    }

    fn compile_error(&self, _head: &Head) -> Option<&str> {
        self.compile_error.as_deref()
    }

    fn diagnostics(&self, _head: &Head) -> &[gantz_core::Diagnostic] {
        &self.diagnostics
    }
}

/// The value-level codec for the demo's node set. This is the node set's
/// manifest.
fn codec() -> NodeCodec {
    gantz_egui::ui_node_codec! {
        NodeSet {
            gantz_core::node::Expr,
            gantz_egui::node::Bang,
            gantz_egui::node::Comment,
            gantz_egui::node::Inspect,
            gantz_egui::node::Number,
            gantz_egui::node::Plot,
            gantz_io::Log,
        }
    }
}

/// The node palette, as data. Each node type is in the codec.
fn builtins() -> gantz_core::Builtins {
    use gantz_core::Builtin;
    gantz_core::Builtins::from_specs([
        Builtin::new("bang", &gantz_egui::node::Bang::default()),
        Builtin::new("comment", &gantz_egui::node::Comment::default()),
        Builtin::new("expr", &gantz_core::node::Expr::new("()").unwrap()),
        Builtin::new("inspect", &gantz_egui::node::Inspect::default()),
        Builtin::new("log", &gantz_io::Log::default()),
        Builtin::new("number", &gantz_egui::node::Number::default()),
        Builtin::new("plot", &gantz_egui::node::Plot::default()),
    ])
}

/// Node lookup for graph references. The demo creates none.
fn no_refs(_: &ContentAddr) -> Option<&'static dyn gantz_core::Node> {
    None
}

/// Register the open graph's nodes and compile it into its VM, keeping node
/// state across edits.
fn compile(codec: &NodeCodec, config: &gantz_core::compile::Config, open: &mut Open) {
    let graph = match codec.reify_graph(&open.graph) {
        Ok(graph) => graph,
        Err(e) => {
            log::error!("failed to reify the graph: {e}");
            return;
        }
    };
    gantz_core::graph::register(&no_refs, &graph, &[], &mut open.vm);
    let eps = gantz_core::compile::push_pull_entrypoints(&no_refs, &graph);
    match gantz_core::vm::compile(&no_refs, &graph, &mut open.vm, &eps, config) {
        Ok(module) => {
            open.module = Some(module);
            open.compile_error = None;
            open.diagnostics.clear();
        }
        Err(e) => {
            let error = gantz_core::vm::error_chain(&e);
            log::error!("failed to compile the graph: {error}");
            open.diagnostics = gantz_core::diagnostic::from_compile_error(&e);
            open.compile_error = Some(error);
            open.module = e.into_module();
        }
    }
}

/// Run an entrypoint, such as a pressed bang, and show any runtime error.
fn eval(open: &mut Open, ep: &gantz_core::compile::Entrypoint) {
    let fn_name = gantz_core::compile::entry_fn_name(&ep.id());
    let result = open.vm.call_function_by_name_with_args(&fn_name, vec![]);
    let runtime = gantz_core::diagnostic::Severity::Runtime;
    open.diagnostics.retain(|d| d.severity != runtime);
    if let Err(e) = result {
        if let Some(module) = &open.module {
            let diag = gantz_core::diagnostic::from_eval_error(&e, &open.vm, module);
            open.diagnostics.push(diag);
        }
        log::error!("{e}");
    }
}

/// The time of a new commit.
fn timestamp() -> std::time::Duration {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
}
