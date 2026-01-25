// Tests for the Fn and Apply nodes - first-class functions in gantz.

use gantz_core::compile::pull_eval_fn_name;
use gantz_core::node::{self, Apply, Fn, Node, Ref, WithPullEval, graph};
use gantz_core::{Edge, ROOT_STATE};
use std::collections::HashMap;
use std::fmt::Debug;
use steel::SteelVal;
use steel::steel_vm::engine::Engine;

// A simple test environment that maps addresses to nodes.
#[derive(Debug, Default)]
struct TestEnv {
    nodes: HashMap<gantz_ca::ContentAddr, Box<dyn DebugNode>>,
}

impl node::ref_::NodeRegistry for TestEnv {
    type Node = Box<dyn DebugNode>;
    fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&Self::Node> {
        self.nodes.get(&ca).map(|boxed| &*boxed)
    }
}

fn node_bang() -> node::expr::Expr {
    node::expr("'bang").unwrap()
}

fn node_int(i: i32) -> node::expr::Expr {
    node::expr(format!("{}", i)).unwrap()
}

fn node_list_single() -> node::expr::Expr {
    node::expr("(list $x)").unwrap()
}

fn node_assert_eq() -> node::expr::Expr {
    node::expr("(assert! (equal? $l $r))").unwrap()
}

// Helper trait for debugging
trait DebugNode: Debug + Node<TestEnv> + gantz_ca::CaHash {}
impl<T> DebugNode for T where T: Debug + Node<TestEnv> + gantz_ca::CaHash {}

// Test that Fn can wrap the identity function and Apply can call it
//
//    --------
//    | bang |
//    -+------
//     |
//     |
//    -+------
//    |  fn  |  ------
//    | (id) |  | 42 |
//    -+------  -+----
//     |         |
//     |     -----
//     |     |
//    -+-----+-
//    | apply |
//    -+-------
//     |
//     |
//    ----------
//    | result |
//    ----------
#[test]
fn test_fn_apply_identity() {
    let mut g = petgraph::graph::DiGraph::new();

    // Setup the test environment.
    let mut env = TestEnv::default();

    // Just add the identity node.
    let id = gantz_core::node::Identity;
    let id_ca = gantz_ca::content_addr(&id);
    env.nodes.insert(id_ca, Box::new(id) as Box<dyn DebugNode>);

    // Create nodes
    let bang = node_bang();
    let fn_node = Fn::new(Ref::new(id_ca));
    let apply_node = Apply;
    let value = node_int(42);
    let list = node_list_single(); // Wrap value in list for apply
    let expected = node_int(42);
    let assert_eq = node_assert_eq().with_pull_eval();

    // Add nodes to graph
    let bang = g.add_node(Box::new(bang) as Box<dyn DebugNode>);
    let fn_node = g.add_node(Box::new(fn_node) as Box<_>);
    let apply_node = g.add_node(Box::new(apply_node) as Box<_>);
    let value = g.add_node(Box::new(value) as Box<_>);
    let list = g.add_node(Box::new(list) as Box<_>);
    let expected = g.add_node(Box::new(expected) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);

    // Bang triggers fn to emit lambda.
    g.add_edge(bang, fn_node, Edge::from((0, 0)));
    // Fn output (lambda) goes to apply's function input.
    g.add_edge(fn_node, apply_node, Edge::from((0, 0)));
    // Value goes to list to wrap it.
    g.add_edge(value, list, Edge::from((0, 0)));
    // List goes to apply's argument input.
    g.add_edge(list, apply_node, Edge::from((0, 1)));
    // Apply output goes to assert_eq.
    g.add_edge(apply_node, assert_eq, Edge::from((0, 0)));
    // Expected value goes to assert_eq.
    g.add_edge(expected, assert_eq, Edge::from((0, 1)));

    // Generate the module.
    let module = gantz_core::compile::module(&env, &g).unwrap();

    // Create and setup VM.
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&env, &g, &[], &mut vm);

    // Register all functions
    for expr in module {
        vm.run(expr.to_pretty(80)).unwrap();
    }

    // Execute pull evaluation from assert_eq
    vm.call_function_by_name_with_args(&pull_eval_fn_name(&[assert_eq.index()]), vec![])
        .unwrap();
}

// Test that Fn can wrap a graph node (not just a primitive) and Apply can call it.
// This tests that the NodeFns visitor correctly recurses into the graph via Fn's
// visit() delegation to generate the nested node functions.
//
// The "double" graph: inlet -> add (both inputs from inlet) -> outlet
// Result: double(x) = x + x
//
//    --------
//    | bang |
//    -+------
//     |
//     |
//    -+--------
//    |   fn   |  ------
//    | (dbl)  |  | 21 |
//    -+--------  -+----
//     |           |
//     |       -----
//     |       |
//    -+-------+-
//    |  apply  |
//    -+---------
//     |
//     |
//    ----------
//    | result |  (should be 42)
//    ----------
#[test]
fn test_fn_apply_graph() {
    // First, create the "double" graph: inlet -> add -> outlet
    let mut double_graph = graph::Graph::<Box<dyn DebugNode>>::default();

    let inlet = double_graph.add_node(Box::new(graph::Inlet) as Box<dyn DebugNode>);
    let add = double_graph.add_node(Box::new(node::expr("(+ $l $r)").unwrap()) as Box<_>);
    let outlet = double_graph.add_node(Box::new(graph::Outlet) as Box<_>);

    // Connect inlet to both inputs of add.
    double_graph.add_edge(inlet, add, Edge::from((0, 0)));
    double_graph.add_edge(inlet, add, Edge::from((0, 1)));
    // Connect add output to outlet.
    double_graph.add_edge(add, outlet, Edge::from((0, 0)));

    let double_node = graph::GraphNode {
        graph: double_graph,
    };

    // Setup the test environment with the double graph.
    let mut env = TestEnv::default();
    let double_ca = gantz_ca::content_addr(&double_node);
    env.nodes
        .insert(double_ca, Box::new(double_node) as Box<dyn DebugNode>);

    // Now create the main graph that uses Fn<Ref> to wrap the double graph.
    let mut g = petgraph::graph::DiGraph::new();

    let bang = node_bang();
    let fn_node = Fn::new(Ref::new(double_ca));
    let apply_node = Apply;
    let value = node_int(21);
    let list = node_list_single();
    let expected = node_int(42);
    let assert_eq = node_assert_eq().with_pull_eval();

    let bang = g.add_node(Box::new(bang) as Box<dyn DebugNode>);
    let fn_node = g.add_node(Box::new(fn_node) as Box<_>);
    let apply_node = g.add_node(Box::new(apply_node) as Box<_>);
    let value = g.add_node(Box::new(value) as Box<_>);
    let list = g.add_node(Box::new(list) as Box<_>);
    let expected = g.add_node(Box::new(expected) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);

    // Bang triggers fn to emit lambda.
    g.add_edge(bang, fn_node, Edge::from((0, 0)));
    // Fn output (lambda) goes to apply's function input.
    g.add_edge(fn_node, apply_node, Edge::from((0, 0)));
    // Value goes to list to wrap it.
    g.add_edge(value, list, Edge::from((0, 0)));
    // List goes to apply's argument input.
    g.add_edge(list, apply_node, Edge::from((0, 1)));
    // Apply output goes to assert_eq.
    g.add_edge(apply_node, assert_eq, Edge::from((0, 0)));
    // Expected value goes to assert_eq.
    g.add_edge(expected, assert_eq, Edge::from((0, 1)));

    // Generate the module.
    let module = gantz_core::compile::module(&env, &g).unwrap();

    // Create and setup VM.
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&env, &g, &[], &mut vm);

    // Register all functions.
    for expr in module {
        vm.run(expr.to_pretty(80)).unwrap();
    }

    // Execute pull evaluation from assert_eq.
    vm.call_function_by_name_with_args(&pull_eval_fn_name(&[assert_eq.index()]), vec![])
        .unwrap();
}
