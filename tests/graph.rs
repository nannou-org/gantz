// Tests for the graph module.

use gantz::Edge;
use gantz::node::{self, SerdeNode, WithPushEval};

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
    let push = project.add_core_node(Box::new(push) as Box<SerdeNode>);
    let one = project.add_core_node(Box::new(one) as Box<_>);
    let add = project.add_core_node(Box::new(add) as Box<_>);
    let two = project.add_core_node(Box::new(two) as Box<_>);
    let assert_eq = project.add_core_node(Box::new(assert_eq) as Box<_>);

    // Compose the graph.
    let root = project.root_node_id();
    project.update_graph(&root, |g| {
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
    }).unwrap();

    // Retrieve the path to the compiled library.
    let dylib_path = project.graph_node_dylib(&root).unwrap().expect("no dylib or node");
    let lib = libloading::Library::new(&dylib_path).expect("failed to load library");
    let symbol_name = "push".as_bytes();
    unsafe {
        let push_eval_fn: libloading::Symbol<fn()> =
            lib.get(symbol_name).expect("failed to load symbol");
        // Execute the gantz graph.
        push_eval_fn();
    }
}
