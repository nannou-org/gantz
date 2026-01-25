//! Tests related to the nesting of graphs.

use gantz_core::{
    Edge, ROOT_STATE,
    compile::push_eval_fn_name,
    node::{self, GraphNode, Node, WithPushEval},
};
use std::fmt::Debug;
use steel::{SteelVal, steel_vm::engine::Engine};

fn node_push() -> node::Push<(), node::Expr> {
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
trait DebugNode: Debug + Node<()> {}
impl<T> DebugNode for T where T: Debug + Node<()> {}

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

    // No need to share an environment between nodes for this test.
    let env = ();

    // Generate the module, which should have just one top-level expr for `push`.
    let module = gantz_core::compile::module(&env, &gb).unwrap();

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state vars.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&env, &gb, &[], &mut vm);

    // Register the fns.
    for f in module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Call the `push` eval function.
    vm.call_function_by_name_with_args(&push_eval_fn_name(&[push.index()]), vec![])
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

    // No need to share an environment between nodes for this test.
    let env = ();

    // Generate the module.
    let module = gantz_core::compile::module(&env, &gb).unwrap();

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state vars.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&env, &gb, &[], &mut vm);

    // Register the fns.
    for f in module {
        println!("{}\n", f.to_pretty(100));
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Increment the nested counter by pushing evaluation.
    // The first is `0`, the second is `1`.
    vm.call_function_by_name_with_args(&push_eval_fn_name(&[push.index()]), vec![])
        .unwrap();
    vm.call_function_by_name_with_args(&push_eval_fn_name(&[push.index()]), vec![])
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
#[test]
#[ignore = "requires #78, #77"]
fn test_graph_nested_push_eval() {
    // GRAPH A
    let mut ga = GraphNode::default();
    let push = ga.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int = ga.add_node(Box::new(node_int(42)) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet) as Box<_>);
    ga.add_edge(push, int, Edge::from((0, 0)));
    ga.add_edge(int, outlet, Edge::from((0, 0)));

    // Graph B.
    let mut gb = petgraph::graph::DiGraph::new();
    let graph_a = gb.add_node(Box::new(ga) as Box<dyn DebugNode>);
    let number = gb.add_node(Box::new(node_number()) as Box<_>);
    gb.add_edge(graph_a, number, Edge::from((0, 0)));

    // No need to share an environment between nodes for this test.
    let env = ();

    // Generate the module.
    let module = gantz_core::compile::module(&env, &gb).unwrap();

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state vars.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&env, &gb, &[], &mut vm);

    // Register the fns.
    for f in module {
        println!("{}\n", f.to_pretty(100));
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Call the nested push node's eval fn.
    let push_path = [graph_a.index(), push.index()];
    vm.call_function_by_name_with_args(&push_eval_fn_name(&push_path), vec![])
        .unwrap();

    // Now check that the outlet's state is `42`.
    let outlet_state = node::state::extract::<u32>(&vm, &[graph_a.index(), outlet.index()])
        .expect("failed to extract outlet state")
        .expect("outlet state was `None`");
    assert_eq!(outlet_state, 42);

    // Check that the number in the root graph was updated from the outlet.
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was `None`");
    assert_eq!(number_state, 42);
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

    let env = ();

    // Generate the module.
    let module = gantz_core::compile::module(&env, &gb).unwrap();

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state vars.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&env, &gb, &[], &mut vm);

    // Register the fns.
    for f in module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Call the `push` eval function - should compute 10 - 3 = 7
    vm.call_function_by_name_with_args(&push_eval_fn_name(&[push.index()]), vec![])
        .unwrap();
}
