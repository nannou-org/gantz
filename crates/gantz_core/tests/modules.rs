//! Tests for per-node Steel module requirements: `(require ...)` emission
//! from `Node::required_modules` declarations, and evaluation against the
//! modules registered by `vm::new_engine`.

use gantz_core::{
    Edge,
    compile::{Config, ModuleError, SourceMap, entry_fn_name, push_pull_entrypoints},
    node::{self, ExprCtx, ExprResult, MetaCtx, Node, RegCtx, WithPushEval},
};
use std::fmt::Debug;

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A no-op node lookup function for tests that don't need it.
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
// `(require ...)`, and the SourceMap still resolves every node def around
// the (nameless) require form.
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

    // One def per module expression. The require's def carries no name;
    // every other def remains a recognised define.
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

// A declaring node outside every eval path still gets its require: steel
// resolves the free identifiers of every emitted fn at definition time, so
// the module bindings must exist even for fns nothing calls. Pinned under
// both configs (`emit_all_node_fns` emits the orphan's fn unconditionally).
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
        // No edges: the orphan is outside every eval path.
        let _orphan = g
            .add_node(
                Box::new(node_requires("(unwrap-or (Some $x) 0)", &["gantz/option"])) as Box<_>,
            );

        let eps = push_pull_entrypoints(&no_lookup, &g);
        let module = gantz_core::compile::module(&no_lookup, &g, &eps, &config).unwrap();
        let src = gantz_core::vm::fmt_module(&module);
        assert_eq!(src.matches("gantz/option").count(), 1);

        // The module must also run: `vm::init` registers `gantz/option` so
        // the emitted require resolves.
        gantz_core::vm::init(&no_lookup, &g, &eps, &config).unwrap();
    }
}

// End-to-end: a node whose expr uses a `gantz/option` binding compiles and
// evaluates through `vm::init`.
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

// The `Expr` node's own `requires` field drives emission end-to-end: an
// expr using a `gantz/option` binding compiles and evaluates via `vm::init`
// with no custom node type involved.
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
