// Tests for the graph module.

use gantz_core::codegen::{pull_eval_fn_name, push_eval_fn_name};
use gantz_core::node::{self, Node, WithPullEval, WithPushEval};
use gantz_core::{Edge, ROOT_STATE};
use std::fmt::Debug;
use steel::SteelVal;
use steel::steel_vm::engine::Engine;

fn node_push() -> node::Push<node::Expr> {
    node::expr("'()").unwrap().with_push_eval()
}

fn node_int(i: i32) -> node::Expr {
    node::expr(format!("(begin $push {})", i)).unwrap()
}

fn node_add() -> node::Expr {
    node::expr("(+ $l $r)").unwrap()
}

fn node_assert_eq() -> node::Expr {
    node::expr("(assert! (equal? $l $r))").unwrap()
}

// Helper trait for debugging the graph.
trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A simple test graph that adds two "one"s and checks that it equals "two".
//
//    --------
//    | push | // push_eval
//    -+------
//     |
//     |---------
//     |        |
//    -+-----   |
//    | one |   |
//    -+-----   |
//     |\       |
//     | \      |
//     |  \     |
//    -+---+-  -+-----
//    | add |  | two |
//    -+-----  -+-----
//     |        |
//     |       --
//     |       |
//    -+-------+-
//    |assert_eq|
//    -----------
#[test]
fn test_graph_push_eval() {
    let mut g = petgraph::graph::DiGraph::new();

    // Instantiate the nodes.
    let push = node_push();
    let one = node_int(1);
    let add = node_add();
    let two = node_int(2);
    let assert_eq = node_assert_eq();

    // Add the nodes to the project.
    let push = g.add_node(Box::new(push) as Box<dyn DebugNode>);
    let one = g.add_node(Box::new(one) as Box<_>);
    let add = g.add_node(Box::new(add) as Box<_>);
    let two = g.add_node(Box::new(two) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);
    g.add_edge(push, one, Edge::from((0, 0)));
    g.add_edge(push, two, Edge::from((0, 0)));
    g.add_edge(one, add, Edge::from((0, 0)));
    g.add_edge(one, add, Edge::from((0, 1)));
    g.add_edge(add, assert_eq, Edge::from((0, 0)));
    g.add_edge(two, assert_eq, Edge::from((0, 1)));

    // Generate the module, which should have just one top-level expr for `push`.
    let module = gantz_core::codegen::module(&g, &[], &[], &[]);
    // Function per node alongside the single push eval function.
    assert_eq!(module.len(), g.node_count() + 1);

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    node::state::register_graph(&g, &mut vm);

    // Register the functions, then call push_eval.
    for f in module {
        vm.run(format!("{f}")).unwrap();
    }
    vm.call_function_by_name_with_args(&push_eval_fn_name(push.index()), vec![])
        .unwrap();
}

// A simple test graph that adds two "one"s and checks that it equals "two".
//
//    -+-----
//    | one |
//    -+-----
//     |\
//     | \
//     |  \
//    -+---+-  -+-----
//    | add |  | two |
//    -+-----  -+-----
//     |        |
//     |       --
//     |       |
//    -+-------+-
//    |assert_eq| // pull_eval
//    -----------
#[test]
fn test_graph_pull_eval() {
    let mut g = petgraph::graph::DiGraph::new();

    // Instantiate the nodes.
    let one = node_int(1);
    let add = node_add();
    let two = node_int(2);
    let assert_eq = node_assert_eq().with_pull_eval();

    // Add the nodes to the project.
    let one = g.add_node(Box::new(one) as Box<dyn DebugNode>);
    let add = g.add_node(Box::new(add) as Box<_>);
    let two = g.add_node(Box::new(two) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);
    g.add_edge(one, add, Edge::from((0, 0)));
    g.add_edge(one, add, Edge::from((0, 1)));
    g.add_edge(add, assert_eq, Edge::from((0, 0)));
    g.add_edge(two, assert_eq, Edge::from((0, 1)));

    // Generate the steel module.
    let module = gantz_core::codegen::module(&g, &[], &[], &[]);

    // Prepare the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    node::state::register_graph(&g, &mut vm);

    // Prepare the eval fn.
    for expr in module {
        vm.run(expr.to_pretty(100)).unwrap();
    }

    // Call the eval fn.
    vm.call_function_by_name_with_args(&pull_eval_fn_name(assert_eq.index()), vec![])
        .unwrap();
}

// A simple test graph that is expected to `panic!`.
//
//    -+-----
//    | one |
//    -+-----
//     |\----
//     | \   \
//     |  \   \
//    -+---+-  |
//    | add |  |
//    -+-----  |
//     |       |
//     |       |
//     |       |
//    -+-------+-
//    |assert_eq| // pull_eval & panic!
//    -----------
#[test]
#[should_panic]
fn test_graph_eval_should_panic() {
    let mut g = petgraph::graph::DiGraph::new();

    // Instantiate the nodes.
    let one = node_int(1);
    let add = node_add();
    let assert_eq = node_assert_eq().with_pull_eval();

    // Add the nodes to the project.
    let one = g.add_node(Box::new(one) as Box<dyn DebugNode>);
    let add = g.add_node(Box::new(add) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);
    g.add_edge(one, add, Edge::from((0, 0)));
    g.add_edge(one, add, Edge::from((0, 1)));
    g.add_edge(add, assert_eq, Edge::from((0, 0)));
    g.add_edge(one, assert_eq, Edge::from((0, 1)));

    // Generate the steel module.
    let module = gantz_core::codegen::module(&g, &[], &[], &[]);

    // Prepare the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    node::state::register_graph(&g, &mut vm);

    // Run the module.
    for expr in module {
        vm.run(expr.to_pretty(100)).unwrap();
    }
    vm.call_function_by_name_with_args(&pull_eval_fn_name(assert_eq.index()), vec![])
        .unwrap();
}
