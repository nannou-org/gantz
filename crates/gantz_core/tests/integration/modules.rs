//! Tests for per-node Steel module requirements. They cover `(require ...)`
//! emission from `Node::required_modules` declarations and evaluation against
//! the modules registered by `vm::new_engine`.

use gantz_core::{
    Edge,
    compile::{Config, ModuleError, SourceMap, entry_fn_name, push_pull_entrypoints},
    node::{self, ExprCtx, ExprResult, MetaCtx, Node, RegCtx, WithPushEval},
    steel::steel_vm::{builtin::BuiltInModule, register_fn::RegisterFn},
    vm::SteelModule,
};
use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

fn node_push() -> node::Push<node::Expr> {
    node::expr("'()").unwrap().with_push_eval()
}

/// An expr node declaring a dependency on the named Steel modules.
#[derive(Debug)]
struct RequiresNode {
    expr: node::Expr,
    modules: Vec<String>,
}

fn node_requires(src: &str, modules: &[&str]) -> RequiresNode {
    RequiresNode {
        expr: node::expr(src).unwrap(),
        modules: modules.iter().map(|m| m.to_string()).collect(),
    }
}

impl Node for RequiresNode {
    fn n_inputs(&self, ctx: MetaCtx) -> usize {
        self.expr.n_inputs(ctx)
    }

    fn n_outputs(&self, ctx: MetaCtx) -> usize {
        self.expr.n_outputs(ctx)
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        self.expr.expr(ctx)
    }

    fn stateful(&self, ctx: MetaCtx) -> bool {
        self.expr.stateful(ctx)
    }

    fn register(&self, ctx: RegCtx<'_, '_>) {
        self.expr.register(ctx)
    }

    fn required_modules(&self, _ctx: MetaCtx) -> Vec<String> {
        self.modules.clone()
    }
}

// A graph with no module declarations emits no `(require ...)` forms.
#[test]
fn no_requires_without_declarations() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int = g.add_node(Box::new(node::expr("(begin $push 6)").unwrap()) as Box<_>);
    g.add_edge(push, int, Edge::from((0, 0)));
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Config::default()).unwrap();
    let src = gantz_core::vm::fmt_module(&module);
    assert!(!src.contains("(require"));
}

// Multiple declarations of the same module emit exactly one leading
// `(require ...)`. The SourceMap still resolves every node def around the
// nameless require form.
#[test]
fn requires_deduped_and_lead_the_module() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let a = g.add_node(Box::new(node_requires(
        "(begin $push (unwrap-or (Some 1) 0))",
        &["gantz/option"],
    )) as Box<_>);
    let b =
        g.add_node(Box::new(node_requires("(unwrap-or (Some $x) 0)", &["gantz/option"])) as Box<_>);
    g.add_edge(push, a, Edge::from((0, 0)));
    g.add_edge(a, b, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Config::default()).unwrap();
    let src = gantz_core::vm::fmt_module(&module);
    assert_eq!(src.matches("gantz/option").count(), 1);
    assert!(src.starts_with("(require"));

    // One def per module expression. The require's def carries no name.
    // Every other def remains a recognised define.
    let map = SourceMap::parse(&src);
    assert_eq!(map.defs().len(), module.len());
    let named = map.defs().iter().filter(|d| d.name.is_some()).count();
    assert_eq!(named, module.len() - 1);

    // Node defs and refs still resolve to their paths.
    for n in [a, b] {
        let path = vec![n.index()];
        let spans = map.node_spans(&path);
        assert!(!spans.defs.is_empty(), "no defs for {path:?}");
        for range in spans.defs.iter().chain(&spans.refs) {
            assert_eq!(map.node_at(range.clone()), Some(path.clone()));
        }
    }
}

// A declaring node outside every eval path still gets its require. Steel
// resolves the free identifiers of every emitted fn at definition time. So
// the module bindings must exist even for fns nothing calls. Both configs
// are pinned since `emit_all_node_fns` emits the orphan's fn unconditionally.
#[test]
fn off_eval_path_node_still_requires_its_module() {
    for config in [
        Config::default(),
        Config {
            emit_all_node_fns: true,
            ..Config::default()
        },
    ] {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node::expr("(begin $push 6)").unwrap()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        // The orphan has no edges, so it is outside every eval path.
        let _orphan = g
            .add_node(
                Box::new(node_requires("(unwrap-or (Some $x) 0)", &["gantz/option"])) as Box<_>,
            );

        let eps = push_pull_entrypoints(&no_lookup, &g);
        let module = gantz_core::compile::module(&no_lookup, &g, &eps, &config).unwrap();
        let src = gantz_core::vm::fmt_module(&module);
        assert_eq!(src.matches("gantz/option").count(), 1);

        // The module must also run. `vm::init` registers `gantz/option` so
        // the emitted require resolves.
        gantz_core::vm::init(&no_lookup, &g, &eps, &config).unwrap();
    }
}

// A node whose expr uses a `gantz/option` binding compiles and evaluates
// through `vm::init`.
#[test]
fn required_module_bindings_evaluate() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let unwrap = g.add_node(Box::new(node_requires(
        "(begin $push (unwrap-or (Some 40) 2))",
        &["gantz/option"],
    )) as Box<_>);
    let check = g.add_node(Box::new(node::expr("(assert! (equal? $x 40))").unwrap()) as Box<_>);
    g.add_edge(push, unwrap, Edge::from((0, 0)));
    g.add_edge(unwrap, check, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let (mut vm, _compiled) =
        gantz_core::vm::init(&no_lookup, &g, &eps, &Config::default()).unwrap();
    let fn_name = entry_fn_name(&eps[0].id());
    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();
}

// The `Expr` node's own `requires` field drives emission. An expr that uses
// a `gantz/option` binding compiles and evaluates through `vm::init` with no
// custom node type.
#[test]
fn expr_requires_field_evaluates() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let unwrap = g.add_node(Box::new(
        node::expr("(begin $push (unwrap-or (None) 42))")
            .unwrap()
            .with_requires(["gantz/option"]),
    ) as Box<_>);
    let check = g.add_node(Box::new(node::expr("(assert! (equal? $x 42))").unwrap()) as Box<_>);
    g.add_edge(push, unwrap, Edge::from((0, 0)));
    g.add_edge(unwrap, check, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let (mut vm, compiled) =
        gantz_core::vm::init(&no_lookup, &g, &eps, &Config::default()).unwrap();
    assert!(compiled.src.starts_with("(require"));
    let fn_name = entry_fn_name(&eps[0].id());
    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();
}

/// The builtin module behind [`NATIVE`].
fn native_builtin() -> BuiltInModule {
    let mut module = BuiltInModule::new("#%test/native");
    module.register_fn("native-double", |x: isize| x * 2);
    module
}

/// A source module that provides a Rust fn from its builtin module.
const NATIVE: SteelModule = SteelModule::new(
    "test/native",
    "(require-builtin #%test/native) (provide native-double)",
)
.with_builtin(native_builtin);

/// Constructions of the builtin behind [`COUNTED`]. Only
/// `duplicate_module_names_register_once` uses it, so parallel tests do
/// not race on it.
static COUNTED_BUILDS: AtomicUsize = AtomicUsize::new(0);

fn counted_builtin() -> BuiltInModule {
    COUNTED_BUILDS.fetch_add(1, Ordering::SeqCst);
    let mut module = BuiltInModule::new("#%test/counted");
    module.register_fn("counted-triple", |x: isize| x * 3);
    module
}

const COUNTED: SteelModule = SteelModule::new(
    "test/counted",
    "(require-builtin #%test/counted) (provide counted-triple)",
)
.with_builtin(counted_builtin);

// A node that requires a module carrying a builtin can call the Rust fn
// that the module's source provides.
#[test]
fn builtin_module_fns_evaluate() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let double = g.add_node(Box::new(node_requires(
        "(begin $push (native-double 21))",
        &["test/native"],
    )) as Box<_>);
    let check = g.add_node(Box::new(node::expr("(assert! (equal? $x 42))").unwrap()) as Box<_>);
    g.add_edge(push, double, Edge::from((0, 0)));
    g.add_edge(double, check, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let (mut vm, _compiled) =
        gantz_core::vm::init_with_modules(&no_lookup, &g, &eps, &Config::default(), &[NATIVE])
            .unwrap();
    let fn_name = entry_fn_name(&eps[0].id());
    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();
}

// A module listed twice, or listed again after the core modules, is
// registered once. Its builtin is constructed once.
#[test]
fn duplicate_module_names_register_once() {
    let core = gantz_core::vm::modules()[0];
    let mut vm = gantz_core::vm::new_engine(&[COUNTED, core, COUNTED]);
    assert_eq!(COUNTED_BUILDS.load(Ordering::SeqCst), 1);
    let vals = vm
        .run("(require \"test/counted\") (counted-triple 4)".to_string())
        .unwrap();
    assert_eq!(vals.last(), Some(&gantz_core::steel::SteelVal::IntV(12)));
}

// A declared name that cannot be emitted as a `(require ...)` string
// literal surfaces as a compile error rather than emitting broken Steel.
#[test]
fn invalid_module_name_errors() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let bad = g.add_node(Box::new(node_requires("(begin $push 1)", &["bad\"name"])) as Box<_>);
    g.add_edge(push, bad, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let err = gantz_core::compile::module(&no_lookup, &g, &eps, &Config::default()).unwrap_err();
    assert!(matches!(err, ModuleError::InvalidModuleName { .. }));
}
