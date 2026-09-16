//! Tests related to the nesting of graphs.

use gantz_core::{
    Edge, ROOT_STATE,
    compile::{entry_fn_name, entrypoint, push_pull_entrypoints, push_source},
    node::{self, Node, WithPushEval},
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

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

type Nested = node::graph::Graph<Box<dyn DebugNode>>;

fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

// A simple test for nested graph support. Nesting is the core method of
// abstraction in gantz.
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

    let mut ga = Nested::default();
    let inlet_a = ga.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
    let inlet_b = ga.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
    let mul = ga.add_node(Box::new(node_mul()) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    ga.add_edge(inlet_a, mul, Edge::from((0, 0)));
    ga.add_edge(inlet_b, mul, Edge::from((0, 1)));
    ga.add_edge(mul, outlet, Edge::from((0, 0)));

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

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &gb);
    let module = gantz_core::compile::module(&no_lookup, &gb, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();

    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &gb, &[], &mut vm);

    for f in module {
        vm.run(f.to_pretty(100)).unwrap();
    }

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
// Push evaluation from the root graph B's `push` node. Then check the state
// of the `number` node to see the incremented value.
#[test]
fn test_graph_nested_counter() {
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

    let mut ga = Nested::default();
    let inlet = ga.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
    let counter = ga.add_node(Box::new(counter) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    ga.add_edge(inlet, counter, Edge::from((0, 0)));
    ga.add_edge(counter, outlet, Edge::from((0, 0)));

    let mut gb = petgraph::graph::DiGraph::new();
    let push = gb.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let graph_a = gb.add_node(Box::new(ga) as Box<_>);
    let number = gb.add_node(Box::new(node_number()) as Box<_>);
    gb.add_edge(push, graph_a, Edge::from((0, 0)));
    gb.add_edge(graph_a, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &gb);
    let module = gantz_core::compile::module(&no_lookup, &gb, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();

    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &gb, &[], &mut vm);

    for f in module {
        println!("{}\n", f.to_pretty(100));
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Push twice. The counter yields `0` then `1`.
    let ep = entrypoint::push(vec![push.index()], gb[push].n_outputs(ctx) as u8);
    let fn_name = entry_fn_name(&ep.id());
    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();
    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();

    let counter_state = node::state::extract::<u32>(&vm, &[graph_a.index(), counter.index()])
        .expect("failed to extract counter state")
        .expect("counter state was `None`");
    assert_eq!(counter_state, 1);

    // Outlets are stateless and pass through their input value. The value
    // flows through to the downstream `number` node.
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was `None`");
    assert_eq!(number_state, 1);
}

// Push evaluation from a node inside a nested graph.
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
// The push fires inside graph A and drives evaluation to the number node,
// which stores the received value. Entrypoints can target nodes inside
// nested graphs.
#[test]
fn test_graph_nested_push_eval() {
    let mut ga = Nested::default();
    let push = ga.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let num = ga.add_node(Box::new(node_number()) as Box<_>);
    ga.add_edge(push, num, Edge::from((0, 0)));

    // Compute push connection count before moving `ga` into `gb`.
    let ctx = node::MetaCtx::new(&no_lookup);
    let push_n_outputs = ga[push].n_outputs(ctx) as u8;

    // Graph B only contains graph A. No outlet propagation is needed.
    let mut gb = petgraph::graph::DiGraph::new();
    let graph_a = gb.add_node(Box::new(ga) as Box<dyn DebugNode>);

    let ep = entrypoint::from_source(push_source(
        vec![graph_a.index(), push.index()],
        push_n_outputs,
    ));

    let module =
        gantz_core::compile::module(&no_lookup, &gb, &[ep.clone()], &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &gb, &[], &mut vm);

    for f in &module {
        println!("{}\n", f.to_pretty(100));
        vm.run(f.to_pretty(100)).unwrap();
    }

    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    // node_push outputs '(), which is not a number, so number's state stays
    // at its initial void value. The extract call confirms the state path
    // exists and the eval ran without error.
    let _num_state = node::state::extract_value(&vm, &[graph_a.index(), num.index()])
        .expect("failed to extract number state from nested graph");
}

// Inlet bindings must work when node indices do not match inlet positions.
// The inlets are not the first nodes in the graph.
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
    let mut ga = Nested::default();

    // Add dummy nodes first to offset the inlet indices.
    let _dummy1 = ga.add_node(Box::new(node_int(999)) as Box<dyn DebugNode>);
    let _dummy2 = ga.add_node(Box::new(node_int(998)) as Box<_>);

    // The inlets get indices 2 and 3.
    let inlet_a = ga.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
    let inlet_b = ga.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);

    let sub = ga.add_node(Box::new(node::expr("(- $l $r)").unwrap()) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);

    ga.add_edge(inlet_a, sub, Edge::from((0, 0)));
    ga.add_edge(inlet_b, sub, Edge::from((0, 1)));
    ga.add_edge(sub, outlet, Edge::from((0, 0)));

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

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &gb);
    let module = gantz_core::compile::module(&no_lookup, &gb, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();

    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &gb, &[], &mut vm);

    for f in module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // The push computes 10 - 3 = 7.
    let ep = entrypoint::push(vec![push.index()], gb[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}

// Push evaluation inside a nested graph propagates through its outlet to
// downstream nodes in the outer graph.
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
// The push fires inside graph A. Value 42 flows through the outlet to the
// number node in the outer graph.
#[test]
fn test_graph_nested_push_through_outlet() {
    let mut ga = Nested::default();
    let push = ga.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let forty_two = ga.add_node(Box::new(node_int(42)) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    ga.add_edge(push, forty_two, Edge::from((0, 0)));
    ga.add_edge(forty_two, outlet, Edge::from((0, 0)));

    // Compute push connection count before moving `ga` into `gb`.
    let ctx = node::MetaCtx::new(&no_lookup);
    let push_n_outputs = ga[push].n_outputs(ctx) as u8;

    let mut gb = petgraph::graph::DiGraph::new();
    let graph_a = gb.add_node(Box::new(ga) as Box<dyn DebugNode>);
    let number = gb.add_node(Box::new(node_number()) as Box<_>);
    gb.add_edge(graph_a, number, Edge::from((0, 0)));

    let ep = entrypoint::from_source(push_source(
        vec![graph_a.index(), push.index()],
        push_n_outputs,
    ));

    let module =
        gantz_core::compile::module(&no_lookup, &gb, &[ep.clone()], &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &gb, &[], &mut vm);

    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    // The number node received 42 through the outlet.
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was None");
    assert_eq!(number_state, 42);
}

// A nested graph with multiple outlets returns a list. The outer graph
// destructures it with `define-values`.
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
    let mut inner = Nested::default();
    let inlet_a = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
    let inlet_b = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
    let outlet_a = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    let outlet_b = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    inner.add_edge(inlet_a, outlet_a, Edge::from((0, 0)));
    inner.add_edge(inlet_b, outlet_b, Edge::from((0, 0)));

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
    let module =
        gantz_core::compile::module(&no_lookup, &outer, &eps, &Default::default()).unwrap();

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

// Nested push evaluation propagates through multiple outlets.
//
// INNER GRAPH:
//    push -> int(10) -> outlet_a
//                    -> outlet_b (via int(20))
//
// OUTER GRAPH:
//    inner_graph -> num_a (from outlet 0)
//               -> num_b (from outlet 1)
//
// The push fires inside the inner graph. Both outlet values propagate to the
// outer graph's number nodes.
#[test]
fn test_graph_nested_push_through_outlet_multi() {
    let mut inner = Nested::default();
    let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let ten = inner.add_node(Box::new(node_int(10)) as Box<_>);
    let twenty = inner.add_node(Box::new(node_int(20)) as Box<_>);
    let outlet_a = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    let outlet_b = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

    let module =
        gantz_core::compile::module(&no_lookup, &outer, &[ep.clone()], &Default::default())
            .unwrap();

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

// Nested push evaluation propagates through two levels of nesting.
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
// The push fires in the innermost graph. Value 99 propagates through two
// outlet levels to the outer number node.
#[test]
fn test_graph_nested_push_through_outlet_deep() {
    let mut innermost = Nested::default();
    let push = innermost.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let ninety_nine = innermost.add_node(Box::new(node_int(99)) as Box<_>);
    let outlet_inner = innermost.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    innermost.add_edge(push, ninety_nine, Edge::from((0, 0)));
    innermost.add_edge(ninety_nine, outlet_inner, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let push_n_outputs = innermost[push].n_outputs(ctx) as u8;

    let mut middle = Nested::default();
    let innermost_node = middle.add_node(Box::new(innermost) as Box<dyn DebugNode>);
    let outlet_mid = middle.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    middle.add_edge(innermost_node, outlet_mid, Edge::from((0, 0)));

    let mut outer = petgraph::graph::DiGraph::new();
    let middle_node = outer.add_node(Box::new(middle) as Box<dyn DebugNode>);
    let number = outer.add_node(Box::new(node_number()) as Box<_>);
    outer.add_edge(middle_node, number, Edge::from((0, 0)));

    let ep = entrypoint::from_source(push_source(
        vec![middle_node.index(), innermost_node.index(), push.index()],
        push_n_outputs,
    ));

    let module =
        gantz_core::compile::module(&no_lookup, &outer, &[ep.clone()], &Default::default())
            .unwrap();

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

// `push_pull_entrypoints` must discover push eval nodes inside nested graphs.
// This mirrors an UpdateBang node inside a nested graph placed in a top-level
// graph through a NamedRef.
//
// INNER GRAPH:
//    push -> int(42) -> outlet
//
// OUTER GRAPH:
//    inner_graph -> number
//
// On the outer graph it must find the push node inside the inner graph and
// create an entrypoint with path [graph_a, push].
#[test]
fn test_push_pull_entrypoints_discovers_nested_push() {
    let mut inner = Nested::default();
    let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let forty_two = inner.add_node(Box::new(node_int(42)) as Box<_>);
    let outlet = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    inner.add_edge(push, forty_two, Edge::from((0, 0)));
    inner.add_edge(forty_two, outlet, Edge::from((0, 0)));

    let mut outer = petgraph::graph::DiGraph::new();
    let graph_a = outer.add_node(Box::new(inner) as Box<dyn DebugNode>);
    let number = outer.add_node(Box::new(node_number()) as Box<_>);
    outer.add_edge(graph_a, number, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &outer);
    assert!(
        !eps.is_empty(),
        "push_pull_entrypoints should discover the nested push eval node"
    );

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

    // The generated module includes the entry fn for this entrypoint. Value
    // 42 flows through the outlet to number.
    let module =
        gantz_core::compile::module(&no_lookup, &outer, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &outer, &[], &mut vm);

    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

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

// Two nested graph nodes that share a multi-source entrypoint both propagate
// through their outlets to the parent graph. This mirrors two NamedRef
// "deltams" nodes in a top-level graph. Both contain an UpdateBang and
// combine into one multi-source entrypoint.
//
// INNER GRAPH (shared by both):
//    push -> int(10) -> outlet
//
// OUTER GRAPH:
//    graph_a -> num_a
//    graph_b -> num_b
//
// A single multi-source entrypoint fires push inside both graph_a and graph_b.
// Both outlets propagate and write 10 to num_a and num_b.
#[test]
fn test_graph_nested_multi_source_outlet_propagation() {
    let make_inner = || {
        let mut inner = Nested::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let ten = inner.add_node(Box::new(node_int(10)) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        inner.add_edge(push, ten, Edge::from((0, 0)));
        inner.add_edge(ten, outlet, Edge::from((0, 0)));
        (inner, push)
    };

    let (inner_a, push_a) = make_inner();
    let (inner_b, push_b) = make_inner();

    let ctx = node::MetaCtx::new(&no_lookup);
    let push_n_outputs = inner_a[push_a].n_outputs(ctx) as u8;

    let mut outer = petgraph::graph::DiGraph::new();
    let graph_a = outer.add_node(Box::new(inner_a) as Box<dyn DebugNode>);
    let graph_b = outer.add_node(Box::new(inner_b) as Box<dyn DebugNode>);
    let num_a = outer.add_node(Box::new(node_number()) as Box<_>);
    let num_b = outer.add_node(Box::new(node_number()) as Box<_>);
    outer.add_edge(graph_a, num_a, Edge::from((0, 0)));
    outer.add_edge(graph_b, num_b, Edge::from((0, 0)));

    let ep = entrypoint::from_sources([
        push_source(vec![graph_a.index(), push_a.index()], push_n_outputs),
        push_source(vec![graph_b.index(), push_b.index()], push_n_outputs),
    ]);

    let module =
        gantz_core::compile::module(&no_lookup, &outer, &[ep.clone()], &Default::default())
            .unwrap();

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

// A multi-source entrypoint with sources at different nesting levels. One is
// a direct push source at the root. The other is a nested push source inside
// a graph node that propagates through an outlet. This mirrors a top-level
// UpdateBang and a NamedRef "deltams" with its own UpdateBang combined into
// one entrypoint.
//
// INNER GRAPH:
//    push_inner -> int(10) -> outlet
//
// OUTER GRAPH:
//    graph_node --(outlet)--> add (input 0)
//    push_outer -> int(20) -> add (input 1)
//    add -> number
//
// Both pushes fire in a single entrypoint. The graph_node outlet value 10
// and the push_outer chain value 20 reach add, whose result is stored in
// number.
#[test]
fn test_graph_nested_mixed_level_multi_source() {
    let mut inner = Nested::default();
    let push_inner = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let ten = inner.add_node(Box::new(node_int(10)) as Box<_>);
    let outlet = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    inner.add_edge(push_inner, ten, Edge::from((0, 0)));
    inner.add_edge(ten, outlet, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let push_inner_n = inner[push_inner].n_outputs(ctx) as u8;

    let mut outer = petgraph::graph::DiGraph::new();
    let graph_node = outer.add_node(Box::new(inner) as Box<dyn DebugNode>);
    let push_outer = outer.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let twenty = outer.add_node(Box::new(node_int(20)) as Box<_>);
    let add = outer.add_node(Box::new(node::expr("(+ $l $r)").unwrap()) as Box<_>);
    let number = outer.add_node(Box::new(node_number()) as Box<_>);
    outer.add_edge(graph_node, add, Edge::from((0, 0))); // outlet(10) -> add input 0
    outer.add_edge(push_outer, twenty, Edge::from((0, 0))); // push -> int(20)
    outer.add_edge(twenty, add, Edge::from((0, 1))); // int(20) -> add input 1
    outer.add_edge(add, number, Edge::from((0, 0)));

    let push_outer_n = outer[push_outer].n_outputs(ctx) as u8;

    let ep = entrypoint::from_sources([
        push_source(vec![graph_node.index(), push_inner.index()], push_inner_n),
        push_source(vec![push_outer.index()], push_outer_n),
    ]);

    let module =
        gantz_core::compile::module(&no_lookup, &outer, &[ep.clone()], &Default::default())
            .unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &outer, &[], &mut vm);

    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    // The outlet gives 10 and the push_outer chain gives 20, so add stores 30.
    let val = node::state::extract::<i32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was None");
    assert_eq!(val, 30);
}

// Nested-graph branching tests.
//
// A nested graph whose interior branches must report that branching to the
// outer graph through `Node::branches`. The outer graph then only evaluates
// the downstream of outlets the taken inner branch produced.
//
// Each test's inner graph is sketched above it. `branches()` lists the sets
// of outputs active per external branch. `{}` is a branch that produces
// nothing. Diagram legend:
//   [In]     inlet            [Out X]  outlet
//   [Sel]    node_select: input ==0 -> o0(42), else -> o1(99)
//   oN       branch output N    /  \   the two arms of a branch
// The outer graph is uniform. A push feeds int values into the nested graph,
// and a `number` store per output records the value it receives.

// A 1-input, 2-output branch primitive. Input 0 routes 42 to output 0.
// Anything else routes 99 to output 1.
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

// Assert that `inner.branches()` reports exactly `expected`, in any order.
// Each entry lists the output indices active in that branch.
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

// Compile and run `g` from `push`. Returns the VM for state queries.
fn compile_and_push<N: DebugNode + ?Sized>(
    g: &petgraph::graph::DiGraph<Box<N>, Edge>,
    push: petgraph::graph::NodeIndex,
) -> Engine {
    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, g);
    let module = gantz_core::compile::module(&no_lookup, g, &eps, &Default::default()).unwrap();
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

// `Some(v)` if the store node holds a number. `None` means it was never
// evaluated.
fn store_val(vm: &Engine, store: petgraph::graph::NodeIndex) -> Option<i32> {
    node::state::extract::<i32>(vm, &[store.index()])
        .ok()
        .flatten()
}

// Compile and run `g` from a `push_eval` nested at `path`, for example
// `[graph_node, push]`. Returns the VM for state queries. Unlike
// `compile_and_push`, the push lives inside a nested graph and propagates out
// through that graph's outlets.
fn compile_and_push_nested<N: DebugNode + ?Sized>(
    g: &petgraph::graph::DiGraph<Box<N>, Edge>,
    path: Vec<usize>,
    push_n_outputs: u8,
) -> Engine {
    let ep = entrypoint::from_source(push_source(path, push_n_outputs));
    let module =
        gantz_core::compile::module(&no_lookup, g, &[ep.clone()], &Default::default()).unwrap();
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

// A divergent branch. Each arm routes to its own outlet. branches: [{A}, {B}]
//
//        [In]
//         |
//       [Sel]
//      o0/  \o1
//   [Out A]  [Out B]
#[test]
fn test_graph_nested_divergent_branch() {
    let make_inner = || {
        let mut inner = Nested::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        inner.add_edge(inlet, select, Edge::from((0, 0)));
        inner.add_edge(select, outlet_a, Edge::from((0, 0)));
        inner.add_edge(select, outlet_b, Edge::from((1, 0)));
        inner
    };

    // Arm 0 fires only outlet A and arm 1 only outlet B.
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

    // sel == 0 takes arm 0, so only store_a is written with 42.
    assert_eq!(build(0), (Some(42), None));
    // sel != 0 takes arm 1, so only store_b is written with 99.
    assert_eq!(build(1), (None, Some(99)));
}

// Reconvergent. Both arms feed the same outlet, so it is always produced and
// there is no external branching. The value still differs per arm.
// branches: []
//
//        [In]
//         |
//       [Sel]
//      o0\  /o1
//       [Out A]
#[test]
fn test_graph_nested_reconvergent_branch() {
    let make_inner = || {
        let mut inner = Nested::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        inner.add_edge(inlet, select, Edge::from((0, 0)));
        inner.add_edge(select, outlet, Edge::from((0, 0)));
        inner.add_edge(select, outlet, Edge::from((1, 0)));
        inner
    };

    // No external branching. The outlet is always produced.
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

    // The outlet is always written. The value differs per arm.
    assert_eq!(build(0), Some(42));
    assert_eq!(build(1), Some(99));
}

// A dead arm. Arm 1's output is unconnected, so it produces nothing.
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
        let mut inner = Nested::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        inner.add_edge(inlet, select, Edge::from((0, 0)));
        inner.add_edge(select, outlet, Edge::from((0, 0)));
        // Output 1 of Select stays unconnected as a dead arm.
        inner
    };

    // Two patterns. The dead arm is empty and the other is {outlet}.
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

// Per-arm intermediates. Each arm transforms its value before its outlet.
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
        let mut inner = Nested::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let add10 = inner.add_node(Box::new(node::expr("(+ $x 10)").unwrap()) as Box<_>);
        let add20 = inner.add_node(Box::new(node::expr("(+ $x 20)").unwrap()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// A multi-output arm. Arm 0 fires two outputs with a list value. Arm 1 fires
// one.
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
        let mut inner = Nested::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let br = inner.add_node(Box::new(branch3()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let outlet_c = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Parallel branches. Two independent Selects give a Cartesian product of 4
// branches. This exercises a multi-root inner flow graph.
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
        let mut inner = Nested::default();
        let inlet_a = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let inlet_b = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
        let sel1 = inner.add_node(Box::new(node_select()) as Box<_>);
        let sel2 = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let od = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        inner.add_edge(inlet_a, sel1, Edge::from((0, 0)));
        inner.add_edge(inlet_b, sel2, Edge::from((0, 0)));
        inner.add_edge(sel1, oa, Edge::from((0, 0)));
        inner.add_edge(sel1, ob, Edge::from((1, 0)));
        inner.add_edge(sel2, oc, Edge::from((0, 0)));
        inner.add_edge(sel2, od, Edge::from((1, 0)));
        inner
    };

    // Outputs A=0, B=1, C=2, D=3. 4 Cartesian arms.
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

// Sequential branches. Gate exists only under Sel1's arm 0, so the result is
// pruned to 3 branches instead of the Cartesian 4. branches: [{A}, {B}, {C}]
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
    // A 2-input outer select. $sel picks the arm and $val is passed on arm 0.
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
        let mut inner = Nested::default();
        let inlet_sel =
            inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let inlet_val = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
        let sel1 = inner.add_node(Box::new(outer_sel()) as Box<_>);
        let gate = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        inner.add_edge(inlet_sel, sel1, Edge::from((0, 0))); // $sel
        inner.add_edge(inlet_val, sel1, Edge::from((0, 1))); // $val
        inner.add_edge(sel1, gate, Edge::from((0, 0))); // arm 0 -> gate input
        inner.add_edge(sel1, oc, Edge::from((1, 0))); // arm 1 -> C
        inner.add_edge(gate, oa, Edge::from((0, 0)));
        inner.add_edge(gate, ob, Edge::from((1, 0)));
        inner
    };

    // A=0, B=1, C=2. Pruned to {A}, {B}, {C}.
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

    // sel==0 reaches gate with $val. Gate routes by whether $val == 0.
    assert_eq!(build(0, 0), [Some(42), None, None]); // gate arm 0 -> A
    assert_eq!(build(0, 5), [None, Some(99), None]); // gate arm 1 -> B
    assert_eq!(build(9, 0), [None, None, Some(88)]); // sel arm 1 -> C
}

// A branch after a join. The branch is reachable from two inlet chains, but
// must be assigned once per world. That gives two branches, not four.
// branches: [{A}, {B}]
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
        let mut inner = Nested::default();
        let inlet_l = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let inlet_r = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
        let sum = inner.add_node(Box::new(node::expr("(+ $a $b)").unwrap()) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        inner.add_edge(inlet_l, sum, Edge::from((0, 0)));
        inner.add_edge(inlet_r, sum, Edge::from((0, 1)));
        inner.add_edge(sum, select, Edge::from((0, 0)));
        inner.add_edge(select, oa, Edge::from((0, 0)));
        inner.add_edge(select, ob, Edge::from((1, 0)));
        inner
    };

    // Two branches, not four. The join-fed branch is assigned once.
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

// A static outlet. C is fed by a constant with no inlet, so it is active in
// every branch. branches: [{A, C}, {B, C}]
//
//    [In]          [const 123]
//     |                 |
//   [Sel]               |        (independent chain ->
//  o0/  \o1             |         always produced)
// [A]    [B]        [Out C]
#[test]
fn test_graph_nested_branch_with_constant_outlet() {
    let make_inner = || {
        let mut inner = Nested::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let konst = inner.add_node(Box::new(node::expr("123").unwrap()) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// A three-arm branch. One branch node with three arms, each to its own outlet.
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
        let mut inner = Nested::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let br = inner.add_node(Box::new(branch3()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Two levels of nesting. Branching propagates outward through both.
// inner1.branches: [{X}, {Y}]
//
//  inner2:            inner1 (wraps inner2):
//    [In]               [In]
//     |                  |
//   [Sel]            [inner2]   <- itself a branching nested graph
//  o0/ \o1           o0/  \o1
// [A]   [B]      [Out X]  [Out Y]
#[test]
fn test_graph_nested_branch_two_levels() {
    let make_inner2 = || {
        let mut g = Nested::default();
        let inlet = g.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let select = g.add_node(Box::new(node_select()) as Box<_>);
        let oa = g.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let ob = g.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        g.add_edge(inlet, select, Edge::from((0, 0)));
        g.add_edge(select, oa, Edge::from((0, 0)));
        g.add_edge(select, ob, Edge::from((1, 0)));
        g
    };
    let make_inner1 = || {
        let mut g = Nested::default();
        let inlet = g.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let inner2 = g.add_node(Box::new(make_inner2()) as Box<_>);
        let ox = g.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let oy = g.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// A stateful node on a branch arm. Its state persists across pushes that take
// arm 0.
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
    let mut inner = Nested::default();
    let inlet = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
    let select = inner.add_node(Box::new(node_select()) as Box<_>);
    let count = inner.add_node(Box::new(counter()) as Box<_>);
    let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();
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

    // The counter incremented twice on arm 0, to 0 then 1.
    let count_state = node::state::extract::<i32>(&vm, &[inner_node.index(), count.index()])
        .unwrap()
        .unwrap();
    assert_eq!(count_state, 1);
    assert_eq!(store_val(&vm, sa), Some(1));
}

// Alignment. The same inner shape as `parallel_branches` with two parallel
// Sels and 4 branches. Asserts Node::branches() equals the outer
// meta.branches[inner_node] pointwise.
//
//   [In a]        [In b]
//     |             |
//   [Sel1]        [Sel2]
//  o0/  \o1      o0/  \o1
// [A]    [B]    [C]    [D]
#[test]
fn test_graph_nested_branches_align_with_meta() {
    let mut inner = Nested::default();
    let inlet_a = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
    let inlet_b = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
    let sel1 = inner.add_node(Box::new(node_select()) as Box<_>);
    let sel2 = inner.add_node(Box::new(node_select()) as Box<_>);
    let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    let oc = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    let od = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Two independent chains form a multi-component flow graph. It must compile
// and report no external branching. branches: []
//
//  [In a]   [In b]
//    |        |
//  [+1]     [+2]
//    |        |
// [Out A]  [Out B]
#[test]
fn test_graph_nested_multi_component_no_branch() {
    let mut inner = Nested::default();
    let inlet_a = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
    let inlet_b = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
    let id_a = inner.add_node(Box::new(node::expr("(+ $x 1)").unwrap()) as Box<_>);
    let id_b = inner.add_node(Box::new(node::expr("(+ $x 2)").unwrap()) as Box<_>);
    let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Reconvergent intermediates. Both arms pass through a distinct intermediate,
// then feed the same outlet. No external branching. branches: []
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
        let mut inner = Nested::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let add10 = inner.add_node(Box::new(node::expr("(+ $x 10)").unwrap()) as Box<_>);
        let add1 = inner.add_node(Box::new(node::expr("(+ $x 1)").unwrap()) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Mixed direct and intermediate arms. Arm 0 goes straight to its outlet. Arm
// 1 goes through an intermediate. branches: [{A}, {B}]
#[test]
fn test_graph_nested_branch_mixed_direct_intermediate() {
    let make_inner = || {
        let mut inner = Nested::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let add1 = inner.add_node(Box::new(node::expr("(+ $x 1)").unwrap()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Chained intermediates. Each arm passes through a two-node chain.
// branches: [{A}, {B}]
#[test]
fn test_graph_nested_branch_chained_intermediates() {
    let make_inner = || {
        let mut inner = Nested::default();
        let inlet = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let a10 = inner.add_node(Box::new(node::expr("(+ $x 10)").unwrap()) as Box<_>);
        let a1 = inner.add_node(Box::new(node::expr("(+ $x 1)").unwrap()) as Box<_>);
        let b10 = inner.add_node(Box::new(node::expr("(+ $x 10)").unwrap()) as Box<_>);
        let b1 = inner.add_node(Box::new(node::expr("(+ $x 1)").unwrap()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Cascading reconvergence. Three sequential branches, but Select A and Select
// B each reconverge at a join. So only Select C affects the outlet. The 2^3 =
// 8 inner worlds collapse to two external branches, deduplicated by outlet
// set. branches: [{A}, {B}]
#[test]
fn test_graph_nested_cascading_reconvergence() {
    let make_inner = || {
        let mut inner = Nested::default();
        let in1 = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let in2 = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
        let in3 = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
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
        let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Inner reconvergence and an independent outer branch in the same graph.
// Select1 reconverges to A, which is always active. Select2 picks B or C.
// branches: [{A, B}, {A, C}]
#[test]
fn test_graph_nested_inner_reconvergence_outer_branching() {
    let make_inner = || {
        let mut inner = Nested::default();
        let in1 = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let in2 = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
        let s1 = inner.add_node(Box::new(node_select()) as Box<_>);
        let a10 = inner.add_node(Box::new(node::expr("(+ $x 10)").unwrap()) as Box<_>);
        let a20 = inner.add_node(Box::new(node::expr("(+ $x 20)").unwrap()) as Box<_>);
        let join = inner.add_node(Box::new(node::expr("(begin $x)").unwrap()) as Box<_>);
        let s2 = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// A static inlet used at branch depth 3. `value` feeds `depth3`, which sits
// on Select3's arm 0. `value` must stay in scope inside that arm even though
// it enters from outside the arm. Sequential, so 4 branches.
// branches: [{A}, {B}, {C}, {D}]
#[test]
fn test_graph_nested_static_inlet_at_depth_three() {
    let make_inner = || {
        let mut inner = Nested::default();
        let f1 = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let f2 = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
        let f3 = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
        let value = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
        let s1 = inner.add_node(Box::new(node_select()) as Box<_>);
        let pa = inner.add_node(Box::new(node::expr("(begin $l $r)").unwrap()) as Box<_>);
        let s2 = inner.add_node(Box::new(node_select()) as Box<_>);
        let pb = inner.add_node(Box::new(node::expr("(begin $l $r)").unwrap()) as Box<_>);
        let s3 = inner.add_node(Box::new(node_select()) as Box<_>);
        let d3 = inner.add_node(Box::new(node::expr("(begin $l $r)").unwrap()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let oc = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let od = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Three independent parallel branches give 2^3 = 8 external branches in a
// 3-component flow graph. branches: all 8 of {A|B} x {C|D} x {E|F}.
#[test]
fn test_graph_nested_multi_branch_three() {
    let make_inner = || {
        let mut inner = Nested::default();
        let i1 = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let i2 = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
        let i3 = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
        let s1 = inner.add_node(Box::new(node_select()) as Box<_>);
        let s2 = inner.add_node(Box::new(node_select()) as Box<_>);
        let s3 = inner.add_node(Box::new(node_select()) as Box<_>);
        let mut add =
            |n: i32| inner.add_node(Box::new(node::expr(format!("(+ $x {n})")).unwrap()) as Box<_>);
        let (a10, a20, a30, a40, a50, a60) = (add(10), add(20), add(30), add(40), add(50), add(60));
        let o: Vec<_> = (0..6)
            .map(|_| inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>))
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
    // (0,0,0) fires A=42+10, C=42+30, E=42+50. B, D and F are dead.
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

// Push-through-outlet branching tests.
//
// Here the `push_eval` lives inside the nested graph and propagates out
// through the graph's outlets through an interior branch. The bridged graph
// node acts as a branch node in the parent for that entrypoint. So the parent
// only evaluates downstream of the outlets the taken arm produced. Each
// test's inner graph is sketched above it. `[Sel]` is `node_select`.

// Divergent push-through. Each arm drives its own outlet and its own outer
// store.
//
//   INNER: [push]->[int(sel)]->[Sel]        OUTER: [inner]
//                            o0/  \o1               o0/  \o1
//                        [OutA]   [OutB]      [store_a] [store_b]
#[test]
fn test_graph_nested_push_through_divergent_branch() {
    let build = |sel: i32| {
        let mut inner = Nested::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Dead-arm push-through. Arm 1 leaves the select output unconnected, so it
// produces nothing and no outer store is written.
//
//   INNER: [push]->[int(sel)]->[Sel]        OUTER: [inner]
//                            o0|  \o1 (dead)         o0|
//                        [OutA]    x              [store]
#[test]
fn test_graph_nested_push_through_dead_arm() {
    let build = |sel: i32| {
        let mut inner = Nested::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        inner.add_edge(push, int, Edge::from((0, 0)));
        inner.add_edge(int, select, Edge::from((0, 0)));
        inner.add_edge(select, outlet, Edge::from((0, 0)));
        // Select output 1 stays unconnected as a dead arm.

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

// Multi-output-arm push-through. Arm 0 fires two outlets and arm 1 fires one.
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
        let mut inner = Nested::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner.add_node(Box::new(node_int(sel)) as Box<_>);
        let br = inner.add_node(Box::new(branch3()) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let outlet_c = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Two-level push-through. The push is inside the innermost graph. Its branch
// propagates out through two levels of outlets. The middle graph branches
// because the inner one does.
//
//   INNER2: [push]->[int(sel)]->[Sel]->{oa,ob}
//   INNER1: [inner2]->{ox,oy}
//   OUTER:  [inner1]->{store_x, store_y}
#[test]
fn test_graph_nested_push_through_two_levels() {
    let build = |sel: i32| {
        let mut inner2 = Nested::default();
        let push = inner2.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner2.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = inner2.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner2.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let ob = inner2.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        inner2.add_edge(push, int, Edge::from((0, 0)));
        inner2.add_edge(int, select, Edge::from((0, 0)));
        inner2.add_edge(select, oa, Edge::from((0, 0)));
        inner2.add_edge(select, ob, Edge::from((1, 0)));

        let ctx = node::MetaCtx::new(&no_lookup);
        let push_n = inner2[push].n_outputs(ctx) as u8;

        let mut inner1 = Nested::default();
        let inner2_node = inner1.add_node(Box::new(inner2) as Box<dyn DebugNode>);
        let ox = inner1.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let oy = inner1.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Reconvergent push-through. A divergent interior branch whose two arms feed
// distinct outlets that re-join at a single outer store. This is a join
// across the bridge boundary. The store always fires with the taken arm's
// value.
//
//   INNER: [push]->[int(sel)]->[Sel]->{oa(o0), ob(o1)}
//   OUTER: inner.o0 -\
//          inner.o1 --> [store]
#[test]
fn test_graph_nested_push_through_reconvergent_branch() {
    let build = |sel: i32| {
        let mut inner = Nested::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let oa = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let ob = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        inner.add_edge(push, int, Edge::from((0, 0)));
        inner.add_edge(int, select, Edge::from((0, 0)));
        inner.add_edge(select, oa, Edge::from((0, 0)));
        inner.add_edge(select, ob, Edge::from((1, 0)));

        let ctx = node::MetaCtx::new(&no_lookup);
        let push_n = inner[push].n_outputs(ctx) as u8;

        let mut g = petgraph::graph::DiGraph::new();
        let inner_node = g.add_node(Box::new(inner) as Box<dyn DebugNode>);
        let store = g.add_node(Box::new(node_number()) as Box<_>);
        // Both arms route to the same store, so they reconverge across the
        // bridge.
        g.add_edge(inner_node, store, Edge::from((0, 0)));
        g.add_edge(inner_node, store, Edge::from((1, 0)));

        let vm = compile_and_push_nested(&g, vec![inner_node.index(), push.index()], push_n);
        store_val(&vm, store)
    };

    assert_eq!(build(0), Some(42)); // arm 0 -> outlet A -> store
    assert_eq!(build(1), Some(99)); // arm 1 -> outlet B -> store
}

// A push-through branch alongside an always-active outlet. The push also
// drives a constant-fed outlet that every arm produces. Its store always
// fires while the branch arms route to their own stores.
//
//   INNER: [push]-+->[int(sel)]->[Sel]->{oa(o0), ob(o1)}
//                 +->[int(7)]---------->{oc(o2)}   (always produced)
//   OUTER: inner.{o0,o1,o2} -> {store_a, store_b, store_c}
#[test]
fn test_graph_nested_push_through_branch_with_constant_outlet() {
    let build = |sel: i32| {
        let mut inner = Nested::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let int = inner.add_node(Box::new(node_int(sel)) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let seven = inner.add_node(Box::new(node_int(7)) as Box<_>);
        let outlet_a = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let outlet_b = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
        let outlet_c = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Multi-root branch-reconvergence ordering tests.
//
// A single entrypoint with two flow roots. One root branches and its arms
// reconverge at a join that also consumes the other root's value. The join is
// the branch's post-dominator yet depends on a second root. The emitter must
// emit a producing component before a consuming one. It must also
// destructure a terminal block's last node so its outputs are available
// cross-component.

// Compile and run `g` from two push sources in one entrypoint. Returns the VM
// for state queries.
fn run_two_push<N: DebugNode + ?Sized>(
    g: &petgraph::graph::DiGraph<Box<N>, Edge>,
    a: (Vec<usize>, u8),
    b: (Vec<usize>, u8),
) -> Engine {
    let ep = entrypoint::from_sources([push_source(a.0, a.1), push_source(b.0, b.1)]);
    let module =
        gantz_core::compile::module(&no_lookup, g, &[ep.clone()], &Default::default()).unwrap();
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

// Two push roots. Root A branches and its arms reconverge at `add`, which also
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

// The same logical graph, but the predecessor's nodes are added first. So the
// topological ordering linearizes `int(20)` ahead of the branch within a
// single component. This guards the linearized form.
#[test]
fn test_multiroot_branch_join_external_pred_reversed() {
    let build = |sel: i32| {
        let mut g = petgraph::graph::DiGraph::new();
        // Root B first, with lower ids.
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

// The same branch-join shape one level down. Inside a nested graph, inlet_a
// branches and its arms reconverge at `add`, which also takes inlet_b. Here
// the inlets linearize into one component. This guards the nested codegen
// path.
//
//   INNER: [inlet_a]->[select] o0,o1 -> add.$l ;  [inlet_b] -> add.$r ;  add -> outlet
//   OUTER: [push]->[int sel]->inlet_a ;  [push]->[int 20]->inlet_b ;  inner -> store
#[test]
fn test_nested_branch_join_external_inlet() {
    let make_inner = || {
        let mut inner = Nested::default();
        let inlet_a = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
        let inlet_b = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
        let select = inner.add_node(Box::new(node_select()) as Box<_>);
        let add = inner.add_node(Box::new(node::expr("(+ $l $r)").unwrap()) as Box<_>);
        let outlet = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// An `Outlet` at the root level has no enclosing graph node. It must compile
// and be ignored. There is no parent to read its value, so it is a no-op
// while the rest of the graph still evaluates.
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
    let outlet = g.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    g.add_edge(push, int, Edge::from((0, 0)));
    g.add_edge(int, store, Edge::from((0, 0)));
    g.add_edge(store, outlet, Edge::from((0, 0)));

    // Compiles, runs, and the upstream `number` still receives the value even
    // though the root outlet leads nowhere.
    assert_eq!(store_val(&compile_and_push(&g, push), store), Some(42));
}

// A disconnected `Outlet` at the root level has no incoming edge. The flow
// never reaches it, so it emits nothing and the rest of the graph evaluates
// normally.
#[test]
fn test_graph_root_outlet_disconnected() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int = g.add_node(Box::new(node_int(42)) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    let _outlet = g.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    g.add_edge(push, int, Edge::from((0, 0)));
    g.add_edge(int, store, Edge::from((0, 0)));

    assert_eq!(store_val(&compile_and_push(&g, push), store), Some(42));
}

// pd+ optional-input cold/hot inlet tests.
//
// These emulate Pure Data's stateful `+`. The left hot inlet always outputs
// the sum. The right cold inlet only updates internal state. The node is a
// nested `Graph` whose interior is a single `Branch` that reads two optional
// inputs, `$?l` and `$?r`. The cold/hot behaviour relies on the inner branch
// seeing `(None)` for the inlet that did not fire. That requires the
// active-input-set to propagate into the nested graph's interior.

// The pd+ Branch. Cold `$?r` sets state. Hot `$?l` outputs `left + state`.
// Branch 0 activates the single output when hot fired. Branch 1 activates
// nothing for cold-only.
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

// A pd+ nested graph. Input 0 is the left hot inlet. Input 1 is the right
// cold inlet. Output 0 is the sum. Returns the graph and the inner branch
// node id for state queries through the path `[pd_node, branch]`.
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
fn pd_plus() -> (Nested, usize) {
    let mut g = Nested::default();
    let inlet_l = g.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
    let inlet_r = g.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
    let branch = g.add_node(Box::new(pd_plus_branch()) as Box<_>);
    let outlet = g.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    g.add_edge(inlet_r, branch, Edge::from((0, 0))); // In R -> $?r (branch input 0)
    g.add_edge(inlet_l, branch, Edge::from((0, 1))); // In L -> $?l (branch input 1)
    g.add_edge(branch, outlet, Edge::from((0, 0)));
    (g, branch.index())
}

// Compile `g` and register fns, returning a VM ready to be pushed.
fn compile_only<N: DebugNode + ?Sized>(g: &petgraph::graph::DiGraph<Box<N>, Edge>) -> Engine {
    let eps = push_pull_entrypoints(&no_lookup, g);
    let module = gantz_core::compile::module(&no_lookup, g, &eps, &Default::default()).unwrap();
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

// A root graph. A left push feeds a left value and a right push feeds a right
// value into a pd+ node. Its output feeds a `store`. Returns (graph,
// left_push, right_push, pd, branch_id, store).
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

// Pushing only the cold right inlet must set state and produce no output. It
// must not raise `+ expects a number, found '()`.
#[test]
fn test_nested_pd_plus_cold_only() {
    let (g, _left_push, right_push, pd, branch_ix, store) = pd_plus_root(10, 5);
    let mut vm = compile_only(&g);
    push_from(&mut vm, &g, right_push);
    assert_eq!(branch_state(&vm, pd, branch_ix), Some(5), "cold sets state");
    assert_eq!(store_val(&vm, store), None, "cold produces no output");
}

// Cold right then hot left. The cold push seeds state and the hot push
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

// Firing both inlets in one push. The cold value updates state first, then
// the hot arm outputs `left + state`.
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

// A sequence of pushes across multiple entrypoint calls. Two cold updates,
// then a hot output. This exercises state persistence.
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

// The same Branch at top level and nested must behave identically.
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
    // Nested. Cold 5 then hot 10.
    let (gn, ln, rn, _pd, _bix, sn) = pd_plus_root(10, 5);
    let mut vmn = compile_only(&gn);
    push_from(&mut vmn, &gn, rn);
    let cold_n = store_val(&vmn, sn);
    push_from(&mut vmn, &gn, ln);
    let hot_n = store_val(&vmn, sn);

    // Top level, same sequence.
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

// The reduced inner-branch variants must be defined in the module, not just
// the all-connected i11. A cold push gives i10 and a hot push gives i01. This
// guards the conf post-pass and call/def agreement.
#[test]
fn test_nested_pd_plus_emits_reduced_variant() {
    let (g, _l, _r, pd, branch_ix, _store) = pd_plus_root(10, 5);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();
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

// pd+ wrapped in a second nested `Graph`. Returns (outer, pd_id_in_outer,
// branch_id_in_pd). Outer input 0 feeds the hot pd left inlet. Input 1 feeds
// the cold pd right inlet.
fn pd_plus_wrapped() -> (Nested, usize, usize) {
    let (pd_inner, branch_ix) = pd_plus();
    let mut outer = Nested::default();
    let inlet_l = outer.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
    let inlet_r = outer.add_node(Box::new(node::graph::Inlet::default()) as Box<_>);
    let pd = outer.add_node(Box::new(pd_inner) as Box<_>);
    let outlet = outer.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    outer.add_edge(inlet_l, pd, Edge::from((0, 0))); // outer in 0 -> pd in 0 (hot)
    outer.add_edge(inlet_r, pd, Edge::from((0, 1))); // outer in 1 -> pd in 1 (cold)
    outer.add_edge(pd, outlet, Edge::from((0, 0)));
    (outer, pd.index(), branch_ix)
}

// Two-level nesting. A cold-only push from the outside must propagate the
// reduced active-set through both graph layers. The grandchild branch then
// sees `(None)` for the hot inlet. There is no error, state is set and there
// is no output.
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

    // Cold only. State is set deep inside and there is no output.
    push_from(&mut vm, &g, right_push);
    assert_eq!(
        node::state::extract::<i32>(&vm, &branch_path)
            .ok()
            .flatten(),
        Some(5),
    );
    assert_eq!(store_val(&vm, store), None);

    // Hot outputs 10 + 5 = 15.
    push_from(&mut vm, &g, left_push);
    assert_eq!(store_val(&vm, store), Some(15));
}

// A wrapper `Graph` that exposes only pd+'s hot inlet. The cold inlet stays
// permanently unconnected. Even when the wrapper is invoked all-active, its
// interior invokes pd+ with a statically reduced active-set with only the hot
// inlet wired. That inner branch variant must still be defined. This guards
// the conf post-pass recursing through an all-active parent into a reduced
// child.
#[test]
fn test_nested_reduced_child_under_active_parent() {
    let (pd_inner, _branch_ix) = pd_plus();
    let mut outer = Nested::default();
    let inlet = outer.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
    let pd = outer.add_node(Box::new(pd_inner) as Box<_>);
    let outlet = outer.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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
    // The cold inlet is never wired, so `$?r` is `(None)`, state stays 0 and
    // the output is 10 + 0. Reaching this without a free-identifier error
    // proves the reduced inner branch variant was defined and called.
    assert_eq!(store_val(&vm, store), Some(10));
}

// Push-through reaching a nested-optional child. An interior push fires only
// the hot inlet of a nested pd+. Its output propagates out through the
// wrapper's outlet. This exercises a reduced inner-branch variant reached
// through push-through, not through the wrapper's own inlets.
#[test]
fn test_push_through_into_nested_optional_hot() {
    let (c_inner, _branch_ix) = pd_plus();
    let mut l = Nested::default();
    let p = l.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let val = l.add_node(Box::new(node_int(10)) as Box<_>);
    let c = l.add_node(Box::new(c_inner) as Box<_>);
    let outlet = l.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
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

// Push-through into a side-effect-only nested-optional child. An interior
// push fires only the cold inlet of a nested pd+ that produces no output and
// feeds no outlet. The all-connected interior flow never reaches C. So C's
// reduced branch variant is discoverable only from the interior push's flow.
#[test]
fn test_push_through_into_nested_optional_sideeffect() {
    let (c_inner, branch_ix) = pd_plus();
    let mut l = Nested::default();
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
