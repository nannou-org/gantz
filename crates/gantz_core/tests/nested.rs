//! Tests related to the nesting of graphs.

use gantz_core::{
    Edge, ROOT_STATE,
    compile::{entry_fn_name, entrypoint, push_pull_entrypoints, push_source},
    node::{self, GraphNode, Node, WithPushEval},
};
use std::fmt::Debug;
use steel::{SteelVal, steel_vm::engine::Engine};

fn node_push() -> node::Push<node::Expr> {
    node::expr("'()").unwrap().with_push_eval()
}

fn node_int(i: i32) -> node::Expr {
    node::expr(format!("(begin $push {})", i)).unwrap()
}

fn node_mul() -> node::Expr {
    node::expr("(* $l $r)").unwrap()
}

fn node_assert_eq() -> node::Expr {
    node::expr("(begin (assert! (equal? $l $r)))").unwrap()
}

fn node_number() -> node::Expr {
    node::expr(
        "
        (let ((x $x))
          (set! state (if (number? x) x state))
          state)
    ",
    )
    .unwrap()
}

// Helper trait for debugging the graph.
trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A no-op node lookup function for tests that don't need it.
fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

// A simple test for nested graph support.
//
// This is the core method of abstraction provided by gantz, so it better work!
//
// GRAPH A
//
//    --------- ---------
//    | Inlet | | Inlet |
//    -+------- -+-------
//     |         |
//     |   -------
//     |   |
//    -+---+-
//    | Mul |
//    -+-----
//     |
//    -+--------
//    | Outlet |
//    ----------
//
// GRAPH B
//
//    --------
//    | push | // push_eval
//    -+------
//     |
//     |------------
//     |           |
//     |------     |
//     |     |     |
//    -+--- -+---  |
//    | 6 | | 7 |  |
//    -+--- -+---  |
//     |     |     |
//     |     ---   |
//     |       |   |
//    -+-------+- -+----
//    | GRAPH A | | 42 |
//    -+--------- -+----
//     |           |
//     |         ---
//     |         |
//    -+---------+-
//    | assert_eq |
//    -------------
#[test]
fn test_graph_nested_stateless() {
    env_logger::init();

    // Graph A, nested within a node.
    let mut ga = GraphNode::default();
    let inlet_a = ga.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let inlet_b = ga.add_node(Box::new(node::graph::Inlet) as Box<_>);
    let mul = ga.add_node(Box::new(node_mul()) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet) as Box<_>);
    ga.add_edge(inlet_a, mul, Edge::from((0, 0)));
    ga.add_edge(inlet_b, mul, Edge::from((0, 1)));
    ga.add_edge(mul, outlet, Edge::from((0, 0)));

    // Graph B.
    let mut gb = petgraph::graph::DiGraph::new();
    let push = gb.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let six = gb.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = gb.add_node(Box::new(node_int(7)) as Box<_>);
    let graph_a = gb.add_node(Box::new(ga) as Box<_>);
    let forty_two = gb.add_node(Box::new(node_int(42)) as Box<_>);
    let assert_eq = gb.add_node(Box::new(node_assert_eq()) as Box<_>);
    gb.add_edge(push, six, Edge::from((0, 0)));
    gb.add_edge(push, seven, Edge::from((0, 0)));
    gb.add_edge(push, forty_two, Edge::from((0, 0)));
    gb.add_edge(six, graph_a, Edge::from((0, 0)));
    gb.add_edge(seven, graph_a, Edge::from((0, 1)));
    gb.add_edge(graph_a, assert_eq, Edge::from((0, 0)));
    gb.add_edge(forty_two, assert_eq, Edge::from((0, 1)));

    // Generate the module, which should have just one top-level expr for `push`.
    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &gb);
    let module = gantz_core::compile::module(&no_lookup, &gb, &eps).unwrap();

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state vars.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &gb, &[], &mut vm);

    // Register the fns.
    for f in module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Call the `push` eval function.
    let ep = entrypoint::push(vec![push.index()], gb[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}

// A simple test for nested graph support where the nested graph is stateful.
//
// GRAPH A
//
//    ---------
//    | Inlet |
//    -+-------
//     |
//    -+---------
//    | Counter |
//    -+---------
//     |
//    -+--------
//    | Outlet |
//    ----------
//
// GRAPH B
//
//    --------
//    | push | // push_eval
//    -+------
//     |
//    -+---------
//    | GRAPH A |
//    -+---------
//     |
//    -+--------
//    | number |
//    ----------
//
// We push evaluation from the root graph B's `push` node, and then check that
// the value is incremented by checking the state of the `number` node.
#[test]
fn test_graph_nested_counter() {
    // The counter node for the nested graph.
    let counter = node::expr(
        "
        (begin
          $bang
          (set! state
            (if (number? state) (+ state 1) 0))
          state)
    ",
    )
    .unwrap();

    // Graph A.
    let mut ga = GraphNode::default();
    let inlet = ga.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let counter = ga.add_node(Box::new(counter) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet) as Box<_>);
    ga.add_edge(inlet, counter, Edge::from((0, 0)));
    ga.add_edge(counter, outlet, Edge::from((0, 0)));

    // Graph B.
    let mut gb = petgraph::graph::DiGraph::new();
    let push = gb.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let graph_a = gb.add_node(Box::new(ga) as Box<_>);
    let number = gb.add_node(Box::new(node_number()) as Box<_>);
    gb.add_edge(push, graph_a, Edge::from((0, 0)));
    gb.add_edge(graph_a, number, Edge::from((0, 0)));

    // Generate the module.
    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &gb);
    let module = gantz_core::compile::module(&no_lookup, &gb, &eps).unwrap();

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state vars.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &gb, &[], &mut vm);

    // Register the fns.
    for f in module {
        println!("{}\n", f.to_pretty(100));
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Increment the nested counter by pushing evaluation.
    // The first is `0`, the second is `1`.
    let ep = entrypoint::push(vec![push.index()], gb[push].n_outputs(ctx) as u8);
    let fn_name = entry_fn_name(&ep.id());
    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();
    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();

    // First, check that the nested expr's state is `1`.
    let counter_state = node::state::extract::<u32>(&vm, &[graph_a.index(), counter.index()])
        .expect("failed to extract counter state")
        .expect("counter state was `None`");
    assert_eq!(counter_state, 1);

    // Outlets are stateless - they just pass through their input value.
    // The value flows through to the downstream `number` node.

    // Check that the number in the root graph was updated from the outlet.
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was `None`");
    assert_eq!(number_state, 1);
}

// A feedback loop *inside* a GraphNode: the inner graph counts to 3 via a cycle +
// branch, with a stateful `tick` node inside the loop. Verifies the loop fn is
// emitted in the nested `graph-state` scope (so the tick's state persists at the
// nested path) and the loop's outlet value reaches the parent.
#[test]
fn test_graph_nested_loop() {
    // Inner graph: inlet -> add(+1) -> tick(stateful) -> branch(< 3) -> {back | outlet}.
    let add = node::expr("(+ $acc 1)").unwrap();
    let tick = node::expr("(begin (set! state (+ (if (number? state) state 0) 1)) $x)").unwrap();
    let branch = node::branch(
        "(if (< $sum 3) (list 0 $sum) (list 1 $sum))",
        vec!["10".parse().unwrap(), "01".parse().unwrap()],
    )
    .unwrap();

    let mut ga = GraphNode::default();
    let inlet = ga.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let add = ga.add_node(Box::new(add) as Box<_>);
    let tick = ga.add_node(Box::new(tick) as Box<_>);
    let branch = ga.add_node(Box::new(branch) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet) as Box<_>);
    ga.add_edge(inlet, add, Edge::from((0, 0))); // loop seed
    ga.add_edge(add, tick, Edge::from((0, 0)));
    ga.add_edge(tick, branch, Edge::from((0, 0)));
    ga.add_edge(branch, add, Edge::from((0, 0))); // continue (back-edge)
    ga.add_edge(branch, outlet, Edge::from((1, 0))); // exit

    // Parent graph: push -> int(0) -> graph_a -> number.
    let mut gb = petgraph::graph::DiGraph::new();
    let push = gb.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let zero = gb.add_node(Box::new(node_int(0)) as Box<_>);
    let graph_a = gb.add_node(Box::new(ga) as Box<_>);
    let number = gb.add_node(Box::new(node_number()) as Box<_>);
    gb.add_edge(push, zero, Edge::from((0, 0)));
    gb.add_edge(zero, graph_a, Edge::from((0, 0)));
    gb.add_edge(graph_a, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &gb);
    let module = gantz_core::compile::module(&no_lookup, &gb, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &gb, &[], &mut vm);
    for f in module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    let ep = entrypoint::push(vec![push.index()], gb[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    // The loop's outlet value (3) reached the parent number.
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .unwrap()
        .unwrap();
    assert_eq!(number_state, 3, "loop result via outlet");
    // The stateful tick inside the loop ran 3 times; its state persists at the
    // nested path `[graph_a, tick]`.
    let tick_state = node::state::extract::<u32>(&vm, &[graph_a.index(), tick.index()])
        .unwrap()
        .unwrap();
    assert_eq!(tick_state, 3, "stateful node inside nested loop");
}

// A simple test for pushing evaluation from a node within a nested graph.
//
// GRAPH A
//
//    --------
//    | Push |
//    -+------
//     |
//    -+----
//    | 42 |
//    -+----
//     |
//    -+--------
//    | Outlet |
//    ----------
//
// GRAPH B
//
//    -+---------
//    | GRAPH A |
//    -+---------
//     |
//    -+--------
//    | number |
//    ----------
//
// A simple-as-possible demonstration of pushing evaluation from within a nested
// node, and propagating that evaluation through the outlets of the graph node.
// Test pushing evaluation from a node inside a nested graph.
//
// GRAPH A (inner):
//
//    --------
//    | Push |
//    -+------
//     |
//    -+--------
//    | number | (stores received value in state)
//    ----------
//
// GRAPH B (outer):
//
//    -----------
//    | GRAPH A |
//    -----------
//
// The push fires inside graph A, driving evaluation to the number node
// which stores the received value. This demonstrates that entrypoints
// can target nodes inside nested graphs.
#[test]
fn test_graph_nested_push_eval() {
    // GRAPH A: push -> number (stateful, stores value)
    let mut ga = GraphNode::default();
    let push = ga.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let num = ga.add_node(Box::new(node_number()) as Box<_>);
    ga.add_edge(push, num, Edge::from((0, 0)));

    // Compute push connection count before moving `ga` into `gb`.
    let ctx = node::MetaCtx::new(&no_lookup);
    let push_n_outputs = ga[push].n_outputs(ctx) as u8;

    // Graph B: just contains graph A (no outlet propagation needed).
    let mut gb = petgraph::graph::DiGraph::new();
    let graph_a = gb.add_node(Box::new(ga) as Box<dyn DebugNode>);

    // Nested entrypoint: push inside graph A.
    let ep = entrypoint::from_source(push_source(
        vec![graph_a.index(), push.index()],
        push_n_outputs,
    ));

    // Generate the module.
    let module = gantz_core::compile::module(&no_lookup, &gb, &[ep.clone()]).unwrap();

    // Create the VM.
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &gb, &[], &mut vm);

    // Register the fns.
    for f in &module {
        println!("{}\n", f.to_pretty(100));
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Call the nested push eval fn.
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    // The number node inside graph A should have received the push value.
    // node_push outputs '() which is not a number, so number's state stays
    // at its initial void value. But we can verify the eval ran without
    // error - the state::extract call itself confirms the state path exists.
    let _num_state = node::state::extract_value(&vm, &[graph_a.index(), num.index()])
        .expect("failed to extract number state from nested graph");
}

// Test that inlet bindings work correctly when node indices don't match inlet positions.
//
// This verifies that inlets are correctly bound even when they're not the first nodes
// in the graph (i.e., their node indices don't match their inlet positions).
//
// GRAPH A (inner)
//
//    --------- ---------
//    | dummy | | dummy |  // Non-inlet nodes with indices 0, 1
//    --------- ---------
//
//    --------- ---------
//    | Inlet | | Inlet |  // Inlet nodes with indices 2, 3 (positions 0, 1)
//    -+------- -+-------
//     |         |
//     ----   ----
//        |   |
//       -+---+-
//       | sub |  // Subtracts second from first
//       -+-----
//        |
//       -+--------
//       | Outlet |
//       ----------
//
// GRAPH B (outer)
//
//    --------
//    | push | // push_eval
//    -+------
//     |
//     |------------
//     |           |
//    -+----     -+----
//    | 10 |     | 3 |
//    -+----     -+----
//     |           |
//     ----     ----
//        |     |
//    ----+-----+----
//    | GRAPH A |
//    -+-------------
//     |
//    -+----
//    | 7 |  // Expected result: 10 - 3 = 7
//    -+----
//     |
//    -+-----------
//    | assert_eq |
//    -------------
#[test]
fn test_graph_nested_non_sequential_inlets() {
    // Graph A with non-sequential inlet indices.
    let mut ga = GraphNode::default();

    // Add dummy nodes first to offset inlet indices
    let _dummy1 = ga.add_node(Box::new(node_int(999)) as Box<dyn DebugNode>);
    let _dummy2 = ga.add_node(Box::new(node_int(998)) as Box<_>);

    // Now add inlets - they'll have indices 2 and 3
    let inlet_a = ga.add_node(Box::new(node::graph::Inlet) as Box<_>);
    let inlet_b = ga.add_node(Box::new(node::graph::Inlet) as Box<_>);

    // Add processing nodes
    let sub = ga.add_node(Box::new(node::expr("(- $l $r)").unwrap()) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet) as Box<_>);

    // Connect the graph
    ga.add_edge(inlet_a, sub, Edge::from((0, 0)));
    ga.add_edge(inlet_b, sub, Edge::from((0, 1)));
    ga.add_edge(sub, outlet, Edge::from((0, 0)));

    // Graph B that uses graph A.
    let mut gb = petgraph::graph::DiGraph::new();
    let push = gb.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let ten = gb.add_node(Box::new(node_int(10)) as Box<_>);
    let three = gb.add_node(Box::new(node_int(3)) as Box<_>);
    let graph_a = gb.add_node(Box::new(ga) as Box<_>);
    let seven = gb.add_node(Box::new(node_int(7)) as Box<_>);
    let assert_eq = gb.add_node(Box::new(node_assert_eq()) as Box<_>);

    gb.add_edge(push, ten, Edge::from((0, 0)));
    gb.add_edge(push, three, Edge::from((0, 0)));
    gb.add_edge(push, seven, Edge::from((0, 0)));
    gb.add_edge(ten, graph_a, Edge::from((0, 0)));
    gb.add_edge(three, graph_a, Edge::from((0, 1)));
    gb.add_edge(graph_a, assert_eq, Edge::from((0, 0)));
    gb.add_edge(seven, assert_eq, Edge::from((0, 1)));

    // Generate the module.
    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &gb);
    let module = gantz_core::compile::module(&no_lookup, &gb, &eps).unwrap();

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state vars.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &gb, &[], &mut vm);

    // Register the fns.
    for f in module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Call the `push` eval function - should compute 10 - 3 = 7
    let ep = entrypoint::push(vec![push.index()], gb[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}

// Test that push evaluation inside a nested graph propagates through its outlet
// to downstream nodes in the outer graph.
//
// GRAPH A (inner):
//
//    --------
//    | Push |
//    -+------
//     |
//    -+----
//    | 42 |
//    -+----
//     |
//    -+--------
//    | Outlet |
//    ----------
//
// GRAPH B (outer):
//
//    -----------
//    | GRAPH A |
//    -+---------
//     |
//    -+--------
//    | number |
//    ----------
//
// The push fires inside graph A, value 42 flows through the outlet to the
// number node in the outer graph. Verifies that nested push evaluation
// propagates through outlets.
#[test]
fn test_graph_nested_push_through_outlet() {
    // GRAPH A: push -> int(42) -> outlet
    let mut ga = GraphNode::default();
    let push = ga.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let forty_two = ga.add_node(Box::new(node_int(42)) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet) as Box<_>);
    ga.add_edge(push, forty_two, Edge::from((0, 0)));
    ga.add_edge(forty_two, outlet, Edge::from((0, 0)));

    // Compute push connection count before moving `ga` into `gb`.
    let ctx = node::MetaCtx::new(&no_lookup);
    let push_n_outputs = ga[push].n_outputs(ctx) as u8;

    // GRAPH B: graph_a -> number
    let mut gb = petgraph::graph::DiGraph::new();
    let graph_a = gb.add_node(Box::new(ga) as Box<dyn DebugNode>);
    let number = gb.add_node(Box::new(node_number()) as Box<_>);
    gb.add_edge(graph_a, number, Edge::from((0, 0)));

    // Nested entrypoint: push inside graph A.
    let ep = entrypoint::from_source(push_source(
        vec![graph_a.index(), push.index()],
        push_n_outputs,
    ));

    // Generate the module.
    let module = gantz_core::compile::module(&no_lookup, &gb, &[ep.clone()]).unwrap();

    // Create the VM.
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &gb, &[], &mut vm);

    // Register the fns.
    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Call the nested push eval fn.
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    // The number node should have received 42 via the outlet.
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was None");
    assert_eq!(number_state, 42);
}

// Test that a nested graph with multiple outlets correctly returns a list
// that is destructured via `define-values` in the outer graph.
//
// INNER GRAPH:
//
//    --------- ---------
//    | Inlet | | Inlet |
//    -+------- -+-------
//     |         |
//    -+-------  |
//    | Outlet | |
//    ---------- |
//              -+-------
//              | Outlet |
//              ----------
//
// OUTER GRAPH:
//
//    --------
//    | push |
//    -+------
//     |
//     |------
//     |     |
//    -+--- -+---
//    | 6 | | 7 |
//    -+--- -+---
//     |     |
//    -+-----+----
//    | INNER    |
//    -+------+---
//     |      |
//     o0     o1
//     |      |
//   num_a  num_b
#[test]
fn test_graph_nested_multi_outlet() {
    // Inner graph: 2 inlets pass through to 2 outlets.
    let mut inner = GraphNode::default();
    let inlet_a = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let inlet_b = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
    let outlet_a = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    let outlet_b = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    inner.add_edge(inlet_a, outlet_a, Edge::from((0, 0)));
    inner.add_edge(inlet_b, outlet_b, Edge::from((0, 0)));

    // Outer graph.
    let mut outer = petgraph::graph::DiGraph::new();
    let push = outer.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let six = outer.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = outer.add_node(Box::new(node_int(7)) as Box<_>);
    let graph = outer.add_node(Box::new(inner) as Box<_>);
    let num_a = outer.add_node(Box::new(node_number()) as Box<_>);
    let num_b = outer.add_node(Box::new(node_number()) as Box<_>);

    outer.add_edge(push, six, Edge::from((0, 0)));
    outer.add_edge(push, seven, Edge::from((0, 0)));
    outer.add_edge(six, graph, Edge::from((0, 0)));
    outer.add_edge(seven, graph, Edge::from((0, 1)));
    outer.add_edge(graph, num_a, Edge::from((0, 0))); // outlet 0
    outer.add_edge(graph, num_b, Edge::from((1, 0))); // outlet 1

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &outer);
    let module = gantz_core::compile::module(&no_lookup, &outer, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &outer, &[], &mut vm);

    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    let ep = entrypoint::push(vec![push.index()], outer[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    let a = node::state::extract::<u32>(&vm, &[num_a.index()])
        .expect("failed to extract num_a state")
        .expect("num_a state was None");
    let b = node::state::extract::<u32>(&vm, &[num_b.index()])
        .expect("failed to extract num_b state")
        .expect("num_b state was None");
    assert_eq!(a, 6);
    assert_eq!(b, 7);
}

// Test nested push evaluation propagating through multiple outlets.
//
// INNER GRAPH:
//    push -> int(10) -> outlet_a
//                    -> outlet_b (via int(20))
//
// OUTER GRAPH:
//    inner_graph -> num_a (from outlet 0)
//               -> num_b (from outlet 1)
//
// Push fires inside inner graph. Both outlet values should propagate to
// the outer graph's number nodes.
#[test]
fn test_graph_nested_push_through_outlet_multi() {
    let mut inner = GraphNode::default();
    let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let ten = inner.add_node(Box::new(node_int(10)) as Box<_>);
    let twenty = inner.add_node(Box::new(node_int(20)) as Box<_>);
    let outlet_a = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    let outlet_b = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    inner.add_edge(push, ten, Edge::from((0, 0)));
    inner.add_edge(push, twenty, Edge::from((0, 0)));
    inner.add_edge(ten, outlet_a, Edge::from((0, 0)));
    inner.add_edge(twenty, outlet_b, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let push_n_outputs = inner[push].n_outputs(ctx) as u8;

    let mut outer = petgraph::graph::DiGraph::new();
    let graph = outer.add_node(Box::new(inner) as Box<dyn DebugNode>);
    let num_a = outer.add_node(Box::new(node_number()) as Box<_>);
    let num_b = outer.add_node(Box::new(node_number()) as Box<_>);
    outer.add_edge(graph, num_a, Edge::from((0, 0)));
    outer.add_edge(graph, num_b, Edge::from((1, 0)));

    let ep = entrypoint::from_source(push_source(
        vec![graph.index(), push.index()],
        push_n_outputs,
    ));

    let module = gantz_core::compile::module(&no_lookup, &outer, &[ep.clone()]).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &outer, &[], &mut vm);

    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    let a = node::state::extract::<u32>(&vm, &[num_a.index()])
        .expect("failed to extract num_a state")
        .expect("num_a state was None");
    let b = node::state::extract::<u32>(&vm, &[num_b.index()])
        .expect("failed to extract num_b state")
        .expect("num_b state was None");
    assert_eq!(a, 10);
    assert_eq!(b, 20);
}

// Test nested push evaluation propagating through two levels of nesting.
//
// INNERMOST GRAPH:
//    push -> int(99) -> outlet
//
// MIDDLE GRAPH:
//    innermost -> outlet
//
// OUTER GRAPH:
//    middle -> number
//
// Push fires in innermost, value 99 propagates through two outlet levels
// to the outer number node.
#[test]
fn test_graph_nested_push_through_outlet_deep() {
    // Innermost: push -> int(99) -> outlet
    let mut innermost = GraphNode::default();
    let push = innermost.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let ninety_nine = innermost.add_node(Box::new(node_int(99)) as Box<_>);
    let outlet_inner = innermost.add_node(Box::new(node::graph::Outlet) as Box<_>);
    innermost.add_edge(push, ninety_nine, Edge::from((0, 0)));
    innermost.add_edge(ninety_nine, outlet_inner, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let push_n_outputs = innermost[push].n_outputs(ctx) as u8;

    // Middle: innermost_graph -> outlet
    let mut middle = GraphNode::default();
    let innermost_node = middle.add_node(Box::new(innermost) as Box<dyn DebugNode>);
    let outlet_mid = middle.add_node(Box::new(node::graph::Outlet) as Box<_>);
    middle.add_edge(innermost_node, outlet_mid, Edge::from((0, 0)));

    // Outer: middle_graph -> number
    let mut outer = petgraph::graph::DiGraph::new();
    let middle_node = outer.add_node(Box::new(middle) as Box<dyn DebugNode>);
    let number = outer.add_node(Box::new(node_number()) as Box<_>);
    outer.add_edge(middle_node, number, Edge::from((0, 0)));

    let ep = entrypoint::from_source(push_source(
        vec![middle_node.index(), innermost_node.index(), push.index()],
        push_n_outputs,
    ));

    let module = gantz_core::compile::module(&no_lookup, &outer, &[ep.clone()]).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &outer, &[], &mut vm);

    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was None");
    assert_eq!(val, 99);
}

// Test that `push_pull_entrypoints` discovers push eval nodes inside nested
// graphs. This mirrors the real-world scenario of a FrameBang node inside a
// nested graph placed in a top-level graph via NamedRef.
//
// INNER GRAPH:
//    push -> int(42) -> outlet
//
// OUTER GRAPH:
//    inner_graph -> number
//
// `push_pull_entrypoints` on the outer graph should discover the push node
// inside the inner graph and create an entrypoint with path [graph_a, push].
#[test]
fn test_push_pull_entrypoints_discovers_nested_push() {
    // Inner graph: push -> int(42) -> outlet
    let mut inner = GraphNode::default();
    let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let forty_two = inner.add_node(Box::new(node_int(42)) as Box<_>);
    let outlet = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    inner.add_edge(push, forty_two, Edge::from((0, 0)));
    inner.add_edge(forty_two, outlet, Edge::from((0, 0)));

    // Outer graph: inner -> number
    let mut outer = petgraph::graph::DiGraph::new();
    let graph_a = outer.add_node(Box::new(inner) as Box<dyn DebugNode>);
    let number = outer.add_node(Box::new(node_number()) as Box<_>);
    outer.add_edge(graph_a, number, Edge::from((0, 0)));

    // push_pull_entrypoints should find the nested push node.
    let eps = push_pull_entrypoints(&no_lookup, &outer);
    assert!(
        !eps.is_empty(),
        "push_pull_entrypoints should discover the nested push eval node"
    );

    // There should be an entrypoint with path [graph_a, push].
    let has_nested_push = eps.iter().any(|ep| {
        ep.0.iter()
            .any(|src| src.path == vec![graph_a.index(), push.index()])
    });
    assert!(
        has_nested_push,
        "expected entrypoint at path [{}, {}], found: {:?}",
        graph_a.index(),
        push.index(),
        eps.iter()
            .flat_map(|ep| ep.0.iter().map(|s| &s.path))
            .collect::<Vec<_>>()
    );

    // The generated module should include the entry fn for this entrypoint,
    // and it should work end-to-end (value 42 flows through outlet to number).
    let module = gantz_core::compile::module(&no_lookup, &outer, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &outer, &[], &mut vm);

    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Find and call the nested push entrypoint.
    let nested_ep = eps
        .iter()
        .find(|ep| {
            ep.0.iter()
                .any(|src| src.path == vec![graph_a.index(), push.index()])
        })
        .unwrap();
    vm.call_function_by_name_with_args(&entry_fn_name(&nested_ep.id()), vec![])
        .unwrap();

    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was None");
    assert_eq!(val, 42);
}

// Test that two nested graph nodes sharing a multi-source entrypoint both
// propagate through their outlets to the parent graph.
//
// This mirrors the scenario of two NamedRef "deltams" nodes in a top-level
// graph, where both contain a FrameBang and are combined into a single
// multi-source entrypoint.
//
// INNER GRAPH (shared by both):
//    push -> int(10) -> outlet
//
// OUTER GRAPH:
//    graph_a -> num_a
//    graph_b -> num_b
//
// A single multi-source entrypoint fires push inside both graph_a and graph_b.
// Both outlets should propagate, writing 10 to both num_a and num_b.
#[test]
fn test_graph_nested_multi_source_outlet_propagation() {
    // Inner graph: push -> int(10) -> outlet
    let make_inner = || {
        let mut inner = GraphNode::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let ten = inner.add_node(Box::new(node_int(10)) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(push, ten, Edge::from((0, 0)));
        inner.add_edge(ten, outlet, Edge::from((0, 0)));
        (inner, push)
    };

    let (inner_a, push_a) = make_inner();
    let (inner_b, push_b) = make_inner();

    let ctx = node::MetaCtx::new(&no_lookup);
    let push_n_outputs = inner_a[push_a].n_outputs(ctx) as u8;

    // Outer graph: two graph nodes -> two number nodes
    let mut outer = petgraph::graph::DiGraph::new();
    let graph_a = outer.add_node(Box::new(inner_a) as Box<dyn DebugNode>);
    let graph_b = outer.add_node(Box::new(inner_b) as Box<dyn DebugNode>);
    let num_a = outer.add_node(Box::new(node_number()) as Box<_>);
    let num_b = outer.add_node(Box::new(node_number()) as Box<_>);
    outer.add_edge(graph_a, num_a, Edge::from((0, 0)));
    outer.add_edge(graph_b, num_b, Edge::from((0, 0)));

    // Multi-source entrypoint: both pushes in one entrypoint.
    let ep = entrypoint::from_sources([
        push_source(vec![graph_a.index(), push_a.index()], push_n_outputs),
        push_source(vec![graph_b.index(), push_b.index()], push_n_outputs),
    ]);

    let module = gantz_core::compile::module(&no_lookup, &outer, &[ep.clone()]).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &outer, &[], &mut vm);

    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    let a = node::state::extract::<u32>(&vm, &[num_a.index()])
        .expect("failed to extract num_a state")
        .expect("num_a state was None");
    let b = node::state::extract::<u32>(&vm, &[num_b.index()])
        .expect("failed to extract num_b state")
        .expect("num_b state was None");
    assert_eq!(a, 10, "graph_a outlet should propagate to num_a");
    assert_eq!(b, 10, "graph_b outlet should propagate to num_b");
}

// Test a multi-source entrypoint with sources at different nesting levels:
// a direct push source at the root and a nested push source inside a graph
// node that propagates through an outlet.
//
// This mirrors the scenario of a top-level FrameBang + a NamedRef "deltams"
// (which contains its own FrameBang inside) combined into one entrypoint.
//
// INNER GRAPH:
//    push_inner -> int(10) -> outlet
//
// OUTER GRAPH:
//    graph_node --(outlet)--> add (input 0)
//    push_outer ------------> add (input 1)
//    add -> number
//
// Both pushes fire in a single entrypoint. The graph_node outlet value (10)
// and push_outer value ('()) reach add, whose result is stored in number.
#[test]
fn test_graph_nested_mixed_level_multi_source() {
    // Inner graph: push -> int(10) -> outlet
    let mut inner = GraphNode::default();
    let push_inner = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let ten = inner.add_node(Box::new(node_int(10)) as Box<_>);
    let outlet = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    inner.add_edge(push_inner, ten, Edge::from((0, 0)));
    inner.add_edge(ten, outlet, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let push_inner_n = inner[push_inner].n_outputs(ctx) as u8;

    // Outer graph
    let mut outer = petgraph::graph::DiGraph::new();
    let graph_node = outer.add_node(Box::new(inner) as Box<dyn DebugNode>);
    let push_outer = outer.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let twenty = outer.add_node(Box::new(node_int(20)) as Box<_>);
    // add: (+ $l $r) - takes two inputs
    let add = outer.add_node(Box::new(node::expr("(+ $l $r)").unwrap()) as Box<_>);
    let number = outer.add_node(Box::new(node_number()) as Box<_>);
    outer.add_edge(graph_node, add, Edge::from((0, 0))); // outlet(10) -> add input 0
    outer.add_edge(push_outer, twenty, Edge::from((0, 0))); // push -> int(20)
    outer.add_edge(twenty, add, Edge::from((0, 1))); // int(20) -> add input 1
    outer.add_edge(add, number, Edge::from((0, 0)));

    let push_outer_n = outer[push_outer].n_outputs(ctx) as u8;

    // Multi-source entrypoint: nested push + direct push
    let ep = entrypoint::from_sources([
        push_source(vec![graph_node.index(), push_inner.index()], push_inner_n),
        push_source(vec![push_outer.index()], push_outer_n),
    ]);

    let module = gantz_core::compile::module(&no_lookup, &outer, &[ep.clone()]).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &outer, &[], &mut vm);

    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    // outlet produces 10, push_outer -> int produces 20, add = 10 + 20 = 30
    let val = node::state::extract::<i32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was None");
    assert_eq!(val, 30);
}

// ===========================================================================
// Nested-graph branching tests.
//
// A nested graph whose interior branches should report that branching to the
// outer graph via `Node::branches`, so the outer graph only evaluates the
// downstream of outlets actually produced by the taken inner branch.
//
// Each test's INNER graph is sketched above it. `branches()` lists the sets of
// outputs active per external branch (`{}` = a branch producing nothing).
// Diagram legend:
//   [In]     inlet            [Out X]  outlet
//   [Sel]    node_select: input ==0 -> o0(42), else -> o1(99)
//   oN       branch output N    /  \   the two arms of a branch
// The OUTER graph is uniform: push -> int value(s) -> [GraphNode] -> a `number`
// store per output (which records the value it receives).
// ===========================================================================

// A 1-input, 2-output branch primitive. Routes to output 0 (value 42) when the
// input is 0, else to output 1 (value 99).
fn node_select() -> node::Branch {
    node::branch(
        "(if (= 0 $x) (list 0 42) (list 1 99))",
        vec![
            node::Conns::try_from([true, false]).unwrap(),
            node::Conns::try_from([false, true]).unwrap(),
        ],
    )
    .unwrap()
}

// Assert that `inner.branches()` reports exactly `expected` (order-insensitive),
// where each entry lists the output indices active in that branch.
fn assert_inner_branches<N: Node + ?Sized>(inner: &N, n_outputs: usize, expected: &[&[u16]]) {
    let ctx = node::MetaCtx::new(&no_lookup);
    let got: std::collections::BTreeSet<Vec<u16>> = inner
        .branches(ctx)
        .iter()
        .map(|b| {
            let node::EvalConf::Set(c) = b else {
                panic!("expected EvalConf::Set, got {b:?}");
            };
            (0..n_outputs as u16)
                .filter(|&i| c.get(i as usize).unwrap_or(false))
                .collect()
        })
        .collect();
    let want: std::collections::BTreeSet<Vec<u16>> = expected
        .iter()
        .map(|e| {
            let mut v = e.to_vec();
            v.sort();
            v
        })
        .collect();
    assert_eq!(got, want, "branch patterns mismatch");
}

// Build, compile and run a graph from `push`; returns the VM for state queries.
fn compile_and_push<N: DebugNode + ?Sized>(
    g: &petgraph::graph::DiGraph<Box<N>, Edge>,
    push: petgraph::graph::NodeIndex,
) -> Engine {
    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, g);
    let module = gantz_core::compile::module(&no_lookup, g, &eps).unwrap();
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, g, &[], &mut vm);
    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
    vm
}

// `Some(v)` if the store node holds a number, else `None` (never evaluated).
fn store_val(vm: &Engine, store: petgraph::graph::NodeIndex) -> Option<i32> {
    node::state::extract::<i32>(vm, &[store.index()])
        .ok()
        .flatten()
}

// Build, compile and run `g` from a `push_eval` nested at `path` (e.g.
// `[graph_node, push]`); returns the VM for state queries. Unlike
// `compile_and_push`, the push lives *inside* a nested graph and propagates out
// through that graph's outlets.
fn compile_and_push_nested<N: DebugNode + ?Sized>(
    g: &petgraph::graph::DiGraph<Box<N>, Edge>,
    path: Vec<usize>,
    push_n_outputs: u8,
) -> Engine {
    let ep = entrypoint::from_source(push_source(path, push_n_outputs));
    let module = gantz_core::compile::module(&no_lookup, g, &[ep.clone()]).unwrap();
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, g, &[], &mut vm);
    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
    vm
}

// Divergent branch: each arm routes to its own outlet.  branches: [{A}, {B}]
//
//        [In]
//         |
//       [Sel]
//      o0/  \o1
//   [Out A]  [Out B]
#[test]
fn test_graph_nested_divergent_branch() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet, select, Edge::from((0, 0)));
        inner.add_edge(select, outlet_a, Edge::from((0, 0)));
        inner.add_edge(select, outlet_b, Edge::from((1, 0)));
        inner
    };

    // Two arms: arm 0 -> outlet A only, arm 1 -> outlet B only.
    assert_inner_branches(&make_inner(), 2, &[&[0], &[1]]);

    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let store_a = g.add_node(Box::new(node_number()) as Box<_>);
        let store_b = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0)));
        g.add_edge(inner_node, store_a, Edge::from((0, 0)));
        g.add_edge(inner_node, store_b, Edge::from((1, 0)));
        let vm = compile_and_push(&g, push);
        (store_val(&vm, store_a), store_val(&vm, store_b))
    };

    // sel == 0 -> arm 0 -> only store_a written (42).
    assert_eq!(build(0), (Some(42), None));
    // sel != 0 -> arm 1 -> only store_b written (99).
    assert_eq!(build(1), (None, Some(99)));
}

// Reconvergent: both arms feed the SAME outlet, so it is always produced and
// there is no external branching (the value still differs per arm).  branches: []
//
//        [In]
//         |
//       [Sel]
//      o0\  /o1
//       [Out A]
#[test]
fn test_graph_nested_reconvergent_branch() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet, select, Edge::from((0, 0)));
        inner.add_edge(select, outlet, Edge::from((0, 0)));
        inner.add_edge(select, outlet, Edge::from((1, 0)));
        inner
    };

    // No external branching: the outlet is always produced.
    assert_inner_branches(&make_inner(), 1, &[]);

    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let store = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0)));
        g.add_edge(inner_node, store, Edge::from((0, 0)));
        store_val(&compile_and_push(&g, push), store)
    };

    // Outlet always written; the value differs per arm (phi reconvergence).
    assert_eq!(build(0), Some(42));
    assert_eq!(build(1), Some(99));
}

// Dead arm: arm 1's output is unconnected, so it produces nothing.
// branches: [{}, {A}]
//
//        [In]
//         |
//       [Sel]
//      o0|   \o1   (unconnected = dead arm)
//   [Out A]    x
#[test]
fn test_graph_nested_dead_arm() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet, select, Edge::from((0, 0)));
        inner.add_edge(select, outlet, Edge::from((0, 0)));
        // Output 1 of Select is left unconnected (a dead arm).
        inner
    };

    // Two patterns: empty (dead arm) and {outlet}.
    assert_inner_branches(&make_inner(), 1, &[&[], &[0]]);

    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let store = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0)));
        g.add_edge(inner_node, store, Edge::from((0, 0)));
        store_val(&compile_and_push(&g, push), store)
    };

    assert_eq!(build(0), Some(42)); // arm 0 -> outlet
    assert_eq!(build(1), None); // arm 1 -> dead, nothing downstream evaluated
}

// Per-arm intermediates: each arm transforms its value before its outlet.
// branches: [{A}, {B}]
//
//         [In]
//          |
//        [Sel]
//      o0/    \o1
//    [+10]    [+20]
//      |        |
//   [Out A]   [Out B]
#[test]
fn test_graph_nested_branch_intermediates() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let add10 = inner.add_node(Box::new(node::expr("(+ $x 10)").unwrap()) as Box<_>);
        let add20 = inner.add_node(Box::new(node::expr("(+ $x 20)").unwrap()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet, select, Edge::from((0, 0)));
        inner.add_edge(select, add10, Edge::from((0, 0)));
        inner.add_edge(add10, outlet_a, Edge::from((0, 0)));
        inner.add_edge(select, add20, Edge::from((1, 0)));
        inner.add_edge(add20, outlet_b, Edge::from((0, 0)));
        inner
    };

    assert_inner_branches(&make_inner(), 2, &[&[0], &[1]]);

    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let store_a = g.add_node(Box::new(node_number()) as Box<_>);
        let store_b = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0)));
        g.add_edge(inner_node, store_a, Edge::from((0, 0)));
        g.add_edge(inner_node, store_b, Edge::from((1, 0)));
        let vm = compile_and_push(&g, push);
        (store_val(&vm, store_a), store_val(&vm, store_b))
    };

    assert_eq!(build(0), (Some(52), None)); // 42 + 10
    assert_eq!(build(1), (None, Some(119))); // 99 + 20
}

// Multi-output arm: arm 0 fires TWO outputs (a list value); arm 1 fires one.
// branches: [{A, B}, {C}]
//
//          [In]
//           |
//        [Branch]              arm 0 -> o0,o1  (value (10 20))
//      o0/ o1|  \o2            arm 1 -> o2     (value 30)
//  [Out A][Out B][Out C]
#[test]
fn test_graph_nested_branch_multi_outlet_arm() {
    let branch3 = || {
        node::branch(
            "(if (= 0 $x) (list 0 (list 10 20)) (list 1 30))",
            vec![
                node::Conns::try_from([true, true, false]).unwrap(),
                node::Conns::try_from([false, false, true]).unwrap(),
            ],
        )
        .unwrap()
    };
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let br = inner.add_node(Box::new(branch3()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let outlet_c = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet, br, Edge::from((0, 0)));
        inner.add_edge(br, outlet_a, Edge::from((0, 0)));
        inner.add_edge(br, outlet_b, Edge::from((1, 0)));
        inner.add_edge(br, outlet_c, Edge::from((2, 0)));
        inner
    };

    assert_inner_branches(&make_inner(), 3, &[&[0, 1], &[2]]);

    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let store_a = g.add_node(Box::new(node_number()) as Box<_>);
        let store_b = g.add_node(Box::new(node_number()) as Box<_>);
        let store_c = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0)));
        g.add_edge(inner_node, store_a, Edge::from((0, 0)));
        g.add_edge(inner_node, store_b, Edge::from((1, 0)));
        g.add_edge(inner_node, store_c, Edge::from((2, 0)));
        let vm = compile_and_push(&g, push);
        (
            store_val(&vm, store_a),
            store_val(&vm, store_b),
            store_val(&vm, store_c),
        )
    };

    assert_eq!(build(0), (Some(10), Some(20), None)); // arm 0 -> a,b
    assert_eq!(build(1), (None, None, Some(30))); // arm 1 -> c
}

// Parallel branches: two independent Selects -> Cartesian product of 4 branches.
// Exercises a multi-component (multi-root) inner flow graph.
// branches: [{A,C}, {A,D}, {B,C}, {B,D}]
//
//   [In a]        [In b]
//     |             |
//   [Sel1]        [Sel2]
//  o0/  \o1      o0/  \o1
// [A]    [B]    [C]    [D]
#[test]
fn test_graph_nested_parallel_branches() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet_a = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let inlet_b = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let sel1 = inner.add_node(Box::new(node_select()) as Box<_>);
        let sel2 = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let od = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet_a, sel1, Edge::from((0, 0)));
        inner.add_edge(inlet_b, sel2, Edge::from((0, 0)));
        inner.add_edge(sel1, oa, Edge::from((0, 0)));
        inner.add_edge(sel1, ob, Edge::from((1, 0)));
        inner.add_edge(sel2, oc, Edge::from((0, 0)));
        inner.add_edge(sel2, od, Edge::from((1, 0)));
        inner
    };

    // Outputs A=0, B=1, C=2, D=3; 4 Cartesian arms.
    assert_inner_branches(&make_inner(), 4, &[&[0, 2], &[0, 3], &[1, 2], &[1, 3]]);

    let build = |s1: i32, s2: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let i1 = g.add_node(Box::new(node_int(s1)) as Box<_>);
        let i2 = g.add_node(Box::new(node_int(s2)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let sa = g.add_node(Box::new(node_number()) as Box<_>);
        let sb = g.add_node(Box::new(node_number()) as Box<_>);
        let sc = g.add_node(Box::new(node_number()) as Box<_>);
        let sd = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, i1, Edge::from((0, 0)));
        g.add_edge(push, i2, Edge::from((0, 0)));
        g.add_edge(i1, inner_node, Edge::from((0, 0)));
        g.add_edge(i2, inner_node, Edge::from((0, 1)));
        g.add_edge(inner_node, sa, Edge::from((0, 0)));
        g.add_edge(inner_node, sb, Edge::from((1, 0)));
        g.add_edge(inner_node, sc, Edge::from((2, 0)));
        g.add_edge(inner_node, sd, Edge::from((3, 0)));
        let vm = compile_and_push(&g, push);
        [
            store_val(&vm, sa),
            store_val(&vm, sb),
            store_val(&vm, sc),
            store_val(&vm, sd),
        ]
    };

    assert_eq!(build(0, 0), [Some(42), None, Some(42), None]); // A + C
    assert_eq!(build(0, 1), [Some(42), None, None, Some(99)]); // A + D
    assert_eq!(build(1, 0), [None, Some(99), Some(42), None]); // B + C
    assert_eq!(build(1, 1), [None, Some(99), None, Some(99)]); // B + D
}

// Nested/sequential: Gate exists only under Sel1's arm 0, so the result is
// PRUNED to 3 branches (not the Cartesian 4).  branches: [{A}, {B}, {C}]
//
//  [In sel]  [In val]
//       \      /
//      ($sel,$val)
//        [Sel1]            Sel1: ==0 -> o0=$val, else -> o1=88
//      o0/    \o1
//   [Gate]   [Out C]       Gate reached only via Sel1 arm 0,
//  o0/  \o1                then routes by ($val == 0)
// [Out A][Out B]
#[test]
fn test_graph_nested_sequential_branches() {
    // 2-input outer select: $sel picks the arm, $val is passed on arm 0.
    let outer_sel = || {
        node::branch(
            "(if (= 0 $sel) (list 0 $val) (list 1 88))",
            vec![
                node::Conns::try_from([true, false]).unwrap(),
                node::Conns::try_from([false, true]).unwrap(),
            ],
        )
        .unwrap()
    };
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet_sel = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let inlet_val = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let sel1 = inner.add_node(Box::new(outer_sel()) as Box<_>);
        let gate = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet_sel, sel1, Edge::from((0, 0))); // $sel
        inner.add_edge(inlet_val, sel1, Edge::from((0, 1))); // $val
        inner.add_edge(sel1, gate, Edge::from((0, 0))); // arm 0 -> gate input
        inner.add_edge(sel1, oc, Edge::from((1, 0))); // arm 1 -> C
        inner.add_edge(gate, oa, Edge::from((0, 0)));
        inner.add_edge(gate, ob, Edge::from((1, 0)));
        inner
    };

    // A=0, B=1, C=2. Pruned (not Cartesian): {A}, {B}, {C}.
    assert_inner_branches(&make_inner(), 3, &[&[0], &[1], &[2]]);

    let build = |sel: i32, val: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let i_sel = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let i_val = g.add_node(Box::new(node_int(val)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let sa = g.add_node(Box::new(node_number()) as Box<_>);
        let sb = g.add_node(Box::new(node_number()) as Box<_>);
        let sc = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, i_sel, Edge::from((0, 0)));
        g.add_edge(push, i_val, Edge::from((0, 0)));
        g.add_edge(i_sel, inner_node, Edge::from((0, 0)));
        g.add_edge(i_val, inner_node, Edge::from((0, 1)));
        g.add_edge(inner_node, sa, Edge::from((0, 0)));
        g.add_edge(inner_node, sb, Edge::from((1, 0)));
        g.add_edge(inner_node, sc, Edge::from((2, 0)));
        let vm = compile_and_push(&g, push);
        [store_val(&vm, sa), store_val(&vm, sb), store_val(&vm, sc)]
    };

    // sel==0 reaches gate with $val; gate routes by ($val == 0).
    assert_eq!(build(0, 0), [Some(42), None, None]); // gate arm 0 -> A
    assert_eq!(build(0, 5), [None, Some(99), None]); // gate arm 1 -> B
    assert_eq!(build(9, 0), [None, None, Some(88)]); // sel arm 1 -> C
}

// Branch after a join: the branch is reachable from two inlet chains, but must
// be assigned ONCE per world (two branches, not four).  branches: [{A}, {B}]
//
//  [In l]  [In r]
//      \    /
//      [Sum]               (+ $a $b)
//        |
//      [Sel]
//     o0/  \o1
//  [Out A] [Out B]
#[test]
fn test_graph_nested_branch_after_join() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet_l = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let inlet_r = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let sum = inner.add_node(Box::new(node::expr("(+ $a $b)").unwrap()) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet_l, sum, Edge::from((0, 0)));
        inner.add_edge(inlet_r, sum, Edge::from((0, 1)));
        inner.add_edge(sum, select, Edge::from((0, 0)));
        inner.add_edge(select, oa, Edge::from((0, 0)));
        inner.add_edge(select, ob, Edge::from((1, 0)));
        inner
    };

    // Two branches, NOT four (the join-fed branch is assigned once).
    assert_inner_branches(&make_inner(), 2, &[&[0], &[1]]);

    let build = |l: i32, r: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let il = g.add_node(Box::new(node_int(l)) as Box<_>);
        let ir = g.add_node(Box::new(node_int(r)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let sa = g.add_node(Box::new(node_number()) as Box<_>);
        let sb = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, il, Edge::from((0, 0)));
        g.add_edge(push, ir, Edge::from((0, 0)));
        g.add_edge(il, inner_node, Edge::from((0, 0)));
        g.add_edge(ir, inner_node, Edge::from((0, 1)));
        g.add_edge(inner_node, sa, Edge::from((0, 0)));
        g.add_edge(inner_node, sb, Edge::from((1, 0)));
        let vm = compile_and_push(&g, push);
        (store_val(&vm, sa), store_val(&vm, sb))
    };

    assert_eq!(build(0, 0), (Some(42), None)); // sum 0 -> arm 0
    assert_eq!(build(1, 0), (None, Some(99))); // sum 1 -> arm 1
}

// Static outlet: C is fed by a constant (reached via pull, no inlet), so it is
// active in EVERY branch.  branches: [{A, C}, {B, C}]
//
//    [In]          [const 123]
//     |                 |
//   [Sel]               |        (independent chain ->
//  o0/  \o1             |         always produced)
// [A]    [B]        [Out C]
#[test]
fn test_graph_nested_branch_with_constant_outlet() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let konst = inner.add_node(Box::new(node::expr("123").unwrap()) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet, select, Edge::from((0, 0)));
        inner.add_edge(select, oa, Edge::from((0, 0)));
        inner.add_edge(select, ob, Edge::from((1, 0)));
        inner.add_edge(konst, oc, Edge::from((0, 0)));
        inner
    };

    // C (output 2) is active in both branches.
    assert_inner_branches(&make_inner(), 3, &[&[0, 2], &[1, 2]]);

    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let sa = g.add_node(Box::new(node_number()) as Box<_>);
        let sb = g.add_node(Box::new(node_number()) as Box<_>);
        let sc = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0)));
        g.add_edge(inner_node, sa, Edge::from((0, 0)));
        g.add_edge(inner_node, sb, Edge::from((1, 0)));
        g.add_edge(inner_node, sc, Edge::from((2, 0)));
        let vm = compile_and_push(&g, push);
        [store_val(&vm, sa), store_val(&vm, sb), store_val(&vm, sc)]
    };

    assert_eq!(build(0), [Some(42), None, Some(123)]); // A + C
    assert_eq!(build(1), [None, Some(99), Some(123)]); // B + C
}

// Three-arm branch: one branch node with three arms, each to its own outlet.
// branches: [{A}, {B}, {C}]
//
//         [In]
//          |
//       [Branch]   (3 arms)
//     o0/ o1| \o2
//    [A]  [B]  [C]
#[test]
fn test_graph_nested_three_arm_branch() {
    let branch3 = || {
        node::branch(
            "(if (= 0 $x) (list 0 1) (if (= 1 $x) (list 1 2) (list 2 3)))",
            vec![
                node::Conns::try_from([true, false, false]).unwrap(),
                node::Conns::try_from([false, true, false]).unwrap(),
                node::Conns::try_from([false, false, true]).unwrap(),
            ],
        )
        .unwrap()
    };
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let br = inner.add_node(Box::new(branch3()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet, br, Edge::from((0, 0)));
        inner.add_edge(br, oa, Edge::from((0, 0)));
        inner.add_edge(br, ob, Edge::from((1, 0)));
        inner.add_edge(br, oc, Edge::from((2, 0)));
        inner
    };

    assert_inner_branches(&make_inner(), 3, &[&[0], &[1], &[2]]);

    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let sa = g.add_node(Box::new(node_number()) as Box<_>);
        let sb = g.add_node(Box::new(node_number()) as Box<_>);
        let sc = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0)));
        g.add_edge(inner_node, sa, Edge::from((0, 0)));
        g.add_edge(inner_node, sb, Edge::from((1, 0)));
        g.add_edge(inner_node, sc, Edge::from((2, 0)));
        let vm = compile_and_push(&g, push);
        [store_val(&vm, sa), store_val(&vm, sb), store_val(&vm, sc)]
    };

    assert_eq!(build(0), [Some(1), None, None]);
    assert_eq!(build(1), [None, Some(2), None]);
    assert_eq!(build(2), [None, None, Some(3)]);
}

// Two levels of nesting: branching propagates outward through both.
// inner1.branches: [{X}, {Y}]
//
//  inner2:            inner1 (wraps inner2):
//    [In]               [In]
//     |                  |
//   [Sel]            [inner2]   <- itself a branching GraphNode
//  o0/ \o1           o0/  \o1
// [A]   [B]      [Out X]  [Out Y]
#[test]
fn test_graph_nested_branch_two_levels() {
    let make_inner2 = || {
        let mut g = GraphNode::default();
        let inlet = g.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let select = g.add_node(Box::new(node_select()) as Box<_>);
        let oa = g.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let ob = g.add_node(Box::new(node::graph::Outlet) as Box<_>);
        g.add_edge(inlet, select, Edge::from((0, 0)));
        g.add_edge(select, oa, Edge::from((0, 0)));
        g.add_edge(select, ob, Edge::from((1, 0)));
        g
    };
    let make_inner1 = || {
        let mut g = GraphNode::default();
        let inlet = g.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let inner2 = g.add_node(Box::new(make_inner2()) as Box<_>);
        let ox = g.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let oy = g.add_node(Box::new(node::graph::Outlet) as Box<_>);
        g.add_edge(inlet, inner2, Edge::from((0, 0)));
        g.add_edge(inner2, ox, Edge::from((0, 0)));
        g.add_edge(inner2, oy, Edge::from((1, 0)));
        g
    };

    // inner1 branches because inner2 branches.
    assert_inner_branches(&make_inner1(), 2, &[&[0], &[1]]);

    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner1()) as Box<_>);
        let sx = g.add_node(Box::new(node_number()) as Box<_>);
        let sy = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0)));
        g.add_edge(inner_node, sx, Edge::from((0, 0)));
        g.add_edge(inner_node, sy, Edge::from((1, 0)));
        let vm = compile_and_push(&g, push);
        (store_val(&vm, sx), store_val(&vm, sy))
    };

    assert_eq!(build(0), (Some(42), None));
    assert_eq!(build(1), (None, Some(99)));
}

// Stateful node on a branch arm: its state persists across pushes taking arm 0.
// branches: [{A}, {B}]
//
//       [In]
//        |
//      [Sel]
//     o0/  \o1
//  [Counter] |          (state persists across pushes on arm 0)
//     |      |
//  [Out A] [Out B]
#[test]
fn test_graph_nested_branch_stateful() {
    let counter = || {
        node::expr("(begin $bang (set! state (if (number? state) (+ state 1) 0)) state)").unwrap()
    };
    let mut inner = GraphNode::default();
    let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let select = inner.add_node(Box::new(node_select()) as Box<_>);
    let count = inner.add_node(Box::new(counter()) as Box<_>);
    let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    inner.add_edge(inlet, select, Edge::from((0, 0)));
    inner.add_edge(select, count, Edge::from((0, 0)));
    inner.add_edge(count, oa, Edge::from((0, 0)));
    inner.add_edge(select, ob, Edge::from((1, 0)));

    assert_inner_branches(&inner, 2, &[&[0], &[1]]);

    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int = g.add_node(Box::new(node_int(0)) as Box<_>); // always arm 0
    let inner_node = g.add_node(Box::new(inner) as Box<_>);
    let sa = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, int, Edge::from((0, 0)));
    g.add_edge(int, inner_node, Edge::from((0, 0)));
    g.add_edge(inner_node, sa, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    let fname = entry_fn_name(&ep.id());
    vm.call_function_by_name_with_args(&fname, vec![]).unwrap();
    vm.call_function_by_name_with_args(&fname, vec![]).unwrap();

    // Counter incremented twice on arm 0: 0 then 1.
    let count_state = node::state::extract::<i32>(&vm, &[inner_node.index(), count.index()])
        .unwrap()
        .unwrap();
    assert_eq!(count_state, 1);
    assert_eq!(store_val(&vm, sa), Some(1));
}

// Alignment: same inner shape as `parallel_branches` (two parallel Sels -> 4
// branches); asserts Node::branches() == outer meta.branches[inner_node],
// pointwise.
//
//   [In a]        [In b]
//     |             |
//   [Sel1]        [Sel2]
//  o0/  \o1      o0/  \o1
// [A]    [B]    [C]    [D]
#[test]
fn test_graph_nested_branches_align_with_meta() {
    let mut inner = GraphNode::default();
    let inlet_a = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let inlet_b = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
    let sel1 = inner.add_node(Box::new(node_select()) as Box<_>);
    let sel2 = inner.add_node(Box::new(node_select()) as Box<_>);
    let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    let oc = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    let od = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    inner.add_edge(inlet_a, sel1, Edge::from((0, 0)));
    inner.add_edge(inlet_b, sel2, Edge::from((0, 0)));
    inner.add_edge(sel1, oa, Edge::from((0, 0)));
    inner.add_edge(sel1, ob, Edge::from((1, 0)));
    inner.add_edge(sel2, oc, Edge::from((0, 0)));
    inner.add_edge(sel2, od, Edge::from((1, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let declared = inner.branches(ctx);
    assert_eq!(declared.len(), 4);

    let mut g = petgraph::graph::DiGraph::new();
    let va = g.add_node(Box::new(node_int(0)) as Box<dyn DebugNode>);
    let vb = g.add_node(Box::new(node_int(0)) as Box<_>);
    let inner_node = g.add_node(Box::new(inner) as Box<_>);
    g.add_edge(va, inner_node, Edge::from((0, 0)));
    g.add_edge(vb, inner_node, Edge::from((0, 1)));
    let meta = gantz_core::compile::Meta::from_graph(&no_lookup, &g).unwrap();
    let observed = meta.branches.get(&inner_node.index()).unwrap();

    assert_eq!(observed.len(), declared.len());
    for (got, want) in observed.iter().zip(&declared) {
        let node::EvalConf::Set(want) = want else {
            panic!("expected Set")
        };
        assert_eq!(got, want);
    }
}

// Regression: two independent (non-branching) chains form a multi-component
// flow graph. It must compile (previously the single-entry assertion panicked)
// and report no external branching.  branches: []
//
//  [In a]   [In b]
//    |        |
//  [+1]     [+2]
//    |        |
// [Out A]  [Out B]
#[test]
fn test_graph_nested_multi_component_no_branch() {
    let mut inner = GraphNode::default();
    let inlet_a = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let inlet_b = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
    let id_a = inner.add_node(Box::new(node::expr("(+ $x 1)").unwrap()) as Box<_>);
    let id_b = inner.add_node(Box::new(node::expr("(+ $x 2)").unwrap()) as Box<_>);
    let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    inner.add_edge(inlet_a, id_a, Edge::from((0, 0)));
    inner.add_edge(id_a, oa, Edge::from((0, 0)));
    inner.add_edge(inlet_b, id_b, Edge::from((0, 0)));
    inner.add_edge(id_b, ob, Edge::from((0, 0)));

    // No external branching.
    assert_inner_branches(&inner, 2, &[]);

    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let i10 = g.add_node(Box::new(node_int(10)) as Box<_>);
    let i20 = g.add_node(Box::new(node_int(20)) as Box<_>);
    let inner_node = g.add_node(Box::new(inner) as Box<_>);
    let sa = g.add_node(Box::new(node_number()) as Box<_>);
    let sb = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, i10, Edge::from((0, 0)));
    g.add_edge(push, i20, Edge::from((0, 0)));
    g.add_edge(i10, inner_node, Edge::from((0, 0)));
    g.add_edge(i20, inner_node, Edge::from((0, 1)));
    g.add_edge(inner_node, sa, Edge::from((0, 0)));
    g.add_edge(inner_node, sb, Edge::from((1, 0)));
    let vm = compile_and_push(&g, push);
    assert_eq!(store_val(&vm, sa), Some(11)); // 10 + 1
    assert_eq!(store_val(&vm, sb), Some(22)); // 20 + 2
}

// Reconvergent intermediates: both arms pass through a distinct intermediate
// then feed the SAME outlet -> no external branching (exercises is_join via an
// intermediate). branches: []
//
//        [In]
//         |
//       [Sel]
//      o0/  \o1
//    [+10]  [+1]
//       \   /
//      [Out A]
#[test]
fn test_graph_nested_branch_reconvergent_intermediates() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let add10 = inner.add_node(Box::new(node::expr("(+ $x 10)").unwrap()) as Box<_>);
        let add1 = inner.add_node(Box::new(node::expr("(+ $x 1)").unwrap()) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet, select, Edge::from((0, 0)));
        inner.add_edge(select, add10, Edge::from((0, 0)));
        inner.add_edge(add10, outlet, Edge::from((0, 0)));
        inner.add_edge(select, add1, Edge::from((1, 0)));
        inner.add_edge(add1, outlet, Edge::from((0, 0)));
        inner
    };
    assert_inner_branches(&make_inner(), 1, &[]);
    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let store = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0)));
        g.add_edge(inner_node, store, Edge::from((0, 0)));
        store_val(&compile_and_push(&g, push), store)
    };
    assert_eq!(build(0), Some(52)); // 42 + 10
    assert_eq!(build(1), Some(100)); // 99 + 1
}

// Mixed direct/intermediate arms: arm 0 goes straight to its outlet, arm 1 via
// an intermediate. branches: [{A}, {B}]
#[test]
fn test_graph_nested_branch_mixed_direct_intermediate() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let add1 = inner.add_node(Box::new(node::expr("(+ $x 1)").unwrap()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet, select, Edge::from((0, 0)));
        inner.add_edge(select, outlet_a, Edge::from((0, 0)));
        inner.add_edge(select, add1, Edge::from((1, 0)));
        inner.add_edge(add1, outlet_b, Edge::from((0, 0)));
        inner
    };
    assert_inner_branches(&make_inner(), 2, &[&[0], &[1]]);
    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let sa = g.add_node(Box::new(node_number()) as Box<_>);
        let sb = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0)));
        g.add_edge(inner_node, sa, Edge::from((0, 0)));
        g.add_edge(inner_node, sb, Edge::from((1, 0)));
        let vm = compile_and_push(&g, push);
        (store_val(&vm, sa), store_val(&vm, sb))
    };
    assert_eq!(build(0), (Some(42), None)); // direct
    assert_eq!(build(1), (None, Some(100))); // 99 + 1
}

// Chained intermediates: each arm passes through a two-node chain.
// branches: [{A}, {B}]
#[test]
fn test_graph_nested_branch_chained_intermediates() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let a10 = inner.add_node(Box::new(node::expr("(+ $x 10)").unwrap()) as Box<_>);
        let a1 = inner.add_node(Box::new(node::expr("(+ $x 1)").unwrap()) as Box<_>);
        let b10 = inner.add_node(Box::new(node::expr("(+ $x 10)").unwrap()) as Box<_>);
        let b1 = inner.add_node(Box::new(node::expr("(+ $x 1)").unwrap()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet, select, Edge::from((0, 0)));
        inner.add_edge(select, a10, Edge::from((0, 0)));
        inner.add_edge(a10, a1, Edge::from((0, 0)));
        inner.add_edge(a1, outlet_a, Edge::from((0, 0)));
        inner.add_edge(select, b10, Edge::from((1, 0)));
        inner.add_edge(b10, b1, Edge::from((0, 0)));
        inner.add_edge(b1, outlet_b, Edge::from((0, 0)));
        inner
    };
    assert_inner_branches(&make_inner(), 2, &[&[0], &[1]]);
    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let sa = g.add_node(Box::new(node_number()) as Box<_>);
        let sb = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0)));
        g.add_edge(inner_node, sa, Edge::from((0, 0)));
        g.add_edge(inner_node, sb, Edge::from((1, 0)));
        let vm = compile_and_push(&g, push);
        (store_val(&vm, sa), store_val(&vm, sb))
    };
    assert_eq!(build(0), (Some(53), None)); // 42 + 10 + 1
    assert_eq!(build(1), (None, Some(110))); // 99 + 10 + 1
}

// Cascading reconvergence: three sequential branches, but Select A and Select B
// each reconverge at a join, so only Select C affects the outlet. The 2^3 = 8
// inner worlds collapse to just TWO external branches (dedup by outlet set;
// the prior implementation reported 8). branches: [{A}, {B}]
#[test]
fn test_graph_nested_cascading_reconvergence() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let in1 = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let in2 = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let in3 = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let sa = inner.add_node(Box::new(node_select()) as Box<_>);
        let a10 = inner.add_node(Box::new(node::expr("(+ $x 10)").unwrap()) as Box<_>);
        let a20 = inner.add_node(Box::new(node::expr("(+ $x 20)").unwrap()) as Box<_>);
        let joina = inner.add_node(Box::new(node::expr("(begin $x)").unwrap()) as Box<_>);
        let passa = inner.add_node(Box::new(node::expr("(begin $l $r)").unwrap()) as Box<_>);
        let sb = inner.add_node(Box::new(node_select()) as Box<_>);
        let b30 = inner.add_node(Box::new(node::expr("(+ $x 30)").unwrap()) as Box<_>);
        let b40 = inner.add_node(Box::new(node::expr("(+ $x 40)").unwrap()) as Box<_>);
        let joinb = inner.add_node(Box::new(node::expr("(begin $x)").unwrap()) as Box<_>);
        let passb = inner.add_node(Box::new(node::expr("(begin $l $r)").unwrap()) as Box<_>);
        let sc = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(in1, sa, Edge::from((0, 0)));
        inner.add_edge(sa, a10, Edge::from((0, 0)));
        inner.add_edge(sa, a20, Edge::from((1, 0)));
        inner.add_edge(a10, joina, Edge::from((0, 0)));
        inner.add_edge(a20, joina, Edge::from((0, 0)));
        inner.add_edge(joina, passa, Edge::from((0, 0)));
        inner.add_edge(in2, passa, Edge::from((0, 1)));
        inner.add_edge(passa, sb, Edge::from((0, 0)));
        inner.add_edge(sb, b30, Edge::from((0, 0)));
        inner.add_edge(sb, b40, Edge::from((1, 0)));
        inner.add_edge(b30, joinb, Edge::from((0, 0)));
        inner.add_edge(b40, joinb, Edge::from((0, 0)));
        inner.add_edge(joinb, passb, Edge::from((0, 0)));
        inner.add_edge(in3, passb, Edge::from((0, 1)));
        inner.add_edge(passb, sc, Edge::from((0, 0)));
        inner.add_edge(sc, oa, Edge::from((0, 0)));
        inner.add_edge(sc, ob, Edge::from((1, 0)));
        inner
    };
    assert_inner_branches(&make_inner(), 2, &[&[0], &[1]]);
    let build = |c: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let v1 = g.add_node(Box::new(node_int(1)) as Box<_>);
        let v2 = g.add_node(Box::new(node_int(1)) as Box<_>);
        let v3 = g.add_node(Box::new(node_int(c)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let sa = g.add_node(Box::new(node_number()) as Box<_>);
        let sb = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, v1, Edge::from((0, 0)));
        g.add_edge(push, v2, Edge::from((0, 0)));
        g.add_edge(push, v3, Edge::from((0, 0)));
        g.add_edge(v1, inner_node, Edge::from((0, 0)));
        g.add_edge(v2, inner_node, Edge::from((0, 1)));
        g.add_edge(v3, inner_node, Edge::from((0, 2)));
        g.add_edge(inner_node, sa, Edge::from((0, 0)));
        g.add_edge(inner_node, sb, Edge::from((1, 0)));
        let vm = compile_and_push(&g, push);
        (store_val(&vm, sa), store_val(&vm, sb))
    };
    assert_eq!(build(0), (Some(42), None)); // SelectC arm 0 -> A
    assert_eq!(build(1), (None, Some(99))); // SelectC arm 1 -> B
}

// Inner reconvergence + an independent outer branch in the same graph:
// Select1 reconverges to A (always active); Select2 picks B or C.
// branches: [{A, B}, {A, C}]
#[test]
fn test_graph_nested_inner_reconvergence_outer_branching() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let in1 = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let in2 = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let s1 = inner.add_node(Box::new(node_select()) as Box<_>);
        let a10 = inner.add_node(Box::new(node::expr("(+ $x 10)").unwrap()) as Box<_>);
        let a20 = inner.add_node(Box::new(node::expr("(+ $x 20)").unwrap()) as Box<_>);
        let join = inner.add_node(Box::new(node::expr("(begin $x)").unwrap()) as Box<_>);
        let s2 = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(in1, s1, Edge::from((0, 0)));
        inner.add_edge(s1, a10, Edge::from((0, 0)));
        inner.add_edge(s1, a20, Edge::from((1, 0)));
        inner.add_edge(a10, join, Edge::from((0, 0)));
        inner.add_edge(a20, join, Edge::from((0, 0)));
        inner.add_edge(join, oa, Edge::from((0, 0)));
        inner.add_edge(in2, s2, Edge::from((0, 0)));
        inner.add_edge(s2, ob, Edge::from((0, 0)));
        inner.add_edge(s2, oc, Edge::from((1, 0)));
        inner
    };
    assert_inner_branches(&make_inner(), 3, &[&[0, 1], &[0, 2]]);
    let build = |x: i32, y: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let v1 = g.add_node(Box::new(node_int(x)) as Box<_>);
        let v2 = g.add_node(Box::new(node_int(y)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let sa = g.add_node(Box::new(node_number()) as Box<_>);
        let sb = g.add_node(Box::new(node_number()) as Box<_>);
        let sc = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, v1, Edge::from((0, 0)));
        g.add_edge(push, v2, Edge::from((0, 0)));
        g.add_edge(v1, inner_node, Edge::from((0, 0)));
        g.add_edge(v2, inner_node, Edge::from((0, 1)));
        g.add_edge(inner_node, sa, Edge::from((0, 0)));
        g.add_edge(inner_node, sb, Edge::from((1, 0)));
        g.add_edge(inner_node, sc, Edge::from((2, 0)));
        let vm = compile_and_push(&g, push);
        [store_val(&vm, sa), store_val(&vm, sb), store_val(&vm, sc)]
    };
    assert_eq!(build(0, 0), [Some(52), Some(42), None]); // A=42+10, B
    assert_eq!(build(0, 1), [Some(52), None, Some(99)]); // A, C
    assert_eq!(build(1, 0), [Some(119), Some(42), None]); // A=99+20, B
}

// A Static inlet used at branch depth 3: `value` feeds `depth3`, which sits on
// Select3's arm 0. node_inputs_in_scope must keep `value` in scope inside that
// arm even though it enters from outside the arm. Sequential -> 4 branches.
// branches: [{A}, {B}, {C}, {D}]
#[test]
fn test_graph_nested_static_inlet_at_depth_three() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let f1 = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let f2 = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let f3 = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let value = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let s1 = inner.add_node(Box::new(node_select()) as Box<_>);
        let pa = inner.add_node(Box::new(node::expr("(begin $l $r)").unwrap()) as Box<_>);
        let s2 = inner.add_node(Box::new(node_select()) as Box<_>);
        let pb = inner.add_node(Box::new(node::expr("(begin $l $r)").unwrap()) as Box<_>);
        let s3 = inner.add_node(Box::new(node_select()) as Box<_>);
        let d3 = inner.add_node(Box::new(node::expr("(begin $l $r)").unwrap()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let od = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(f1, s1, Edge::from((0, 0)));
        inner.add_edge(s1, oa, Edge::from((0, 0)));
        inner.add_edge(s1, pa, Edge::from((1, 0)));
        inner.add_edge(f2, pa, Edge::from((0, 1)));
        inner.add_edge(pa, s2, Edge::from((0, 0)));
        inner.add_edge(s2, ob, Edge::from((0, 0)));
        inner.add_edge(s2, pb, Edge::from((1, 0)));
        inner.add_edge(f3, pb, Edge::from((0, 1)));
        inner.add_edge(pb, s3, Edge::from((0, 0)));
        inner.add_edge(s3, d3, Edge::from((0, 0)));
        inner.add_edge(value, d3, Edge::from((0, 1)));
        inner.add_edge(d3, oc, Edge::from((0, 0)));
        inner.add_edge(s3, od, Edge::from((1, 0)));
        inner
    };
    assert_inner_branches(&make_inner(), 4, &[&[0], &[1], &[2], &[3]]);
    let build = |a: i32, b: i32, c: i32, v: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let n1 = g.add_node(Box::new(node_int(a)) as Box<_>);
        let n2 = g.add_node(Box::new(node_int(b)) as Box<_>);
        let n3 = g.add_node(Box::new(node_int(c)) as Box<_>);
        let nv = g.add_node(Box::new(node_int(v)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let st: Vec<_> = (0..4)
            .map(|_| g.add_node(Box::new(node_number()) as Box<_>))
            .collect();
        for n in [n1, n2, n3, nv] {
            g.add_edge(push, n, Edge::from((0, 0)));
        }
        g.add_edge(n1, inner_node, Edge::from((0, 0)));
        g.add_edge(n2, inner_node, Edge::from((0, 1)));
        g.add_edge(n3, inner_node, Edge::from((0, 2)));
        g.add_edge(nv, inner_node, Edge::from((0, 3)));
        for (k, &s) in st.iter().enumerate() {
            g.add_edge(inner_node, s, Edge::from((k as u16, 0)));
        }
        let vm = compile_and_push(&g, push);
        st.iter().map(|&s| store_val(&vm, s)).collect::<Vec<_>>()
    };
    assert_eq!(build(0, 9, 9, 7), [Some(42), None, None, None]); // f1==0 -> A
    assert_eq!(build(9, 0, 9, 7), [None, Some(42), None, None]); // f2==0 -> B
    assert_eq!(build(9, 9, 0, 7), [None, None, Some(7), None]); // f3==0 -> depth3 = value 7
    assert_eq!(build(9, 9, 9, 7), [None, None, None, Some(99)]); // all !=0 -> D
}

// Three independent parallel branches -> 2^3 = 8 external branches (a 3-component
// flow graph). branches: all 8 of {A|B} x {C|D} x {E|F}.
#[test]
fn test_graph_nested_multi_branch_three() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let i1 = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let i2 = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let i3 = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let s1 = inner.add_node(Box::new(node_select()) as Box<_>);
        let s2 = inner.add_node(Box::new(node_select()) as Box<_>);
        let s3 = inner.add_node(Box::new(node_select()) as Box<_>);
        let mut add =
            |n: i32| inner.add_node(Box::new(node::expr(format!("(+ $x {n})")).unwrap()) as Box<_>);
        let (a10, a20, a30, a40, a50, a60) = (add(10), add(20), add(30), add(40), add(50), add(60));
        let o: Vec<_> = (0..6)
            .map(|_| inner.add_node(Box::new(node::graph::Outlet) as Box<_>))
            .collect();
        inner.add_edge(i1, s1, Edge::from((0, 0)));
        inner.add_edge(i2, s2, Edge::from((0, 0)));
        inner.add_edge(i3, s3, Edge::from((0, 0)));
        for (sel, lo, hi, ol, oh) in [
            (s1, a10, a20, o[0], o[1]),
            (s2, a30, a40, o[2], o[3]),
            (s3, a50, a60, o[4], o[5]),
        ] {
            inner.add_edge(sel, lo, Edge::from((0, 0)));
            inner.add_edge(lo, ol, Edge::from((0, 0)));
            inner.add_edge(sel, hi, Edge::from((1, 0)));
            inner.add_edge(hi, oh, Edge::from((0, 0)));
        }
        inner
    };
    assert_inner_branches(
        &make_inner(),
        6,
        &[
            &[0, 2, 4],
            &[0, 2, 5],
            &[0, 3, 4],
            &[0, 3, 5],
            &[1, 2, 4],
            &[1, 2, 5],
            &[1, 3, 4],
            &[1, 3, 5],
        ],
    );
    let build = |a: i32, b: i32, c: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let n1 = g.add_node(Box::new(node_int(a)) as Box<_>);
        let n2 = g.add_node(Box::new(node_int(b)) as Box<_>);
        let n3 = g.add_node(Box::new(node_int(c)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let st: Vec<_> = (0..6)
            .map(|_| g.add_node(Box::new(node_number()) as Box<_>))
            .collect();
        for n in [n1, n2, n3] {
            g.add_edge(push, n, Edge::from((0, 0)));
        }
        g.add_edge(n1, inner_node, Edge::from((0, 0)));
        g.add_edge(n2, inner_node, Edge::from((0, 1)));
        g.add_edge(n3, inner_node, Edge::from((0, 2)));
        for (k, &s) in st.iter().enumerate() {
            g.add_edge(inner_node, s, Edge::from((k as u16, 0)));
        }
        let vm = compile_and_push(&g, push);
        st.iter().map(|&s| store_val(&vm, s)).collect::<Vec<_>>()
    };
    // (0,0,0): A=42+10, C=42+30, E=42+50 fire; B,D,F dead.
    assert_eq!(
        build(0, 0, 0),
        [Some(52), None, Some(72), None, Some(92), None]
    );
    // (1,1,1): B=99+20, D=99+40, F=99+60.
    assert_eq!(
        build(1, 1, 1),
        [None, Some(119), None, Some(139), None, Some(159)]
    );
    // (0,1,0): A, D, E.
    assert_eq!(
        build(0, 1, 0),
        [Some(52), None, None, Some(139), Some(92), None]
    );
}

// ===========================================================================
// Push-through-outlet branching tests.
//
// Here the `push_eval` lives *inside* the nested graph and propagates *out*
// through the graph's outlets via an interior branch. The bridged graph node
// therefore acts as a branch node in the parent for that entrypoint, so the
// parent only evaluates downstream of the outlets the taken arm produced.
// Each test's INNER graph (with the push inside) is sketched above it; `[Sel]`
// is `node_select` (input ==0 -> o0(42), else -> o1(99)).
// ===========================================================================

// Divergent push-through: each arm drives its own outlet -> its own outer store.
//
//   INNER: [push]->[int(sel)]->[Sel]        OUTER: [inner]
//                            o0/  \o1               o0/  \o1
//                        [OutA]   [OutB]      [store_a] [store_b]
#[test]
fn test_graph_nested_push_through_divergent_branch() {
    let build = |sel: i32| {
        let mut inner = GraphNode::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(push, int, Edge::from((0, 0)));
        inner.add_edge(int, select, Edge::from((0, 0)));
        inner.add_edge(select, outlet_a, Edge::from((0, 0)));
        inner.add_edge(select, outlet_b, Edge::from((1, 0)));

        let ctx = node::MetaCtx::new(&no_lookup);
        let push_n = inner[push].n_outputs(ctx) as u8;

        let mut g = petgraph::graph::DiGraph::new();
        let inner_node = g.add_node(Box::new(inner) as Box<dyn DebugNode>);
        let store_a = g.add_node(Box::new(node_number()) as Box<_>);
        let store_b = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(inner_node, store_a, Edge::from((0, 0)));
        g.add_edge(inner_node, store_b, Edge::from((1, 0)));

        let vm = compile_and_push_nested(&g, vec![inner_node.index(), push.index()], push_n);
        (store_val(&vm, store_a), store_val(&vm, store_b))
    };

    assert_eq!(build(0), (Some(42), None)); // arm 0 -> outlet A -> store_a
    assert_eq!(build(1), (None, Some(99))); // arm 1 -> outlet B -> store_b
}

// Dead-arm push-through: arm 1 leaves the select output unconnected, so it
// produces nothing and no outer store is written.
//
//   INNER: [push]->[int(sel)]->[Sel]        OUTER: [inner]
//                            o0|  \o1 (dead)         o0|
//                        [OutA]    x              [store]
#[test]
fn test_graph_nested_push_through_dead_arm() {
    let build = |sel: i32| {
        let mut inner = GraphNode::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(push, int, Edge::from((0, 0)));
        inner.add_edge(int, select, Edge::from((0, 0)));
        inner.add_edge(select, outlet, Edge::from((0, 0)));
        // Select output 1 left unconnected (dead arm).

        let ctx = node::MetaCtx::new(&no_lookup);
        let push_n = inner[push].n_outputs(ctx) as u8;

        let mut g = petgraph::graph::DiGraph::new();
        let inner_node = g.add_node(Box::new(inner) as Box<dyn DebugNode>);
        let store = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(inner_node, store, Edge::from((0, 0)));

        let vm = compile_and_push_nested(&g, vec![inner_node.index(), push.index()], push_n);
        store_val(&vm, store)
    };

    assert_eq!(build(0), Some(42)); // arm 0 -> outlet -> store
    assert_eq!(build(1), None); // arm 1 -> dead, store never evaluated
}

// Multi-output-arm push-through: arm 0 fires two outlets, arm 1 fires one.
//
//   INNER: [push]->[int(sel)]->[Branch]     arm 0 -> o0,o1 (values 10,20)
//                          o0/o1|\o2         arm 1 -> o2    (value 30)
//                     [A][B][C]              OUTER stores: a,b,c
#[test]
fn test_graph_nested_push_through_multi_outlet_arm() {
    let branch3 = || {
        node::branch(
            "(if (= 0 $x) (list 0 (list 10 20)) (list 1 30))",
            vec![
                node::Conns::try_from([true, true, false]).unwrap(),
                node::Conns::try_from([false, false, true]).unwrap(),
            ],
        )
        .unwrap()
    };
    let build = |sel: i32| {
        let mut inner = GraphNode::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner.add_node(Box::new(node_int(sel)) as Box<_>);
        let br = inner.add_node(Box::new(branch3()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let outlet_c = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(push, int, Edge::from((0, 0)));
        inner.add_edge(int, br, Edge::from((0, 0)));
        inner.add_edge(br, outlet_a, Edge::from((0, 0)));
        inner.add_edge(br, outlet_b, Edge::from((1, 0)));
        inner.add_edge(br, outlet_c, Edge::from((2, 0)));

        let ctx = node::MetaCtx::new(&no_lookup);
        let push_n = inner[push].n_outputs(ctx) as u8;

        let mut g = petgraph::graph::DiGraph::new();
        let inner_node = g.add_node(Box::new(inner) as Box<dyn DebugNode>);
        let store_a = g.add_node(Box::new(node_number()) as Box<_>);
        let store_b = g.add_node(Box::new(node_number()) as Box<_>);
        let store_c = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(inner_node, store_a, Edge::from((0, 0)));
        g.add_edge(inner_node, store_b, Edge::from((1, 0)));
        g.add_edge(inner_node, store_c, Edge::from((2, 0)));

        let vm = compile_and_push_nested(&g, vec![inner_node.index(), push.index()], push_n);
        (
            store_val(&vm, store_a),
            store_val(&vm, store_b),
            store_val(&vm, store_c),
        )
    };

    assert_eq!(build(0), (Some(10), Some(20), None)); // arm 0 -> a,b
    assert_eq!(build(1), (None, None, Some(30))); // arm 1 -> c
}

// Two-level push-through: the push is inside the *innermost* graph; its branch
// propagates out through two levels of outlets. The middle graph branches
// because the inner one does (multi-level pattern threading).
//
//   INNER2: [push]->[int(sel)]->[Sel]->{oa,ob}
//   INNER1: [inner2]->{ox,oy}
//   OUTER:  [inner1]->{store_x, store_y}
#[test]
fn test_graph_nested_push_through_two_levels() {
    let build = |sel: i32| {
        let mut inner2 = GraphNode::default();
        let push = inner2.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner2.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = inner2.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner2.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let ob = inner2.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner2.add_edge(push, int, Edge::from((0, 0)));
        inner2.add_edge(int, select, Edge::from((0, 0)));
        inner2.add_edge(select, oa, Edge::from((0, 0)));
        inner2.add_edge(select, ob, Edge::from((1, 0)));

        let ctx = node::MetaCtx::new(&no_lookup);
        let push_n = inner2[push].n_outputs(ctx) as u8;

        let mut inner1 = GraphNode::default();
        let inner2_node = inner1.add_node(Box::new(inner2) as Box<dyn DebugNode>);
        let ox = inner1.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let oy = inner1.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner1.add_edge(inner2_node, ox, Edge::from((0, 0)));
        inner1.add_edge(inner2_node, oy, Edge::from((1, 0)));

        let mut g = petgraph::graph::DiGraph::new();
        let inner1_node = g.add_node(Box::new(inner1) as Box<dyn DebugNode>);
        let store_x = g.add_node(Box::new(node_number()) as Box<_>);
        let store_y = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(inner1_node, store_x, Edge::from((0, 0)));
        g.add_edge(inner1_node, store_y, Edge::from((1, 0)));

        let vm = compile_and_push_nested(
            &g,
            vec![inner1_node.index(), inner2_node.index(), push.index()],
            push_n,
        );
        (store_val(&vm, store_x), store_val(&vm, store_y))
    };

    assert_eq!(build(0), (Some(42), None));
    assert_eq!(build(1), (None, Some(99)));
}

// Reconvergent push-through: a divergent interior branch whose two arms feed
// distinct outlets that re-join at a single outer store - a phi across the
// bridge boundary. The store always fires, with the taken arm's value.
//
//   INNER: [push]->[int(sel)]->[Sel]->{oa(o0), ob(o1)}
//   OUTER: inner.o0 -\
//          inner.o1 --> [store]
#[test]
fn test_graph_nested_push_through_reconvergent_branch() {
    let build = |sel: i32| {
        let mut inner = GraphNode::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(push, int, Edge::from((0, 0)));
        inner.add_edge(int, select, Edge::from((0, 0)));
        inner.add_edge(select, oa, Edge::from((0, 0)));
        inner.add_edge(select, ob, Edge::from((1, 0)));

        let ctx = node::MetaCtx::new(&no_lookup);
        let push_n = inner[push].n_outputs(ctx) as u8;

        let mut g = petgraph::graph::DiGraph::new();
        let inner_node = g.add_node(Box::new(inner) as Box<dyn DebugNode>);
        let store = g.add_node(Box::new(node_number()) as Box<_>);
        // Both arms route to the same store (phi reconvergence across the bridge).
        g.add_edge(inner_node, store, Edge::from((0, 0)));
        g.add_edge(inner_node, store, Edge::from((1, 0)));

        let vm = compile_and_push_nested(&g, vec![inner_node.index(), push.index()], push_n);
        store_val(&vm, store)
    };

    assert_eq!(build(0), Some(42)); // arm 0 -> outlet A -> store
    assert_eq!(build(1), Some(99)); // arm 1 -> outlet B -> store
}

// Push-through branch alongside an always-active outlet: the push also drives a
// constant-fed outlet that every arm produces, so its store always fires while
// the branch arms route to their own stores.
//
//   INNER: [push]-+->[int(sel)]->[Sel]->{oa(o0), ob(o1)}
//                 +->[int(7)]---------->{oc(o2)}   (always produced)
//   OUTER: inner.{o0,o1,o2} -> {store_a, store_b, store_c}
#[test]
fn test_graph_nested_push_through_branch_with_constant_outlet() {
    let build = |sel: i32| {
        let mut inner = GraphNode::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let seven = inner.add_node(Box::new(node_int(7)) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        let outlet_c = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(push, int, Edge::from((0, 0)));
        inner.add_edge(int, select, Edge::from((0, 0)));
        inner.add_edge(select, outlet_a, Edge::from((0, 0)));
        inner.add_edge(select, outlet_b, Edge::from((1, 0)));
        inner.add_edge(push, seven, Edge::from((0, 0)));
        inner.add_edge(seven, outlet_c, Edge::from((0, 0)));

        let ctx = node::MetaCtx::new(&no_lookup);
        let push_n = inner[push].n_outputs(ctx) as u8;

        let mut g = petgraph::graph::DiGraph::new();
        let inner_node = g.add_node(Box::new(inner) as Box<dyn DebugNode>);
        let store_a = g.add_node(Box::new(node_number()) as Box<_>);
        let store_b = g.add_node(Box::new(node_number()) as Box<_>);
        let store_c = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(inner_node, store_a, Edge::from((0, 0)));
        g.add_edge(inner_node, store_b, Edge::from((1, 0)));
        g.add_edge(inner_node, store_c, Edge::from((2, 0)));

        let vm = compile_and_push_nested(&g, vec![inner_node.index(), push.index()], push_n);
        (
            store_val(&vm, store_a),
            store_val(&vm, store_b),
            store_val(&vm, store_c),
        )
    };

    assert_eq!(build(0), (Some(42), None, Some(7))); // arm 0 -> a, plus constant c
    assert_eq!(build(1), (None, Some(99), Some(7))); // arm 1 -> b, plus constant c
}

// ===========================================================================
// Multi-root branch-reconvergence ordering tests.
//
// A single entrypoint with two flow roots, where one root branches and its arms
// reconverge at a join that ALSO consumes the other root's value. The join is
// the branch's post-dominator yet depends on a second root: previously the
// branch root was emitted first and the join referenced the second root's output
// before it was defined (a `FreeIdentifier` VM error). Fixed by `order_roots`
// (emit a producing component before a consuming one) plus destructuring a
// terminal block's last node so its outputs are available cross-component.
// ===========================================================================

// Build, compile and run `g` from two push sources in one entrypoint; returns
// the VM for state queries.
fn run_two_push<N: DebugNode + ?Sized>(
    g: &petgraph::graph::DiGraph<Box<N>, Edge>,
    a: (Vec<usize>, u8),
    b: (Vec<usize>, u8),
) -> Engine {
    let ep = entrypoint::from_sources([push_source(a.0, a.1), push_source(b.0, b.1)]);
    let module = gantz_core::compile::module(&no_lookup, g, &[ep.clone()]).unwrap();
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, g, &[], &mut vm);
    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
    vm
}

// Two push roots; Root A branches and its arms reconverge at `add`, which also
// takes Root B's `int(20)`.
//
//   ROOT A: [push_a]->[int sel]->[select]   o0,o1 -> add.$l (phi)
//   ROOT B: [push_b]->[int 20] -------------------> add.$r ;  add -> store
#[test]
fn test_multiroot_branch_join_external_pred() {
    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push_a = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = g.add_node(Box::new(node_select()) as Box<_>);
        let push_b = g.add_node(Box::new(node_push()) as Box<_>);
        let twenty = g.add_node(Box::new(node_int(20)) as Box<_>);
        let add = g.add_node(Box::new(node::expr("(+ $l $r)").unwrap()) as Box<_>);
        let store = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push_a, int, Edge::from((0, 0)));
        g.add_edge(int, select, Edge::from((0, 0)));
        g.add_edge(select, add, Edge::from((0, 0))); // arm 0 -> add.l
        g.add_edge(select, add, Edge::from((1, 0))); // arm 1 -> add.l
        g.add_edge(push_b, twenty, Edge::from((0, 0)));
        g.add_edge(twenty, add, Edge::from((0, 1))); // -> add.r
        g.add_edge(add, store, Edge::from((0, 0)));
        let n = g[push_a].n_outputs(node::MetaCtx::new(&no_lookup)) as u8;
        let vm = run_two_push(&g, (vec![push_a.index()], n), (vec![push_b.index()], n));
        store_val(&vm, store)
    };
    assert_eq!(build(0), Some(62)); // arm 0: 42 + 20
    assert_eq!(build(1), Some(119)); // arm 1: 99 + 20
}

// Sibling-shape guard: the same logical graph, but the predecessor's nodes are
// added first so the topological `last`-chaining linearizes `int(20)` ahead of
// the branch within a single component. This shape already worked; it verifies
// the terminal-destructure / `order_roots` changes don't regress the linearized
// form.
#[test]
fn test_multiroot_branch_join_external_pred_reversed() {
    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        // Root B first (lower ids).
        let push_b = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let twenty = g.add_node(Box::new(node_int(20)) as Box<_>);
        // Root A second.
        let push_a = g.add_node(Box::new(node_push()) as Box<_>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = g.add_node(Box::new(node_select()) as Box<_>);
        let add = g.add_node(Box::new(node::expr("(+ $l $r)").unwrap()) as Box<_>);
        let store = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push_b, twenty, Edge::from((0, 0)));
        g.add_edge(twenty, add, Edge::from((0, 1))); // -> add.r
        g.add_edge(push_a, int, Edge::from((0, 0)));
        g.add_edge(int, select, Edge::from((0, 0)));
        g.add_edge(select, add, Edge::from((0, 0))); // arm 0 -> add.l
        g.add_edge(select, add, Edge::from((1, 0))); // arm 1 -> add.l
        g.add_edge(add, store, Edge::from((0, 0)));
        let n = g[push_a].n_outputs(node::MetaCtx::new(&no_lookup)) as u8;
        let vm = run_two_push(&g, (vec![push_a.index()], n), (vec![push_b.index()], n));
        store_val(&vm, store)
    };
    assert_eq!(build(0), Some(62));
    assert_eq!(build(1), Some(119));
}

// The same branch-join shape one level down: inside a nested graph, inlet_a
// branches and its arms reconverge at `add`, which also takes inlet_b. Here the
// inlets linearize into one component, so it already worked - this guards the
// node-style `nested_expr` codegen path against the terminal-destructure change.
//
//   INNER: [inlet_a]->[select] o0,o1 -> add.$l ;  [inlet_b] -> add.$r ;  add -> outlet
//   OUTER: [push]->[int sel]->inlet_a ;  [push]->[int 20]->inlet_b ;  inner -> store
#[test]
fn test_nested_branch_join_external_inlet() {
    let make_inner = || {
        let mut inner = GraphNode::default();
        let inlet_a = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
        let inlet_b = inner.add_node(Box::new(node::graph::Inlet) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let add = inner.add_node(Box::new(node::expr("(+ $l $r)").unwrap()) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
        inner.add_edge(inlet_a, select, Edge::from((0, 0)));
        inner.add_edge(select, add, Edge::from((0, 0)));
        inner.add_edge(select, add, Edge::from((1, 0)));
        inner.add_edge(inlet_b, add, Edge::from((0, 1)));
        inner.add_edge(add, outlet, Edge::from((0, 0)));
        inner
    };
    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = g.add_node(Box::new(node_int(sel)) as Box<_>);
        let twenty = g.add_node(Box::new(node_int(20)) as Box<_>);
        let inner_node = g.add_node(Box::new(make_inner()) as Box<_>);
        let store = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(push, int, Edge::from((0, 0)));
        g.add_edge(push, twenty, Edge::from((0, 0)));
        g.add_edge(int, inner_node, Edge::from((0, 0))); // -> inlet_a (input 0)
        g.add_edge(twenty, inner_node, Edge::from((0, 1))); // -> inlet_b (input 1)
        g.add_edge(inner_node, store, Edge::from((0, 0)));
        store_val(&compile_and_push(&g, push), store)
    };
    assert_eq!(build(0), Some(62));
    assert_eq!(build(1), Some(119));
}

// An `Outlet` at the *root* level (no enclosing graph node) must compile and be
// ignored: there is no parent to read its value, so it is a no-op while the rest
// of the graph still evaluates.
//
//    --------
//    | push | // push_eval
//    -+------
//     |
//    -+----
//    | 42 |
//    -+----
//     |
//    -+--------
//    | number | (stores received value in state)
//    -+--------
//     |
//    -+--------
//    | Outlet | (root-level: ignored)
//    ----------
#[test]
fn test_graph_root_outlet_connected() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int = g.add_node(Box::new(node_int(42)) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    let outlet = g.add_node(Box::new(node::graph::Outlet) as Box<_>);
    g.add_edge(push, int, Edge::from((0, 0)));
    g.add_edge(int, store, Edge::from((0, 0)));
    g.add_edge(store, outlet, Edge::from((0, 0)));

    // Compiles, runs, and the upstream `number` still receives the value even
    // though the root outlet leads nowhere.
    assert_eq!(store_val(&compile_and_push(&g, push), store), Some(42));
}

// A *disconnected* `Outlet` at the root level (no incoming edge) is never
// reached by the flow, so it emits nothing and the rest of the graph evaluates
// normally.
#[test]
fn test_graph_root_outlet_disconnected() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int = g.add_node(Box::new(node_int(42)) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    let _outlet = g.add_node(Box::new(node::graph::Outlet) as Box<_>);
    g.add_edge(push, int, Edge::from((0, 0)));
    g.add_edge(int, store, Edge::from((0, 0)));

    assert_eq!(store_val(&compile_and_push(&g, push), store), Some(42));
}

// === pd+ optional-input ($?) cold/hot inlet tests ===
//
// Emulates Pure Data's stateful `+`: a left "hot" inlet (always outputs the
// sum) and a right "cold" inlet (updates internal state only, no output). The
// node is a nested `GraphNode` whose interior is a single `Branch` reading two
// optional inputs (`$?l`, `$?r`). The cold/hot behaviour relies on the inner
// branch seeing `(None)` for the inlet that did not fire - which only works once
// the active-input-set is propagated into the nested graph's interior.

// The pd+ Branch: cold (`$?r`) sets state; hot (`$?l`) outputs `left + state`.
// Branch 0 activates the single output (hot fired), branch 1 activates nothing
// (cold-only, no output).
fn pd_plus_branch() -> node::Branch {
    node::Branch::new(
        r#"
        (begin
          (if (Some? $?r) (set! state (Some->value $?r)) '())
          (if (Some? $?l)
            (list 0 (+ (Some->value $?l) (if (number? state) state 0)))
            (list 1 '())))
        "#,
        vec![
            node::Conns::try_from([true]).unwrap(),
            node::Conns::try_from([false]).unwrap(),
        ],
    )
    .unwrap()
}

// A pd+ nested graph. Input 0 = left/hot inlet, input 1 = right/cold inlet,
// output 0 = the sum. Returns the graph and the inner branch node id (for state
// queries via the path `[pd_node, branch]`).
//
//    [In L]   [In R]
//       \       /        (In R -> $?r branch input 0, In L -> $?l branch input 1)
//      -+-------+-
//      | Branch |
//      -+--------
//       |
//    -+--------
//    | Outlet |
//    ----------
fn pd_plus() -> (GraphNode<Box<dyn DebugNode>>, usize) {
    let mut g = GraphNode::default();
    let inlet_l = g.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let inlet_r = g.add_node(Box::new(node::graph::Inlet) as Box<_>);
    let branch = g.add_node(Box::new(pd_plus_branch()) as Box<_>);
    let outlet = g.add_node(Box::new(node::graph::Outlet) as Box<_>);
    g.add_edge(inlet_r, branch, Edge::from((0, 0))); // In R -> $?r (branch input 0)
    g.add_edge(inlet_l, branch, Edge::from((0, 1))); // In L -> $?l (branch input 1)
    g.add_edge(branch, outlet, Edge::from((0, 0)));
    (g, branch.index())
}

// Compile `g` and register fns, returning a VM ready to be pushed.
fn compile_only<N: DebugNode + ?Sized>(g: &petgraph::graph::DiGraph<Box<N>, Edge>) -> Engine {
    let eps = push_pull_entrypoints(&no_lookup, g);
    let module = gantz_core::compile::module(&no_lookup, g, &eps).unwrap();
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, g, &[], &mut vm);
    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }
    vm
}

// Fire the entrypoint that pushes from `push`.
fn push_from<N: DebugNode + ?Sized>(
    vm: &mut Engine,
    g: &petgraph::graph::DiGraph<Box<N>, Edge>,
    push: petgraph::graph::NodeIndex,
) {
    let ctx = node::MetaCtx::new(&no_lookup);
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}

// Root graph wiring a left push -> left value, a right push -> right value, into
// a pd+ node whose output feeds a `store`. Returns (graph, left_push,
// right_push, pd, branch_id, store).
type PdPlusRoot = (
    petgraph::graph::DiGraph<Box<dyn DebugNode>, Edge>,
    petgraph::graph::NodeIndex,
    petgraph::graph::NodeIndex,
    petgraph::graph::NodeIndex,
    usize,
    petgraph::graph::NodeIndex,
);
fn pd_plus_root(left: i32, right: i32) -> PdPlusRoot {
    let (inner, branch_ix) = pd_plus();
    let mut g = petgraph::graph::DiGraph::new();
    let left_push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let right_push = g.add_node(Box::new(node_push()) as Box<_>);
    let left_val = g.add_node(Box::new(node_int(left)) as Box<_>);
    let right_val = g.add_node(Box::new(node_int(right)) as Box<_>);
    let pd = g.add_node(Box::new(inner) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(left_push, left_val, Edge::from((0, 0)));
    g.add_edge(right_push, right_val, Edge::from((0, 0)));
    g.add_edge(left_val, pd, Edge::from((0, 0))); // left -> pd input 0 (hot)
    g.add_edge(right_val, pd, Edge::from((0, 1))); // right -> pd input 1 (cold)
    g.add_edge(pd, store, Edge::from((0, 0)));
    (g, left_push, right_push, pd, branch_ix, store)
}

fn branch_state(vm: &Engine, pd: petgraph::graph::NodeIndex, branch_ix: usize) -> Option<i32> {
    node::state::extract::<i32>(vm, &[pd.index(), branch_ix])
        .ok()
        .flatten()
}

// Pushing ONLY the cold (right) inlet must set state and produce no output -
// and crucially must NOT raise `+ expects a number, found '()`.
#[test]
fn test_nested_pd_plus_cold_only() {
    let (g, _left_push, right_push, pd, branch_ix, store) = pd_plus_root(10, 5);
    let mut vm = compile_only(&g);
    push_from(&mut vm, &g, right_push);
    assert_eq!(branch_state(&vm, pd, branch_ix), Some(5), "cold sets state");
    assert_eq!(store_val(&vm, store), None, "cold produces no output");
}

// Cold (right) then hot (left): state is seeded by the cold push, the hot push
// outputs `left + state`.
#[test]
fn test_nested_pd_plus_hot_after_cold() {
    let (g, left_push, right_push, pd, branch_ix, store) = pd_plus_root(10, 5);
    let mut vm = compile_only(&g);
    push_from(&mut vm, &g, right_push); // cold: state = 5
    push_from(&mut vm, &g, left_push); // hot: 10 + 5 = 15
    assert_eq!(branch_state(&vm, pd, branch_ix), Some(5));
    assert_eq!(store_val(&vm, store), Some(15));
}

// Firing both inlets in one push: the cold value updates state first, then the
// hot arm outputs `left + state`.
#[test]
fn test_nested_pd_plus_both() {
    let (inner, branch_ix) = pd_plus();
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let left_val = g.add_node(Box::new(node_int(10)) as Box<_>);
    let right_val = g.add_node(Box::new(node_int(5)) as Box<_>);
    let pd = g.add_node(Box::new(inner) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, left_val, Edge::from((0, 0)));
    g.add_edge(push, right_val, Edge::from((0, 0)));
    g.add_edge(left_val, pd, Edge::from((0, 0)));
    g.add_edge(right_val, pd, Edge::from((0, 1)));
    g.add_edge(pd, store, Edge::from((0, 0)));

    let mut vm = compile_only(&g);
    push_from(&mut vm, &g, push);
    assert_eq!(branch_state(&vm, pd, branch_ix), Some(5));
    assert_eq!(store_val(&vm, store), Some(15)); // 10 + 5
}

// The user's feedback graph: a `pd+` accumulator looped through a `store` Number.
// `hot` drives the left inlet; `pd+ -> store -> pd+`'s cold (right) inlet closes
// the cycle. One push is a 2-pass loop: pd+{hot} outputs `hot + state` -> store
// holds it -> pd+{cold} sets state to it and exits. So each push adds `hot` to the
// running total held in pd+'s internal state (and mirrored in `store`).
#[test]
fn test_pd_plus_feedback_loop() {
    let (inner, branch_ix) = pd_plus();
    let mut g = petgraph::graph::DiGraph::new();
    let hot = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let hot_val = g.add_node(Box::new(node_int(1)) as Box<_>);
    let pd = g.add_node(Box::new(inner) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(hot, hot_val, Edge::from((0, 0)));
    g.add_edge(hot_val, pd, Edge::from((0, 0))); // hot -> pd input 0 (hot/left)
    g.add_edge(store, pd, Edge::from((0, 1))); // store -> pd input 1 (cold/right)
    g.add_edge(pd, store, Edge::from((0, 0))); // pd -> store closes the cycle

    let mut vm = compile_only(&g);
    push_from(&mut vm, &g, hot); // 1: state 0 -> 1
    assert_eq!(branch_state(&vm, pd, branch_ix), Some(1));
    assert_eq!(store_val(&vm, store), Some(1));
    push_from(&mut vm, &g, hot); // 2: state 1 -> 2
    assert_eq!(branch_state(&vm, pd, branch_ix), Some(2));
    assert_eq!(store_val(&vm, store), Some(2));
}

// A sequence of pushes across multiple entrypoint calls: two cold updates then
// a hot output, exercising state persistence.
#[test]
fn test_nested_pd_plus_sequence() {
    let (inner, branch_ix) = pd_plus();
    let mut g = petgraph::graph::DiGraph::new();
    let cold_a = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let cold_b = g.add_node(Box::new(node_push()) as Box<_>);
    let hot = g.add_node(Box::new(node_push()) as Box<_>);
    let cold_a_val = g.add_node(Box::new(node_int(3)) as Box<_>);
    let cold_b_val = g.add_node(Box::new(node_int(7)) as Box<_>);
    let hot_val = g.add_node(Box::new(node_int(10)) as Box<_>);
    let pd = g.add_node(Box::new(inner) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(cold_a, cold_a_val, Edge::from((0, 0)));
    g.add_edge(cold_b, cold_b_val, Edge::from((0, 0)));
    g.add_edge(hot, hot_val, Edge::from((0, 0)));
    g.add_edge(cold_a_val, pd, Edge::from((0, 1))); // cold -> right
    g.add_edge(cold_b_val, pd, Edge::from((0, 1))); // cold -> right
    g.add_edge(hot_val, pd, Edge::from((0, 0))); // hot -> left
    g.add_edge(pd, store, Edge::from((0, 0)));

    let mut vm = compile_only(&g);
    push_from(&mut vm, &g, cold_a); // state = 3
    assert_eq!(branch_state(&vm, pd, branch_ix), Some(3));
    assert_eq!(store_val(&vm, store), None);
    push_from(&mut vm, &g, cold_b); // state = 7
    assert_eq!(branch_state(&vm, pd, branch_ix), Some(7));
    assert_eq!(store_val(&vm, store), None);
    push_from(&mut vm, &g, hot); // 10 + 7 = 17
    assert_eq!(store_val(&vm, store), Some(17));
}

// The same Branch at top level (where `$?` already works) and nested must
// behave identically.
fn pd_plus_top_level(
    left: i32,
    right: i32,
) -> (
    petgraph::graph::DiGraph<Box<dyn DebugNode>, Edge>,
    petgraph::graph::NodeIndex,
    petgraph::graph::NodeIndex,
    petgraph::graph::NodeIndex,
) {
    let mut g = petgraph::graph::DiGraph::new();
    let left_push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let right_push = g.add_node(Box::new(node_push()) as Box<_>);
    let left_val = g.add_node(Box::new(node_int(left)) as Box<_>);
    let right_val = g.add_node(Box::new(node_int(right)) as Box<_>);
    let branch = g.add_node(Box::new(pd_plus_branch()) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(left_push, left_val, Edge::from((0, 0)));
    g.add_edge(right_push, right_val, Edge::from((0, 0)));
    g.add_edge(right_val, branch, Edge::from((0, 0))); // right -> $?r (input 0)
    g.add_edge(left_val, branch, Edge::from((0, 1))); // left -> $?l (input 1)
    g.add_edge(branch, store, Edge::from((0, 0)));
    (g, left_push, right_push, store)
}

#[test]
fn test_pd_plus_top_level_vs_nested_equivalence() {
    // Nested: cold(5) then hot(10).
    let (gn, ln, rn, _pd, _bix, sn) = pd_plus_root(10, 5);
    let mut vmn = compile_only(&gn);
    push_from(&mut vmn, &gn, rn);
    let cold_n = store_val(&vmn, sn);
    push_from(&mut vmn, &gn, ln);
    let hot_n = store_val(&vmn, sn);

    // Top level: same sequence.
    let (gt, lt, rt, st) = pd_plus_top_level(10, 5);
    let mut vmt = compile_only(&gt);
    push_from(&mut vmt, &gt, rt);
    let cold_t = store_val(&vmt, st);
    push_from(&mut vmt, &gt, lt);
    let hot_t = store_val(&vmt, st);

    assert_eq!(cold_n, cold_t);
    assert_eq!(hot_n, hot_t);
    assert_eq!(cold_n, None);
    assert_eq!(hot_n, Some(15));
}

// The reduced inner-branch variants (cold push -> i10, hot push -> i01) must be
// DEFINED in the module, not just the all-connected i11. Guards the conf
// post-pass and call/def agreement.
#[test]
fn test_nested_pd_plus_emits_reduced_variant() {
    let (g, _l, _r, pd, branch_ix, _store) = pd_plus_root(10, 5);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();
    let text: String = module
        .iter()
        .map(|f| f.to_pretty(100))
        .collect::<Vec<_>>()
        .join("\n");
    let prefix = format!("node-fn-{}:{}-", pd.index(), branch_ix);
    assert!(
        text.contains(&format!("{prefix}i10-o1")),
        "missing reduced inner branch variant {prefix}i10-o1"
    );
    assert!(
        text.contains(&format!("{prefix}i01-o1")),
        "missing reduced inner branch variant {prefix}i01-o1"
    );
}

// pd+ wrapped in a second `GraphNode`. Returns (outer, pd_id_in_outer,
// branch_id_in_pd). Outer input 0 -> pd left (hot), input 1 -> pd right (cold).
fn pd_plus_wrapped() -> (GraphNode<Box<dyn DebugNode>>, usize, usize) {
    let (pd_inner, branch_ix) = pd_plus();
    let mut outer = GraphNode::default();
    let inlet_l = outer.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let inlet_r = outer.add_node(Box::new(node::graph::Inlet) as Box<_>);
    let pd = outer.add_node(Box::new(pd_inner) as Box<_>);
    let outlet = outer.add_node(Box::new(node::graph::Outlet) as Box<_>);
    outer.add_edge(inlet_l, pd, Edge::from((0, 0))); // outer in 0 -> pd in 0 (hot)
    outer.add_edge(inlet_r, pd, Edge::from((0, 1))); // outer in 1 -> pd in 1 (cold)
    outer.add_edge(pd, outlet, Edge::from((0, 0)));
    (outer, pd.index(), branch_ix)
}

// Two-level nesting: cold-only push from the outside must propagate the reduced
// active-set through BOTH graph layers (outer + pd) so the grandchild branch
// sees `(None)` for the hot inlet - no error, state set, no output.
#[test]
fn test_nested_pd_plus_two_levels() {
    let (outer, pd_in_outer, branch_in_pd) = pd_plus_wrapped();
    let mut g = petgraph::graph::DiGraph::new();
    let left_push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let right_push = g.add_node(Box::new(node_push()) as Box<_>);
    let left_val = g.add_node(Box::new(node_int(10)) as Box<_>);
    let right_val = g.add_node(Box::new(node_int(5)) as Box<_>);
    let outer_node = g.add_node(Box::new(outer) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(left_push, left_val, Edge::from((0, 0)));
    g.add_edge(right_push, right_val, Edge::from((0, 0)));
    g.add_edge(left_val, outer_node, Edge::from((0, 0))); // -> outer in 0 (hot)
    g.add_edge(right_val, outer_node, Edge::from((0, 1))); // -> outer in 1 (cold)
    g.add_edge(outer_node, store, Edge::from((0, 0)));

    let branch_path = [outer_node.index(), pd_in_outer, branch_in_pd];
    let mut vm = compile_only(&g);

    // Cold-only: state set deep inside, no output.
    push_from(&mut vm, &g, right_push);
    assert_eq!(
        node::state::extract::<i32>(&vm, &branch_path)
            .ok()
            .flatten(),
        Some(5),
    );
    assert_eq!(store_val(&vm, store), None);

    // Hot: outputs 10 + 5 = 15.
    push_from(&mut vm, &g, left_push);
    assert_eq!(store_val(&vm, store), Some(15));
}

// A wrapper `GraphNode` exposing ONLY pd+'s hot inlet, leaving the cold inlet
// permanently unconnected. Even when the wrapper is invoked all-active, its
// interior invokes pd+ with a statically reduced active-set (only the hot inlet
// wired), whose inner branch variant must still be defined. Guards the conf
// post-pass recursing through an all-active parent into a reduced child.
#[test]
fn test_nested_reduced_child_under_active_parent() {
    let (pd_inner, _branch_ix) = pd_plus();
    let mut outer = GraphNode::default();
    let inlet = outer.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let pd = outer.add_node(Box::new(pd_inner) as Box<_>);
    let outlet = outer.add_node(Box::new(node::graph::Outlet) as Box<_>);
    outer.add_edge(inlet, pd, Edge::from((0, 0))); // outer inlet -> pd left (hot)
    outer.add_edge(pd, outlet, Edge::from((0, 0))); // pd right inlet left unconnected

    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let val = g.add_node(Box::new(node_int(10)) as Box<_>);
    let outer_node = g.add_node(Box::new(outer) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, val, Edge::from((0, 0)));
    g.add_edge(val, outer_node, Edge::from((0, 0)));
    g.add_edge(outer_node, store, Edge::from((0, 0)));

    let mut vm = compile_only(&g);
    push_from(&mut vm, &g, push);
    // The cold inlet is never wired => `$?r` is `(None)` => state stays 0 =>
    // output = 10 + 0. Reaching this without a free-identifier error proves the
    // reduced inner branch variant was defined and called.
    assert_eq!(store_val(&vm, store), Some(10));
}

// Push-through reaching a nested-optional child: an interior push fires only the
// hot inlet of a nested pd+, whose output propagates out through the wrapper's
// outlet. Exercises a reduced inner-branch variant reached via push-through (not
// via the wrapper's own inlets).
#[test]
fn test_push_through_into_nested_optional_hot() {
    let (c_inner, _branch_ix) = pd_plus();
    let mut l = GraphNode::default();
    let p = l.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let val = l.add_node(Box::new(node_int(10)) as Box<_>);
    let c = l.add_node(Box::new(c_inner) as Box<_>);
    let outlet = l.add_node(Box::new(node::graph::Outlet) as Box<_>);
    l.add_edge(p, val, Edge::from((0, 0)));
    l.add_edge(val, c, Edge::from((0, 0))); // -> C hot (input 0); C cold (input 1) unwired
    l.add_edge(c, outlet, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let push_n = l[p].n_outputs(ctx) as u8;

    let mut g = petgraph::graph::DiGraph::new();
    let l_node = g.add_node(Box::new(l) as Box<dyn DebugNode>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(l_node, store, Edge::from((0, 0)));

    let vm = compile_and_push_nested(&g, vec![l_node.index(), p.index()], push_n);
    assert_eq!(store_val(&vm, store), Some(10)); // 10 + state(0)
}

// Push-through into a side-effect-only nested-optional child: an interior push
// fires only the cold inlet of a nested pd+ that produces no output and feeds no
// outlet. The all-connected interior flow never reaches C, so C's reduced branch
// variant is discoverable only from the interior push's flow - which the
// all-connected nested_fg + node-style reduction miss.
#[test]
fn test_push_through_into_nested_optional_sideeffect() {
    let (c_inner, branch_ix) = pd_plus();
    let mut l = GraphNode::default();
    let p = l.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let val = l.add_node(Box::new(node_int(5)) as Box<_>);
    let c = l.add_node(Box::new(c_inner) as Box<_>);
    l.add_edge(p, val, Edge::from((0, 0)));
    l.add_edge(val, c, Edge::from((0, 1))); // -> C cold inlet (input 1); no outlet

    let ctx = node::MetaCtx::new(&no_lookup);
    let push_n = l[p].n_outputs(ctx) as u8;
    let c_in_l = c.index();

    let mut g = petgraph::graph::DiGraph::new();
    let l_node = g.add_node(Box::new(l) as Box<dyn DebugNode>);

    let vm = compile_and_push_nested(&g, vec![l_node.index(), p.index()], push_n);
    assert_eq!(
        node::state::extract::<i32>(&vm, &[l_node.index(), c_in_l, branch_ix])
            .ok()
            .flatten(),
        Some(5),
    );
}
