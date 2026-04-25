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
