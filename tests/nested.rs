use gantz::node::{self, SerdeNode, WithPushEval};
use gantz::Edge;

fn node_push() -> node::Push<node::Expr> {
    node::expr("()").unwrap().with_push_eval_name("push")
}

fn node_int(i: i32) -> node::Expr {
    node::expr(&format!("{{ #push; {} }}", i)).unwrap()
}

fn node_mul() -> node::Expr {
    node::expr("#l * #r").unwrap()
}

fn node_assert_eq() -> node::Expr {
    node::expr("assert_eq!(#l, #r)").unwrap()
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
    // Create a temp project.
    let mut project = gantz::TempProject::open_with_name("test_graph_nested_stateless").unwrap();

    // Instantiate the nodes.
    let push = node_push();
    let six = node_int(6);
    let seven = node_int(7);
    let forty_two = node_int(42);
    let mul = node_mul();
    let assert_eq = node_assert_eq();
    let inlet = gantz::graph::Inlet::parse("i32").unwrap();
    let outlet = gantz::graph::Outlet::parse("i32").unwrap();

    // Add the nodes to the project.
    let push = project.add_core_node(Box::new(push) as Box<dyn SerdeNode>);
    let six = project.add_core_node(Box::new(six) as Box<_>);
    let seven = project.add_core_node(Box::new(seven) as Box<_>);
    let forty_two = project.add_core_node(Box::new(forty_two) as Box<_>);
    let mul = project.add_core_node(Box::new(mul) as Box<_>);
    let assert_eq = project.add_core_node(Box::new(assert_eq) as Box<_>);
    let inlet = project.add_core_node(Box::new(inlet) as _);
    let outlet = project.add_core_node(Box::new(outlet) as _);
    // We'll use the project root graph as GRAPH B, but we still need to add a node for GRAPH A.
    let graph_a = project
        .add_graph_node(Default::default(), "graph_a")
        .unwrap();

    // Compose the inner GRAPH A first.
    project
        .update_graph(&graph_a, |g| {
            let inlet_a = g.add_inlet(inlet);
            let inlet_b = g.add_inlet(inlet);
            let mul = g.add_node(mul);
            let outlet = g.add_outlet(outlet);
            g.add_edge(inlet_a, mul, Edge::from((0, 0)));
            g.add_edge(inlet_b, mul, Edge::from((0, 1)));
            g.add_edge(mul, outlet, Edge::from((0, 0)));
        })
        .unwrap();

    // Now compose the project root graph.
    let root = project.root_node_id();
    project
        .update_graph(&root, |g| {
            let push = g.add_node(push);
            let six = g.add_node(six);
            let seven = g.add_node(seven);
            let graph_a = g.add_node(graph_a);
            let forty_two = g.add_node(forty_two);
            let assert_eq = g.add_node(assert_eq);
            g.add_edge(push, six, Edge::from((0, 0)));
            g.add_edge(push, seven, Edge::from((0, 0)));
            g.add_edge(push, forty_two, Edge::from((0, 0)));
            g.add_edge(six, graph_a, Edge::from((0, 0)));
            g.add_edge(seven, graph_a, Edge::from((0, 1)));
            g.add_edge(graph_a, assert_eq, Edge::from((0, 0)));
            g.add_edge(forty_two, assert_eq, Edge::from((0, 1)));
        })
        .unwrap();

    // Retrieve the path to the compiled library.
    let root_dylib_path = project
        .graph_node_dylib(&root)
        .unwrap()
        .expect("no dylib or node");
    let lib = libloading::Library::new(&root_dylib_path).expect("failed to load root library");
    let symbol_name = "push".as_bytes();
    unsafe {
        let push_eval_fn: libloading::Symbol<fn(&mut [&mut dyn std::any::Any])> =
            lib.get(symbol_name).expect("failed to load symbol");
        // Execute the gantz graph.
        push_eval_fn(&mut []);
    }
}
