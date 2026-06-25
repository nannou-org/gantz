//! Tests for the bang node's trigger input: a `Bang` ignores its input value
//! and emits a bang (`'()`) downstream when pushed.

use gantz_core::{
    Edge, Node,
    compile::{EvalKind, entry_fn_name, push_pull_entrypoints},
    node::{self, WithPushEval},
};
use std::fmt::Debug;

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A no-op node lookup function for tests that don't need it.
fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

// Firing a value into a bang's trigger input emits `'()` downstream. The
// downstream `check` asserts it received an empty list, so a successful fire
// proves the bang ignored its input and emitted a bang.
#[test]
fn bang_trigger_input_emits_bang() {
    let mut g = petgraph::graph::DiGraph::new();
    let push =
        g.add_node(Box::new(node::expr("'()").unwrap().with_push_eval()) as Box<dyn DebugNode>);
    // A constant value, fired by the push via its own trigger input.
    let val = g.add_node(Box::new(node::expr("42").unwrap()) as Box<_>);
    // The bang ignores `val`'s output and emits `'()`.
    let bang = g.add_node(Box::new(gantz_std::Bang::default()) as Box<_>);
    // Assert the bang emitted an empty list.
    let check = g.add_node(Box::new(node::expr("(assert! (equal? $b '()))").unwrap()) as Box<_>);
    g.add_edge(push, val, Edge::from((0, 0)));
    g.add_edge(val, bang, Edge::from((0, 0)));
    g.add_edge(bang, check, Edge::from((0, 0)));

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
        .expect("firing bang trigger errored");
}
