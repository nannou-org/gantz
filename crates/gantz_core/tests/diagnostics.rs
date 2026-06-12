//! Tests extracting structured diagnostics from compile and runtime errors.

use gantz_core::{
    Edge,
    compile::push_pull_entrypoints,
    diagnostic::{self, Severity},
    node::{self, GraphNode, Node, WithPushEval},
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

// An edge referencing an out-of-range output index yields a compile
// diagnostic carrying the offending node's full path and output index,
// including inside a nested graph.
#[test]
fn invalid_edge_diagnostic() {
    let mut ga = GraphNode::default();
    let inlet = ga.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    // `(+ $l 1)` has one output; the edge below leaves from output 5.
    let inc = ga.add_node(Box::new(node::expr("(+ $l 1)").unwrap()) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet) as Box<_>);
    ga.add_edge(inlet, inc, Edge::from((0, 0)));
    ga.add_edge(inc, outlet, Edge::from((5, 0)));

    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let nested = g.add_node(Box::new(ga) as Box<_>);
    g.add_edge(push, nested, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let err = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap_err();
    let diags = diagnostic::from_module_error(&err);

    let diag = diags
        .iter()
        .find(|d| d.path == [nested.index(), inc.index()])
        .unwrap_or_else(|| panic!("no diagnostic for the inc node in {diags:?}"));
    assert_eq!(diag.outputs, vec![5]);
    assert_eq!(diag.severity, Severity::Compile);
    assert!(diag.message.contains("invalid output index"));
}

// A runtime steel error yields a diagnostic with the erroring node's path
// and a span into the module source covering the failing expression.
#[test]
fn runtime_error_diagnostic() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let boom = g.add_node(Box::new(node::expr("(begin $push (car '()))").unwrap()) as Box<_>);
    g.add_edge(push, boom, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let (mut vm, compiled) =
        gantz_core::vm::init(&no_lookup, &g, &eps, &Default::default()).unwrap();
    let ep = &eps[0];
    let fn_name = gantz_core::compile::entry_fn_name(&ep.id());
    let err = vm
        .call_function_by_name_with_args(&fn_name, vec![])
        .unwrap_err();

    let diag = diagnostic::from_eval_error(&err, &vm, &compiled);
    assert_eq!(diag.path, vec![boom.index()]);
    assert_eq!(diag.severity, Severity::Runtime);
    let span = diag.span.expect("runtime error carries a module span");
    assert_eq!(&compiled.src[span], "car");
}
