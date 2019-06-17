// Testing a simple counter node, the first stateful node type to be tested.

use gantz::node::{self, SerdeNode, WithPushEval, WithStateType};
use gantz::Edge;

fn node_push() -> node::Push<node::Expr> {
    node::expr("()").unwrap().with_push_eval_name("push")
}

fn node_counter() -> node::State<node::Expr> {
    node::expr(r#"{ #push; let count = *state; *state += 1; count }"#)
        .unwrap()
        .with_state_ty("u32")
        .unwrap()
}

#[test]
fn test_graph_with_counter() {
    // Create a temp project.
    let mut project = gantz::TempProject::open_with_name("test_graph_with_counter").unwrap();

    // Instantiate the nodes.
    let push = node_push();
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
    let symbol_name = "push".as_bytes();
    unsafe {
        let push_eval_fn: libloading::Symbol<fn(&mut [&mut dyn std::any::Any])> =
            lib.get(symbol_name).expect("failed to load symbol");
        // Prepare the `node_states` and execute the graph.
        let mut node_states = [&mut count as &mut dyn std::any::Any];
        push_eval_fn(&mut node_states[..]);
        push_eval_fn(&mut node_states[..]);
        push_eval_fn(&mut node_states[..]);
    }

    // Check the counter was incremented 3 times.
    assert_eq!(count, 3);
}
