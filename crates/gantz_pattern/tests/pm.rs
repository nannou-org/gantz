//! End-to-end tests for the `pm` node: compile, evaluate and memoise.

use gantz_core::node::{self, Node, WithPushEval};
use gantz_core::steel::SteelVal;
use gantz_core::{Edge, compile::push_pull_entrypoints};
use std::fmt::Debug;

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

// push -> pm -> query(span [0,1)) -> sink; two evals produce identical
// events and the state holds the memoised (hash . pattern) pair.
#[test]
fn pm_compiles_evaluates_and_memoises() {
    let mut g = petgraph::graph::DiGraph::new();
    let push =
        g.add_node(Box::new(node::expr("'()").unwrap().with_push_eval()) as Box<dyn DebugNode>);
    let pm = g.add_node(Box::new(gantz_pattern::Pm::new("bd(3,8) <hh cp>")) as Box<_>);
    let q = g.add_node(Box::new(
        node::expr("(pat/query $p (pat/span 0 1))")
            .unwrap()
            .with_requires(["gantz/pattern"]),
    ) as Box<_>);
    let sink = g.add_node(Box::new(node::expr("(begin (set! state $x) state)").unwrap()) as Box<_>);
    g.add_edge(push, pm, Edge::from((0, 0)));
    g.add_edge(pm, q, Edge::from((0, 0)));
    g.add_edge(q, sink, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let (mut vm, _c) = gantz_core::vm::init_with_modules(
        &no_lookup,
        &g,
        &eps,
        &Default::default(),
        gantz_pattern::modules(),
    )
    .expect("init");
    let fn_name = gantz_core::compile::entry_fn_name(&eps[0].id());

    vm.call_function_by_name_with_args(&fn_name, vec![])
        .expect("eval 1");
    let first: SteelVal = gantz_core::node::state::extract_value(&vm, &[sink.index()])
        .unwrap()
        .unwrap();
    vm.call_function_by_name_with_args(&fn_name, vec![])
        .expect("eval 2");
    let second: SteelVal = gantz_core::node::state::extract_value(&vm, &[sink.index()])
        .unwrap()
        .unwrap();
    assert_eq!(format!("{first:?}"), format!("{second:?}"));
    assert!(
        format!("{first:?}").contains("event"),
        "events flowed: {first:?}"
    );

    // The pm state holds the memo pair.
    let memo: SteelVal = gantz_core::node::state::extract_value(&vm, &[pm.index()])
        .unwrap()
        .unwrap();
    assert!(matches!(memo, SteelVal::Pair(_)), "memo pair, got {memo:?}");
}
