// Testing a simple counter node, the first stateful node type to be tested.

use gantz::node::{self, SerdeNode, WithPushEval, WithStateType};
use gantz::Edge;

fn node_push(push_eval_name: &str) -> node::Push<node::Expr> {
    node::expr("()")
        .unwrap()
        .with_push_eval_name(push_eval_name)
}

// A simple counter node.
//
// Increases its `u32` state by `1` each time it receives an input of any type.
fn node_counter() -> node::State<node::Expr> {
    node::expr(r#"{ #push; let count = *state; *state += 1; count }"#)
        .unwrap()
        .with_state_ty("u32")
        .unwrap()
}

// A simple as possible test graph for testing state.
//
//    --------
//    | push | // push_eval
//    -+------
//     |
//    -+---------
//    | counter |
//    -+---------
//
// The push evaluation enabled `push` node is called three times once loaded.
#[test]
fn test_graph_with_counter() {
    // Create a temp project.
    let mut project = gantz::TempProject::open_with_name("test_graph_with_counter").unwrap();

    // Instantiate the nodes.
    let symbol_name = "push";
    let push = node_push(symbol_name);
    let counter = node_counter();

    // Add the nodes to the project.
    let push = project.add_core_node(Box::new(push) as Box<dyn SerdeNode>);
    let counter = project.add_core_node(Box::new(counter) as Box<_>);

    // Compose the graph.
    let root = project.root_node_id();
    project
        .update_graph(&root, |g| {
            let push = g.add_node(push);
            let counter = g.add_node(counter);
            g.add_edge(push, counter, Edge::from((0, 0)));
        })
        .unwrap();

    // Initialise the counter state.
    let mut count = 0u32;

    // Retrieve the path to the compiled library.
    let dylib_path = project
        .graph_node_dylib(&root)
        .unwrap()
        .expect("no dylib or node");

    // Load the library.
    let lib = libloading::Library::new(&dylib_path).expect("failed to load library");
    unsafe {
        let push_eval_fn: libloading::Symbol<fn(&mut [&mut dyn std::any::Any])> = lib
            .get(symbol_name.as_bytes())
            .expect("failed to load symbol");
        // Prepare the `node_states` and execute the graph.
        let mut node_states = [&mut count as &mut dyn std::any::Any];
        push_eval_fn(&mut node_states[..]);
        push_eval_fn(&mut node_states[..]);
        push_eval_fn(&mut node_states[..]);
    }

    // Check the counter was incremented 3 times.
    assert_eq!(count, 3);
}

// A slightly more complex test of state.
//
//    --------    --------    --------
//    | push |    | push |    | push |
//    -+------    -+------    -+------
//     |           |           |
//    -+---------  |           |
//    | counter |  |           |
//    -+---------  |           |
//     |           |           |
//     -------------           |
//                 |           |
//                -+---------  |
//                | counter |  |
//                -+---------  |
//                 |           |
//                 -------------
//                             |
//                            -+---------
//                            | counter |
//                            -+---------
//
// Calls each of the `push` evaluation functions once from left to right.
#[test]
fn test_graph_with_counters() {
    // Create a temp project.
    let mut project = gantz::TempProject::open_with_name("test_graph_with_counters").unwrap();

    // Instantiate the nodes.
    let push_a_name = "push_a";
    let push_b_name = "push_b";
    let push_c_name = "push_c";
    let push_a = node_push(push_a_name);
    let push_b = node_push(push_b_name);
    let push_c = node_push(push_c_name);
    let counter = node_counter();

    // Add the nodes to the project.
    let push_a = project.add_core_node(Box::new(push_a) as Box<dyn SerdeNode>);
    let push_b = project.add_core_node(Box::new(push_b) as Box<dyn SerdeNode>);
    let push_c = project.add_core_node(Box::new(push_c) as Box<dyn SerdeNode>);
    let counter = project.add_core_node(Box::new(counter) as Box<_>);

    // Compose the graph.
    let root = project.root_node_id();
    let mut push_ids = vec![];
    let mut counter_ids = vec![];
    project
        .update_graph(&root, |g| {
            let p_a = g.add_node(push_a);
            let p_b = g.add_node(push_b);
            let p_c = g.add_node(push_c);
            let c_a = g.add_node(counter);
            let c_b = g.add_node(counter);
            let c_c = g.add_node(counter);
            g.add_edge(p_a, c_a, Edge::from((0, 0)));
            g.add_edge(c_a, c_b, Edge::from((0, 0)));
            g.add_edge(p_b, c_b, Edge::from((0, 0)));
            g.add_edge(c_b, c_c, Edge::from((0, 0)));
            g.add_edge(p_c, c_c, Edge::from((0, 0)));
            push_ids = vec![p_a, p_b, p_c];
            counter_ids = vec![c_a, c_b, c_c];
        })
        .unwrap();

    // Check the expected stateful node order.
    {
        let g = project
            .ref_graph_node(&root)
            .expect("no graph for project root node");

        let (p_a, p_b, p_c) = (push_ids[0], push_ids[1], push_ids[2]);
        let (c_a, c_b, c_c) = (counter_ids[0], counter_ids[1], counter_ids[2]);

        let eval_order = gantz::graph::codegen::eval_order(&**g, push_ids, vec![]);
        let state_order = gantz::graph::codegen::state_order(&**g, eval_order).collect::<Vec<_>>();
        assert_eq!(state_order, counter_ids);

        // Check `a` evaluation and state ordering.
        let a_eval_order = gantz::graph::codegen::push_eval_order(&**g, p_a).collect::<Vec<_>>();
        assert_eq!(a_eval_order, vec![p_a, c_a, c_b, c_c]);
        let a_state_order =
            gantz::graph::codegen::state_order(&**g, a_eval_order).collect::<Vec<_>>();
        assert_eq!(a_state_order, vec![c_a, c_b, c_c]);

        // Check `b` evaluation and state ordering.
        let b_eval_order = gantz::graph::codegen::push_eval_order(&**g, p_b).collect::<Vec<_>>();
        assert_eq!(b_eval_order, vec![p_b, c_b, c_c]);
        let b_state_order =
            gantz::graph::codegen::state_order(&**g, b_eval_order).collect::<Vec<_>>();
        assert_eq!(b_state_order, vec![c_b, c_c]);

        // Check `c` evaluation and state ordering.
        let c_eval_order = gantz::graph::codegen::push_eval_order(&**g, p_c).collect::<Vec<_>>();
        assert_eq!(c_eval_order, vec![p_c, c_c]);
        let c_state_order =
            gantz::graph::codegen::state_order(&**g, c_eval_order).collect::<Vec<_>>();
        assert_eq!(c_state_order, vec![c_c]);
    }

    // Initialise the counter states.
    let mut a = 0u32;
    let mut b = 0u32;
    let mut c = 0u32;

    // Retrieve the path to the compiled library.
    let dylib_path = project
        .graph_node_dylib(&root)
        .unwrap()
        .expect("no dylib or node");

    // Load the library.
    let lib = libloading::Library::new(&dylib_path).expect("failed to load library");
    unsafe {
        type PushEvalFn = fn(&mut [&mut dyn std::any::Any]);
        type PushEvalFnSymbol<'a> = libloading::Symbol<'a, PushEvalFn>;
        let push_a_fn: PushEvalFnSymbol = lib.get(push_a_name.as_bytes()).unwrap();
        let push_b_fn: PushEvalFnSymbol = lib.get(push_b_name.as_bytes()).unwrap();
        let push_c_fn: PushEvalFnSymbol = lib.get(push_c_name.as_bytes()).unwrap();

        // Ensure the order of node states matches the expected state order for each eval function.
        push_a_fn(&mut [&mut a as _, &mut b as _, &mut c as _]);
        push_b_fn(&mut [&mut b as _, &mut c as _]);
        push_c_fn(&mut [&mut c as _]);
    }

    // Check the counter was incremented 3 times.
    assert_eq!([a, b, c], [1, 2, 3]);
}
