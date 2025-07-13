//! Tests related to the nesting of graphs.

use gantz_core::{
    Edge, ROOT_STATE,
    codegen::push_eval_fn_name,
    graph::{self, GraphNode},
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

// Helper trait for debugging the graph.
trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

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
    let inlet_a = ga.add_inlet(Box::new(graph::Inlet) as Box<dyn DebugNode>);
    let inlet_b = ga.add_inlet(Box::new(graph::Inlet) as Box<_>);
    let mul = ga.add_node(Box::new(node_mul()) as Box<_>);
    let outlet = ga.add_outlet(Box::new(graph::Outlet) as Box<_>);
    ga.add_edge(inlet_a, mul, Edge::from((0, 0)));
    ga.add_edge(inlet_b, mul, Edge::from((0, 1)));
    ga.add_edge(mul, outlet, Edge::from((0, 0)));

    // Graph B.
    let mut gb = gantz_core::Graph::default();
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
    let module = gantz_core::codegen::module(&gb);

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state vars.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    node::state::register_graph(&gb, &mut vm);

    // Register the fns.
    for f in module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Register the `push` eval function, then call it.
    vm.call_function_by_name_with_args(&push_eval_fn_name(&[push.index()]), vec![])
        .ok();
}
