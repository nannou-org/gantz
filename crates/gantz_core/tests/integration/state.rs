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

// A counter node. It increases its `u32` state by `1` on every input of any
// type.
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

// A counter driven by the `$x` input instead of `$push`, so it can live
// inside a nested graph.
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

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

// The simplest test graph for state.
//
//    --------
//    | push | // push_eval
//    -+------
//     |
//    -+---------
//    | counter |
//    -+---------
//
// The test calls the `push` node's eval fn three times once loaded.
#[test]
fn test_graph_with_counter() {
    let mut g = petgraph::graph::DiGraph::new();

    let push = node_push();
    let counter = node_counter();

    let push = g.add_node(Box::new(push) as Box<dyn DebugNode>);
    let counter = g.add_node(Box::new(counter) as Box<_>);
    g.add_edge(push, counter, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();

    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    let fn_name = entry_fn_name(&ep.id());
    for _ in 0..3 {
        vm.call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
    }

    let res = node::state::extract::<Counter>(&vm, &[counter.index()])
        .unwrap()
        .unwrap();
    assert_eq!(res, Counter(3));

    node::state::update(&mut vm, &[counter.index()], Counter(0)).unwrap();
    let res = node::state::extract::<Counter>(&vm, &[counter.index()])
        .unwrap()
        .unwrap();
    assert_eq!(res, Counter(0));

    // The next call increments from the new state, so the value is 1.
    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();

    let res = node::state::extract::<Counter>(&vm, &[counter.index()])
        .unwrap()
        .unwrap();
    assert_eq!(res, Counter(1));
}

// A larger test of state.
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

    let push_a = node_push();
    let push_b = node_push();
    let push_c = node_push();

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

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();

    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

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

// `move_value` rekeys a top-level node, carrying its whole nested subtree along.
#[test]
fn move_value_moves_nested_subtree() {
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());

    // Node 5 is a nested graph whose child 3 holds counter state.
    node::state::update(&mut vm, &[5, 3], Counter(7)).unwrap();
    assert_eq!(
        node::state::extract::<Counter>(&vm, &[5, 3]).unwrap(),
        Some(Counter(7)),
    );

    // Move node 5 to 2. Its child state must follow under the new key.
    node::state::move_value(&mut vm, &[5], &[2]).unwrap();
    assert_eq!(
        node::state::extract::<Counter>(&vm, &[2, 3]).unwrap(),
        Some(Counter(7)),
    );
    assert!(node::state::extract_value(&vm, &[5]).unwrap().is_none());

    // Moving from an empty slot is a no-op.
    node::state::move_value(&mut vm, &[9], &[1]).unwrap();
    assert!(node::state::extract_value(&vm, &[1]).unwrap().is_none());
}

// A plain `petgraph::Graph` swap-removes. The former-last node adopts the
// removed index. The GUI removal migration calls `remove_value` for the deleted
// node and then `move_value` for the swapped node. It must carry the swapped
// stateful node's state to its new index. The recompiled code reads state by
// the new index.
#[test]
fn swap_remove_migrates_stateful_node_state() {
    use gantz_core::node::graph::Graph;

    // Index 0 is a stateless standalone push. Index 1 is push_b. Index 2 is
    // counter_b, driven by push_b.
    let mut g: Graph<Box<dyn DebugNode>> = Graph::default();
    let p_a = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let p_b = g.add_node(Box::new(node_push()) as Box<_>);
    let c_b = g.add_node(Box::new(node_counter()) as Box<_>);
    g.add_edge(p_b, c_b, Edge::from((0, 0)));

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());

    // Compile, register and load the eval fns for the current graph shape.
    let load = |g: &Graph<Box<dyn DebugNode>>, vm: &mut Engine| {
        gantz_core::graph::register(&no_lookup, g, &[], vm);
        let eps = push_pull_entrypoints(&no_lookup, g);
        let module = gantz_core::compile::module(&no_lookup, g, &eps, &Default::default()).unwrap();
        for f in module {
            vm.run(format!("{f}")).unwrap();
        }
    };
    let ctx = node::MetaCtx::new(&no_lookup);
    let push = |g: &Graph<Box<dyn DebugNode>>, vm: &mut Engine, n: usize| {
        let ep = entrypoint::push(vec![n], g[node::graph::NodeIx::new(n)].n_outputs(ctx) as u8);
        vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
            .unwrap();
    };

    // Push b twice, so counter_b at index 2 reads 2.
    load(&g, &mut vm);
    push(&g, &mut vm, p_b.index());
    push(&g, &mut vm, p_b.index());
    assert_eq!(
        node::state::extract::<Counter>(&vm, &[c_b.index()]).unwrap(),
        Some(Counter(2)),
    );

    // Delete p_a (index 0). Swap-remove moves counter_b (last) into index 0.
    let last = g.node_count() - 1;
    assert_ne!(p_a.index(), last);
    node::state::remove_value(&mut vm, &[p_a.index()]).unwrap();
    g.remove_node(p_a);
    node::state::move_value(&mut vm, &[last], &[p_a.index()]).unwrap();

    // counter_b is now at index 0 and p_b stayed at index 1.
    let c_b = node::graph::NodeIx::new(0);
    let p_b = node::graph::NodeIx::new(1);
    load(&g, &mut vm);

    // The migrated state survived the move. One more push of b gives 3.
    assert_eq!(
        node::state::extract::<Counter>(&vm, &[c_b.index()]).unwrap(),
        Some(Counter(2)),
    );
    push(&g, &mut vm, p_b.index());
    assert_eq!(
        node::state::extract::<Counter>(&vm, &[c_b.index()]).unwrap(),
        Some(Counter(3)),
    );
}

// A stateful node must not leak memory per evaluation. The test drives one
// over a single persistent `Engine`, samples RSS, and asserts it stays bounded
// once warmed up.
//
// The risk is the mutated and captured `graph-state` local that
// `compile::emit` threads through the code. Steel must compile it to a
// GC-managed box that the collector reclaims, so that RSS plateaus.
//
// The test is `#[ignore]`d, so it is opt-in. The only available signal is
// process RSS from `/proc/self/statm`. That is Linux-only and conflates this
// graph's footprint with the rest of the shared test process. The test also
// drives about 1e6 VM calls. Steel exposes no public per-engine allocation
// count to measure instead. Run with:
//   cargo test -p gantz_core --test state -- --ignored --nocapture leak
#[cfg(target_os = "linux")]
#[test]
#[ignore]
fn stateful_eval_does_not_leak() {
    fn rss_bytes() -> usize {
        let statm = std::fs::read_to_string("/proc/self/statm").unwrap();
        let resident: usize = statm.split_whitespace().nth(1).unwrap().parse().unwrap();
        resident * 4096
    }

    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let counter = g.add_node(Box::new(node_counter()) as Box<_>);
    g.add_edge(push, counter, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    let fn_name = entry_fn_name(&ep.id());

    // Measure RSS growth after a warmup, so the initial heap ramp of about
    // 100k pushes is not counted. A leak adds about 0.8 KB per push, which is
    // tens of MB over the window. So growth beyond `MAX_GROWTH` means a leak.
    // The limit sits well above allocator noise.
    const PUSHES: usize = 1_000_000;
    const SAMPLE: usize = 100_000;
    const WARMUP: usize = 2 * SAMPLE;
    const MAX_GROWTH: usize = 100 * 1024 * 1024;

    let mut warmup_rss = 0;
    let mut last_rss = 0;
    for i in 0..PUSHES {
        vm.call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
        if i % SAMPLE == 0 {
            let rss = rss_bytes();
            eprintln!("push {i:>9}: RSS {:>6} MB", rss / (1024 * 1024));
            if i == WARMUP {
                warmup_rss = rss;
            }
            last_rss = rss;
        }
    }
    let growth = last_rss.saturating_sub(warmup_rss);
    eprintln!(
        "RSS grew {} MB ({} bytes/push) after warmup",
        growth / (1024 * 1024),
        growth / (PUSHES - WARMUP),
    );
    assert!(
        growth < MAX_GROWTH,
        "RSS grew {} MB after warmup - stateful-node leak (#266) is back",
        growth / (1024 * 1024),
    );
}

// Two `Ref`s to the same nested graph commit, at different positions in the
// parent, must keep independent runtime state. State is keyed by the ref's
// positional path through `graph-fn-{path}` and a per-path state slot, not by
// the shared graph's identity. So a shared definition still yields
// per-instance state.
#[test]
fn nested_ref_instances_have_independent_state() {
    use gantz_core::node::Ref;
    type Nested = node::graph::Graph<Box<dyn DebugNode>>;

    // The nested graph chains inlet, counter and outlet. Each evaluation
    // increments the counter's state.
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

    // Each push evaluates both refs and increments each counter once. Push
    // twice.
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    let fn_name = entry_fn_name(&ep.id());
    for _ in 0..2 {
        vm.call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
    }

    // Each instance counted 2. A shared slot would read 4.
    let s1 = node::state::extract::<Counter>(&vm, &[ref1.index(), counter_ix])
        .unwrap()
        .unwrap();
    let s2 = node::state::extract::<Counter>(&vm, &[ref2.index(), counter_ix])
        .unwrap()
        .unwrap();
    assert_eq!([s1, s2], [Counter(2), Counter(2)]);
}
