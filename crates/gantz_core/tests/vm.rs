//! Tests for the vm module: single-program execution with a registered
//! source, recompilation, and steel-error-to-node attribution.

use gantz_core::{
    Edge, ROOT_STATE,
    compile::{entry_fn_name, push_pull_entrypoints},
    node::{self, GraphNode, Node, WithPushEval},
};
use std::fmt::Debug;
use steel::{SteelVal, steel_vm::engine::Engine};

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A no-op node lookup function for tests that don't need it.
fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

fn node_push() -> node::Push<node::Expr> {
    node::expr("'()").unwrap().with_push_eval()
}

fn node_int(i: i32) -> node::Expr {
    node::expr(format!("(begin $push {})", i)).unwrap()
}

// A counter: increments its numeric state on every push, starting from 1.
fn node_counter() -> node::Expr {
    node::expr("(begin $push (set! state (if (number? state) (+ state 1) 1)) state)").unwrap()
}

// A stateful sink remembering the last value it received.
fn node_sink() -> node::Expr {
    node::expr("(begin (set! state $x) state)").unwrap()
}

// A graph with a stateful counter and a nested graph, ending in a stateful
// sink so results survive evaluation:
//
//    push -> counter -> [inlet -> +1 -> outlet] -> sink
fn test_graph() -> (
    petgraph::graph::DiGraph<Box<dyn DebugNode>, Edge>,
    petgraph::graph::NodeIndex,
    petgraph::graph::NodeIndex,
) {
    let mut ga = GraphNode::default();
    let inlet = ga.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let inc = ga.add_node(Box::new(node::expr("(+ $x 1)").unwrap()) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet) as Box<_>);
    ga.add_edge(inlet, inc, Edge::from((0, 0)));
    ga.add_edge(inc, outlet, Edge::from((0, 0)));

    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let counter = g.add_node(Box::new(node_counter()) as Box<_>);
    let nested = g.add_node(Box::new(ga) as Box<_>);
    let sink = g.add_node(Box::new(node_sink()) as Box<_>);
    g.add_edge(push, counter, Edge::from((0, 0)));
    g.add_edge(counter, nested, Edge::from((0, 0)));
    g.add_edge(nested, sink, Edge::from((0, 0)));
    (g, push, sink)
}

/// The entrypoint fn name for the push node.
fn push_fn_name(
    g: &petgraph::graph::DiGraph<Box<dyn DebugNode>, Edge>,
    push: petgraph::graph::NodeIndex,
) -> String {
    let eps = push_pull_entrypoints(&no_lookup, g);
    let ep = eps
        .iter()
        .find(|ep| ep.0.iter().any(|src| src.path == [push.index()]))
        .unwrap();
    entry_fn_name(&ep.id())
}

/// Extract the sink node's state from the VM's root state.
fn sink_state(vm: &Engine, sink: petgraph::graph::NodeIndex) -> SteelVal {
    node::state::extract_value(vm, &[sink.index()])
        .unwrap()
        .unwrap()
}

// The single-program run (vm::init) must behave identically to the
// historical per-expression loop.
#[test]
fn single_run_parity_with_per_expr() {
    let (g, push, sink) = test_graph();
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let fn_name = push_fn_name(&g, push);

    // New path: single program with registered source.
    let (mut vm_new, _) = gantz_core::vm::init(&no_lookup, &g, &eps, &Default::default()).unwrap();
    // Old path: each expression run separately.
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();
    let mut vm_old = Engine::new_base();
    vm_old.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm_old);
    for expr in &module {
        vm_old.run(expr.to_pretty(80)).unwrap();
    }

    for _ in 0..3 {
        vm_new
            .call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
        vm_old
            .call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
        assert_eq!(sink_state(&vm_new, sink), sink_state(&vm_old, sink));
    }
    // Three increments through the nested `+1`.
    assert_eq!(sink_state(&vm_new, sink), SteelVal::IntV(4));
}

// Recompiling into the same engine redefines the module and preserves node
// state.
#[test]
fn recompile_redefines_module() {
    let (mut g, push, sink) = test_graph();
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let fn_name = push_fn_name(&g, push);
    let (mut vm, _) = gantz_core::vm::init(&no_lookup, &g, &eps, &Default::default()).unwrap();

    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();
    assert_eq!(sink_state(&vm, sink), SteelVal::IntV(2));

    // Replace the sink with a scaling variant and recompile into the same
    // engine.
    g[sink] = Box::new(node::expr("(begin (set! state (* 100 $x)) state)").unwrap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    gantz_core::vm::compile(&no_lookup, &g, &mut vm, &eps, &Default::default()).unwrap();

    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();
    // Counter state persisted (2nd call -> 2), nested +1 -> 3, sink scales.
    assert_eq!(sink_state(&vm, sink), SteelVal::IntV(300));
}

// A runtime steel error maps back to the erroring node's full path via the
// registered source and the module's source map.
#[test]
fn steel_err_node_attribution() {
    // The failing node lives inside a nested graph to exercise full-path
    // resolution: push -> int -> [inlet -> car-of-int -> outlet].
    let mut ga = GraphNode::default();
    let inlet = ga.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let boom = ga.add_node(Box::new(node::expr("(car $x)").unwrap()) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet) as Box<_>);
    ga.add_edge(inlet, boom, Edge::from((0, 0)));
    ga.add_edge(boom, outlet, Edge::from((0, 0)));

    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int = g.add_node(Box::new(node_int(7)) as Box<_>);
    let nested = g.add_node(Box::new(ga) as Box<_>);
    g.add_edge(push, int, Edge::from((0, 0)));
    g.add_edge(int, nested, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let (mut vm, compiled) =
        gantz_core::vm::init(&no_lookup, &g, &eps, &Default::default()).unwrap();
    let fn_name = push_fn_name(&g, push);
    let err = vm
        .call_function_by_name_with_args(&fn_name, vec![])
        .unwrap_err();

    let path = gantz_core::vm::steel_err_node(&err, &vm, &compiled);
    assert_eq!(path, Some(vec![nested.index(), boom.index()]));

    // After a recompile, errors attribute against the *new* module: fail at
    // the root `int` node, which evaluates before the nested graph.
    g[int] = Box::new(node::expr("(begin $push (car '()))").unwrap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let recompiled =
        gantz_core::vm::compile(&no_lookup, &g, &mut vm, &eps, &Default::default()).unwrap();
    let err = vm
        .call_function_by_name_with_args(&fn_name, vec![])
        .unwrap_err();
    let path = gantz_core::vm::steel_err_node(&err, &vm, &recompiled);
    assert_eq!(path, Some(vec![int.index()]));
    // The stale pre-recompile error no longer attributes to the new module.
    // (Its span belongs to the old source text.)
}
