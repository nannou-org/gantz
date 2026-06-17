//! Tests related to node statefulness.

use gantz_core::{
    Edge, ROOT_STATE,
    compile::{entry_fn_name, entrypoint, push_pull_entrypoints},
    node::{self, Node, NodeState, WithPushEval, WithStateType},
};
use std::fmt::Debug;
use steel::{
    SteelVal,
    steel_vm::{engine::Engine, register_fn::RegisterFn},
};
use steel_derive::Steel;

/// Simple node for pushing evaluation through the graph.
fn node_push() -> node::Push<node::Expr> {
    node::expr("'()").unwrap().with_push_eval()
}

// A simple counter node.
//
// Increases its `u32` state by `1` each time it receives an input of any type.
fn node_counter() -> node::State<node::Expr, Counter> {
    let expr = r#"
        (begin
          $push
          (let ((value (counter-value state)))
            (counter-increment state)
            state))
    "#;
    node::expr(expr).unwrap().with_state_type::<Counter>()
}

// A counter driven by a nested graph's inlet (input `$x`) rather than `$push`,
// so it can live inside a nested graph.
fn node_inlet_counter() -> node::State<node::Expr, Counter> {
    let expr = r#"
        (begin
          $x
          (counter-increment state)
          (counter-value state))
    "#;
    node::expr(expr).unwrap().with_state_type::<Counter>()
}

/// The state type used for the counter.
#[derive(Clone, Debug, Default, PartialEq, Steel)]
struct Counter(u32);

impl Counter {
    fn increment(&mut self) {
        self.0 += 1;
    }

    fn value(&self) -> u32 {
        self.0
    }
}

impl NodeState for Counter {
    const NAME: &str = "Counter";
    fn register_fns(vm: &mut Engine) {
        vm.register_fn("counter-increment", Self::increment);
        vm.register_fn("counter-value", Self::value);
    }
}

// Helper trait for debugging the graph.
trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A no-op node lookup function for tests that don't need it.
fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
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
    let mut g = petgraph::graph::DiGraph::new();

    // Instantiate the nodes.
    let push = node_push();
    let counter = node_counter();

    // Add the nodes to the graph.
    let push = g.add_node(Box::new(push) as Box<dyn DebugNode>);
    let counter = g.add_node(Box::new(counter) as Box<_>);
    g.add_edge(push, counter, Edge::from((0, 0)));

    // Generate the module, which should have just one top-level expr for `push`.
    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    // Initialise the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    // Initialise the eval fn.
    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    // Call the push eval fn 3 times to increment the counter thrice.
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    let fn_name = entry_fn_name(&ep.id());
    for _ in 0..3 {
        vm.call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
    }

    // Check the counter was incremented thrice.
    let res = node::state::extract::<Counter>(&vm, &[counter.index()])
        .unwrap()
        .unwrap();
    assert_eq!(res, Counter(3));

    // Set the value back to `0`.
    node::state::update(&mut vm, &[counter.index()], Counter(0)).unwrap();
    let res = node::state::extract::<Counter>(&vm, &[counter.index()])
        .unwrap()
        .unwrap();
    assert_eq!(res, Counter(0));

    // Check that calling the function again works based on the new state.
    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();

    // The value should now be 1.
    let res = node::state::extract::<Counter>(&vm, &[counter.index()])
        .unwrap()
        .unwrap();
    assert_eq!(res, Counter(1));
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
    let mut g = petgraph::graph::DiGraph::new();

    // Instantiate the nodes.
    let push_a = node_push();
    let push_b = node_push();
    let push_c = node_push();

    // Add the nodes to the project.
    let p_a = g.add_node(Box::new(push_a) as Box<dyn DebugNode>);
    let p_b = g.add_node(Box::new(push_b) as Box<_>);
    let p_c = g.add_node(Box::new(push_c) as Box<_>);
    let c_a = g.add_node(Box::new(node_counter()) as Box<_>);
    let c_b = g.add_node(Box::new(node_counter()) as Box<_>);
    let c_c = g.add_node(Box::new(node_counter()) as Box<_>);
    g.add_edge(p_a, c_a, Edge::from((0, 0)));
    g.add_edge(c_a, c_b, Edge::from((0, 0)));
    g.add_edge(p_b, c_b, Edge::from((0, 0)));
    g.add_edge(c_b, c_c, Edge::from((0, 0)));
    g.add_edge(p_c, c_c, Edge::from((0, 0)));

    // Generate the module, which should have one expr for each `push`.
    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    // Initialise the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    // Initialise the eval fns.
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Call a, b then c.
    let ep_a = entrypoint::push(vec![p_a.index()], g[p_a].n_outputs(ctx) as u8);
    let ep_b = entrypoint::push(vec![p_b.index()], g[p_b].n_outputs(ctx) as u8);
    let ep_c = entrypoint::push(vec![p_c.index()], g[p_c].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_a.id()), vec![])
        .unwrap();
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_b.id()), vec![])
        .unwrap();
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_c.id()), vec![])
        .unwrap();

    // A should be incremented once, b twice, and c thrice.
    let a = node::state::extract::<Counter>(&vm, &[c_a.index()])
        .unwrap()
        .unwrap();
    let b = node::state::extract::<Counter>(&vm, &[c_b.index()])
        .unwrap()
        .unwrap();
    let c = node::state::extract::<Counter>(&vm, &[c_c.index()])
        .unwrap()
        .unwrap();
    assert_eq!([a, b, c], [Counter(1), Counter(2), Counter(3)]);
}

// Two `Ref`s to the *same* nested graph commit, at different positions in the
// parent, must keep independent runtime state. State is keyed by the ref's
// positional path (`graph-fn-{path}` + a per-path state slot), not by the
// shared graph's identity - so a shared definition still yields per-instance
// state, exactly as the old inline `GraphNode` (a separate copy) did.
#[test]
fn nested_ref_instances_have_independent_state() {
    use gantz_core::node::Ref;
    type Nested = node::graph::Graph<Box<dyn DebugNode>>;

    // Nested graph: inlet -> counter -> outlet. Each evaluation increments the
    // counter's state.
    let mut inner = Nested::default();
    let i = inner.add_node(Box::new(node::graph::Inlet::default()) as Box<dyn DebugNode>);
    let c = inner.add_node(Box::new(node_inlet_counter()) as Box<_>);
    let o = inner.add_node(Box::new(node::graph::Outlet::default()) as Box<_>);
    inner.add_edge(i, c, Edge::from((0, 0)));
    inner.add_edge(c, o, Edge::from((0, 0)));
    let counter_ix = c.index();

    // Reference the same nested graph commit from two positions.
    let inner_ca = gantz_ca::ContentAddr::from([1u8; 32]);
    let get_node = |ca: &gantz_ca::ContentAddr| -> Option<&dyn Node> {
        (*ca == inner_ca).then_some(&inner as &dyn Node)
    };

    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let ref1 = g.add_node(Box::new(Ref::new(inner_ca)) as Box<_>);
    let ref2 = g.add_node(Box::new(Ref::new(inner_ca)) as Box<_>);
    g.add_edge(push, ref1, Edge::from((0, 0)));
    g.add_edge(push, ref2, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&get_node);
    let eps = push_pull_entrypoints(&get_node, &g);
    let module = gantz_core::compile::module(&get_node, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&get_node, &g, &[], &mut vm);
    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    // Each push evaluates both refs, incrementing each counter once; push twice.
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    let fn_name = entry_fn_name(&ep.id());
    for _ in 0..2 {
        vm.call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
    }

    // Independent state: each instance counted 2 (a shared slot would read 4).
    let s1 = node::state::extract::<Counter>(&vm, &[ref1.index(), counter_ix])
        .unwrap()
        .unwrap();
    let s2 = node::state::extract::<Counter>(&vm, &[ref2.index(), counter_ix])
        .unwrap()
        .unwrap();
    assert_eq!([s1, s2], [Counter(2), Counter(2)]);
}
