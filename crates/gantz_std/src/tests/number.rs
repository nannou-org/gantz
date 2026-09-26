//! Tests for the number node's optional min and max bounds. A bounded
//! `Number` clamps every value it stores, including a value pushed into its
//! input.

use super::{DebugNode, no_lookup};
use crate::number::Number;
use gantz_core::{
    Edge,
    compile::{EvalKind, entry_fn_name, push_pull_entrypoints},
    node::{self, WithPushEval},
};

fn bounded(min: Option<f64>, max: Option<f64>) -> Number {
    let mut n = Number::default();
    n.set_min(min);
    n.set_max(max);
    n
}

// Push `value` into a `Number` configured with `min` and `max`. The `check`
// expression asserts on the value forwarded downstream and panics on failure.
// A successful fire proves the bounds were applied.
fn assert_forwards(case: &str, value: &str, min: Option<f64>, max: Option<f64>, check: &str) {
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
        .unwrap_or_else(|e| panic!("{case}: init: {}", gantz_core::vm::error_chain(&e)));

    let ep = eps
        .iter()
        .find(|ep| {
            ep.0.iter()
                .any(|s| s.kind == EvalKind::Push && s.path == [push.index()])
        })
        .expect("push entrypoint");
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap_or_else(|e| panic!("{case}: firing the number errored: {e:?}"));
}

#[test]
fn bounds_clamp_forwarded_values() {
    let rows: &[(&str, &str, Option<f64>, Option<f64>, &str)] = &[
        (
            "clamps_above_max",
            "150",
            Some(0.0),
            Some(100.0),
            "(assert! (= $n 100))",
        ),
        (
            "clamps_below_min",
            "-50",
            Some(0.0),
            Some(100.0),
            "(assert! (= $n 0))",
        ),
        (
            "passes_value_in_range",
            "42",
            Some(0.0),
            Some(100.0),
            "(assert! (= $n 42))",
        ),
        (
            "lower_bound_only",
            "-3",
            Some(0.0),
            None,
            "(assert! (= $n 0))",
        ),
        (
            "upper_bound_only",
            "500",
            None,
            Some(10.0),
            "(assert! (= $n 10))",
        ),
        (
            "unbounded_passes_through",
            "999",
            None,
            None,
            "(assert! (= $n 999))",
        ),
    ];
    for &(case, value, min, max, check) in rows {
        assert_forwards(case, value, min, max, check);
    }
}
