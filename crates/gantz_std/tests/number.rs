//! Tests for the number node's optional min/max bounds: a bounded `Number`
//! clamps every value it stores, including a value pushed into its input.

use gantz_core::{
    Edge, Node,
    compile::{EvalKind, entry_fn_name, push_pull_entrypoints},
    node::{self, WithPushEval},
};
use gantz_std::number::Number;
use std::fmt::Debug;

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A no-op node lookup function for tests that don't need it.
fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

fn bounded(min: Option<f64>, max: Option<f64>) -> Number {
    let mut n = Number::default();
    n.set_min(min);
    n.set_max(max);
    n
}

// Push `value` into a `Number` configured with `min`/`max`, then assert against
// the value it forwards downstream via `check` (an expression that panics if its
// assertion fails). A successful fire proves the bounds were applied.
fn assert_forwards(value: &str, min: Option<f64>, max: Option<f64>, check: &str) {
    let mut g = petgraph::graph::DiGraph::new();
    let push =
        g.add_node(Box::new(node::expr(value).unwrap().with_push_eval()) as Box<dyn DebugNode>);
    let num = g.add_node(Box::new(bounded(min, max)) as Box<_>);
    let check = g.add_node(Box::new(node::expr(check).unwrap()) as Box<_>);
    g.add_edge(push, num, Edge::from((0, 0)));
    g.add_edge(num, check, Edge::from((0, 0)));

    let config = gantz_core::compile::Config::default();
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let (mut vm, _compiled) = gantz_core::vm::init(&no_lookup, &g, &eps, &config)
        .unwrap_or_else(|e| panic!("init: {}", gantz_core::vm::error_chain(&e)));

    let ep = eps
        .iter()
        .find(|ep| {
            ep.0.iter()
                .any(|s| s.kind == EvalKind::Push && s.path == [push.index()])
        })
        .expect("push entrypoint");
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .expect("firing the number errored");
}

#[test]
fn clamps_above_max() {
    assert_forwards("150", Some(0.0), Some(100.0), "(assert! (= $n 100))");
}

#[test]
fn clamps_below_min() {
    assert_forwards("-50", Some(0.0), Some(100.0), "(assert! (= $n 0))");
}

#[test]
fn passes_value_in_range() {
    assert_forwards("42", Some(0.0), Some(100.0), "(assert! (= $n 42))");
}

#[test]
fn lower_bound_only() {
    assert_forwards("-3", Some(0.0), None, "(assert! (= $n 0))");
}

#[test]
fn upper_bound_only() {
    assert_forwards("500", None, Some(10.0), "(assert! (= $n 10))");
}

#[test]
fn unbounded_passes_through() {
    assert_forwards("999", None, None, "(assert! (= $n 999))");
}
