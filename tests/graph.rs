// Tests for the graph module.

use gantz::node::{self, SerdeNode, WithPushEval};
use gantz::Edge;
use serde::{Deserialize, Serialize};

fn node_push() -> node::Push<node::Expr> {
    node::expr("()").unwrap().with_push_eval_name("push")
}

fn node_int(i: i32) -> node::Expr {
    node::expr(&format!("{{ #push; {} }}", i)).unwrap()
}

fn node_add() -> node::Expr {
    node::expr("#l + #r").unwrap()
}

fn node_assert_eq() -> node::Expr {
    node::expr("assert_eq!(#l, #r)").unwrap()
}

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
fn test_graph1() {
    // Create a temp project.
    let mut project = gantz::TempProject::open_with_name("test_graph1").unwrap();

    // Instantiate the nodes.
    let push = node_push();
    let one = node_int(1);
    let add = node_add();
    let two = node_int(2);
    let assert_eq = node_assert_eq();

    // Add the nodes to the project.
    let push = project.add_core_node(Box::new(push) as Box<dyn SerdeNode>);
    let one = project.add_core_node(Box::new(one) as Box<_>);
    let add = project.add_core_node(Box::new(add) as Box<_>);
    let two = project.add_core_node(Box::new(two) as Box<_>);
    let assert_eq = project.add_core_node(Box::new(assert_eq) as Box<_>);

    // Compose the graph.
    let root = project.root_node_id();
    project
        .update_graph(&root, |g| {
            let push = g.add_node(push);
            let one = g.add_node(one);
            let add = g.add_node(add);
            let two = g.add_node(two);
            let assert_eq = g.add_node(assert_eq);
            g.add_edge(push, one, Edge::from((0, 0)));
            g.add_edge(push, two, Edge::from((0, 0)));
            g.add_edge(one, add, Edge::from((0, 0)));
            g.add_edge(one, add, Edge::from((0, 1)));
            g.add_edge(add, assert_eq, Edge::from((0, 0)));
            g.add_edge(two, assert_eq, Edge::from((0, 1)));
        })
        .unwrap();

    // Retrieve the path to the compiled library.
    let dylib_path = project
        .graph_node_dylib(&root)
        .unwrap()
        .expect("no dylib or node");
    let lib = libloading::Library::new(&dylib_path).expect("failed to load library");
    let symbol_name = "push".as_bytes();
    unsafe {
        let push_eval_fn: libloading::Symbol<fn()> =
            lib.get(symbol_name).expect("failed to load symbol");
        // Execute the gantz graph.
        push_eval_fn();
    }
}

// Create a Node for testing the `Fn` evaluator variant.
#[derive(Deserialize, Serialize)]
struct Mul;

impl gantz::Node for Mul {
    fn evaluator(&self) -> gantz::node::Evaluator {
        let fn_item = syn::parse_quote! {
            fn mul<T>(a: T, b: T) -> T
            where
                T: std::ops::Mul<T, Output = T>,
            {
                a * b
            }
        };
        gantz::node::Evaluator::Fn { fn_item }
    }
}

#[typetag::serde]
impl gantz::node::SerdeNode for Mul {
    fn node(&self) -> &dyn gantz::Node {
        self
    }
}

// A simple test graph that multiplies two "two"s and checks that it equals "two".
//
//    --------
//    | push | // push_eval
//    -+------
//     |
//     |---------
//     |        |
//    -+-----   |
//    | two |   |
//    -+-----   |
//     |\       |
//     | \      |
//     |  \     |
//    -+---+-  -+------
//    | mul |  | four |
//    -+-----  -+------
//     |        |
//     |       --
//     |       |
//    -+-------+-
//    |assert_eq|
//    -----------
#[test]
fn test_graph2_evaluator_fn() {
    // Create a temp project.
    let mut project = gantz::TempProject::open_with_name("test_graph2_evaluator_fn").unwrap();

    // Instantiate the nodes.
    let push = node_push();
    let two = node_int(2);
    let mul = Mul;
    let four = node_int(4);
    let assert_eq = node_assert_eq();

    // Check some properties about `Mul`, our fancy Fn node.
    let mul_eval = gantz::Node::evaluator(&mul);
    assert_eq!(mul_eval.n_inputs(), 2);
    assert_eq!(mul_eval.n_outputs(), 1);

    // Add the nodes to the project.
    let push = project.add_core_node(Box::new(push) as Box<dyn SerdeNode>);
    let two = project.add_core_node(Box::new(two) as Box<_>);
    let mul = project.add_core_node(Box::new(mul) as Box<_>);
    let four = project.add_core_node(Box::new(four) as Box<_>);
    let assert_eq = project.add_core_node(Box::new(assert_eq) as Box<_>);

    // Compose the graph.
    let root = project.root_node_id();
    project
        .update_graph(&root, |g| {
            let push = g.add_node(push);
            let two = g.add_node(two);
            let mul = g.add_node(mul);
            let four = g.add_node(four);
            let assert_eq = g.add_node(assert_eq);
            g.add_edge(push, two, Edge::from((0, 0)));
            g.add_edge(push, four, Edge::from((0, 0)));
            g.add_edge(two, mul, Edge::from((0, 0)));
            g.add_edge(two, mul, Edge::from((0, 1)));
            g.add_edge(mul, assert_eq, Edge::from((0, 0)));
            g.add_edge(four, assert_eq, Edge::from((0, 1)));
        })
        .unwrap();

    // Retrieve the path to the compiled library.
    let dylib_path = project
        .graph_node_dylib(&root)
        .unwrap()
        .expect("no dylib or node");
    let lib = libloading::Library::new(&dylib_path).expect("failed to load library");
    let symbol_name = "push".as_bytes();
    unsafe {
        let push_eval_fn: libloading::Symbol<fn()> =
            lib.get(symbol_name).expect("failed to load symbol");
        // Execute the gantz graph.
        push_eval_fn();
    }
}
