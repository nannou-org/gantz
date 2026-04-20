//! Integration tests for `node::rust` - writing Node expressions as Rust fns.

use gantz_core::{
    Edge, ROOT_STATE,
    compile::{default_entrypoints, entrypoint, eval_fn_name},
    node::{self, Node, WithPushEval},
};
use std::fmt::Debug;
use steel::{SteelVal, steel_vm::engine::Engine};

// Helper trait for debugging the graph.
trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A no-op node lookup function for tests that don't need it.
fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

/// Simple node for pushing evaluation through the graph.
fn node_push() -> node::Push<node::Expr> {
    node::expr("'()").unwrap().with_push_eval()
}

// ---------------------------------------------------------------------------
// Stateless: 2-input add
// ---------------------------------------------------------------------------

/// A Rust fn that adds two integer values.
fn rust_add(a: SteelVal, b: SteelVal) -> SteelVal {
    match (&a, &b) {
        (SteelVal::IntV(x), SteelVal::IntV(y)) => SteelVal::IntV(x + y),
        _ => SteelVal::Void,
    }
}

/// A node that adds two inputs using a registered Rust function.
#[derive(Debug)]
struct RustAdd;

impl Node for RustAdd {
    fn n_inputs(&self, _: node::MetaCtx) -> usize {
        2
    }
    fn n_outputs(&self, _: node::MetaCtx) -> usize {
        1
    }
    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        let inputs = ctx.inputs();
        match (inputs.get(0), inputs.get(1)) {
            (Some(Some(l)), Some(Some(r))) => node::rust::expr("rust-add", &[l, r]),
            _ => node::parse_expr("'()"),
        }
    }
    fn register(&self, mut ctx: node::RegCtx<'_, '_>) {
        node::rust::register(ctx.vm(), "rust-add", rust_add);
    }
}

/// Build a graph: push -> [one, one] -> add -> assert_eq(result, 2.0)
///
///    --------
///    | push |
///    -+------
///     |
///    -+-----
///    | one |
///    -+-----
///     |\
///     | \
///    -+--+------
///    | rust_add |
///    -+---------
///     |           -+-----
///     |           | two |
///     |           -+-----
///     |            |
///    -+------------+-
///    |  assert_eq   |
///    ----------------
#[test]
fn test_stateless_rust_add() {
    let mut g = petgraph::graph::DiGraph::new();

    let push = node_push();
    let one = node::expr("(begin $push 1)").unwrap();
    let add = RustAdd;
    let two = node::expr("(begin $push 2)").unwrap();
    let assert_eq = node::expr("(assert! (equal? $l $r))").unwrap();

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

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in module {
        vm.run(format!("{f}")).unwrap();
    }
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&eval_fn_name(&ep.id()), vec![])
        .unwrap();
}

// ---------------------------------------------------------------------------
// Stateful: counter
// ---------------------------------------------------------------------------

/// A Rust fn that increments an integer state, returning the new count.
fn rust_counter(_trigger: SteelVal, state: &mut SteelVal) -> SteelVal {
    match state {
        SteelVal::IntV(n) => {
            *n += 1;
            state.clone()
        }
        _ => SteelVal::Void,
    }
}

/// A stateful counter node using a registered Rust function.
#[derive(Debug)]
struct RustCounter;

impl Node for RustCounter {
    fn n_inputs(&self, _: node::MetaCtx) -> usize {
        1
    }
    fn n_outputs(&self, _: node::MetaCtx) -> usize {
        1
    }
    fn stateful(&self, _: node::MetaCtx) -> bool {
        true
    }
    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        match ctx.inputs().first() {
            Some(Some(trigger)) => node::rust::expr_stateful("rust-counter", &[trigger]),
            _ => node::parse_expr("state"),
        }
    }
    fn register(&self, mut ctx: node::RegCtx<'_, '_>) {
        node::rust::register_stateful(ctx.vm(), "rust-counter", rust_counter);
        let path = ctx.path().to_vec();
        node::state::init_value_if_absent(ctx.vm(), &path, || SteelVal::IntV(0)).unwrap();
    }
}

/// Build: push -> counter, call push 3 times, verify state = 3.
#[test]
fn test_stateful_rust_counter() {
    let mut g = petgraph::graph::DiGraph::new();

    let push = node_push();
    let counter = RustCounter;

    let push = g.add_node(Box::new(push) as Box<dyn DebugNode>);
    let counter = g.add_node(Box::new(counter) as Box<_>);
    g.add_edge(push, counter, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 3 times.
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    let fn_name = eval_fn_name(&ep.id());
    for _ in 0..3 {
        vm.call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
    }

    // Verify counter state is 3.
    let val = node::state::extract_value(&vm, &[counter.index()])
        .unwrap()
        .unwrap();
    assert_eq!(val, SteelVal::IntV(3));
}

// ---------------------------------------------------------------------------
// Zero-arg stateless fn
// ---------------------------------------------------------------------------

/// A Rust fn that takes no arguments and returns a constant.
fn rust_forty_two() -> SteelVal {
    SteelVal::IntV(42)
}

/// A zero-input node using a registered Rust function.
#[derive(Debug)]
struct RustFortyTwo;

impl Node for RustFortyTwo {
    fn n_inputs(&self, _: node::MetaCtx) -> usize {
        0
    }
    fn n_outputs(&self, _: node::MetaCtx) -> usize {
        1
    }
    fn expr(&self, _ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        node::rust::expr("rust-forty-two", &[])
    }
    fn register(&self, mut ctx: node::RegCtx<'_, '_>) {
        node::rust::register(ctx.vm(), "rust-forty-two", rust_forty_two);
    }
}

/// Build: forty_two -> assert_eq <- literal_42, pull eval from assert_eq.
#[test]
fn test_zero_arg_stateless() {
    use gantz_core::node::WithPullEval;

    let mut g = petgraph::graph::DiGraph::new();

    let forty_two = RustFortyTwo;
    let expected = node::expr("42").unwrap();
    let assert_eq = node::expr("(assert! (equal? $l $r))")
        .unwrap()
        .with_pull_eval();

    let ft = g.add_node(Box::new(forty_two) as Box<dyn DebugNode>);
    let ex = g.add_node(Box::new(expected) as Box<_>);
    let aeq = g.add_node(Box::new(assert_eq) as Box<_>);

    g.add_edge(ft, aeq, Edge::from((0, 0)));
    g.add_edge(ex, aeq, Edge::from((0, 1)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in module {
        vm.run(format!("{f}")).unwrap();
    }
    let ep = entrypoint::pull(vec![aeq.index()], g[aeq].n_inputs(ctx) as u8);
    vm.call_function_by_name_with_args(&eval_fn_name(&ep.id()), vec![])
        .unwrap();
}

// ---------------------------------------------------------------------------
// State-only fn (no positional args)
// ---------------------------------------------------------------------------

/// A Rust fn with state only (no positional inputs).
fn rust_tick(state: &mut SteelVal) -> SteelVal {
    match state {
        SteelVal::IntV(n) => {
            *n += 1;
            state.clone()
        }
        _ => SteelVal::Void,
    }
}

/// A stateful node with no positional inputs.
#[derive(Debug)]
struct RustTick;

impl Node for RustTick {
    fn n_inputs(&self, _: node::MetaCtx) -> usize {
        0
    }
    fn n_outputs(&self, _: node::MetaCtx) -> usize {
        1
    }
    fn stateful(&self, _: node::MetaCtx) -> bool {
        true
    }
    fn expr(&self, _ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        node::rust::expr_stateful("rust-tick", &[])
    }
    fn register(&self, mut ctx: node::RegCtx<'_, '_>) {
        node::rust::register_stateful(ctx.vm(), "rust-tick", rust_tick);
        let path = ctx.path().to_vec();
        node::state::init_value_if_absent(ctx.vm(), &path, || SteelVal::IntV(0)).unwrap();
    }
}

/// Build: tick (pull eval), call 5 times, verify state = 5.
#[test]
fn test_state_only_fn() {
    use gantz_core::node::WithPullEval;

    let mut g = petgraph::graph::DiGraph::new();

    let tick = RustTick;
    // Wrap tick with pull eval.
    let tick_pull = node::expr("$x").unwrap().with_pull_eval();

    let tick_ix = g.add_node(Box::new(tick) as Box<dyn DebugNode>);
    let pull_ix = g.add_node(Box::new(tick_pull) as Box<_>);

    g.add_edge(tick_ix, pull_ix, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    let ep = entrypoint::pull(vec![pull_ix.index()], g[pull_ix].n_inputs(ctx) as u8);
    let fn_name = eval_fn_name(&ep.id());
    for _ in 0..5 {
        vm.call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
    }

    let val = node::state::extract_value(&vm, &[tick_ix.index()])
        .unwrap()
        .unwrap();
    assert_eq!(val, SteelVal::IntV(5));
}

// ---------------------------------------------------------------------------
// Stateful: typed counter (isize state, no SteelVal matching)
// ---------------------------------------------------------------------------

/// A Rust fn with typed `isize` state - no manual `SteelVal` matching needed.
fn rust_typed_counter(_trigger: SteelVal, state: &mut isize) -> isize {
    *state += 1;
    *state
}

/// A stateful counter node using typed state.
#[derive(Debug)]
struct RustTypedCounter;

impl Node for RustTypedCounter {
    fn n_inputs(&self, _: node::MetaCtx) -> usize {
        1
    }
    fn n_outputs(&self, _: node::MetaCtx) -> usize {
        1
    }
    fn stateful(&self, _: node::MetaCtx) -> bool {
        true
    }
    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        match ctx.inputs().first() {
            Some(Some(trigger)) => node::rust::expr_stateful("rust-typed-counter", &[trigger]),
            _ => node::parse_expr("state"),
        }
    }
    fn register(&self, mut ctx: node::RegCtx<'_, '_>) {
        node::rust::register_stateful(ctx.vm(), "rust-typed-counter", rust_typed_counter);
        let path = ctx.path().to_vec();
        node::state::init_value_if_absent(ctx.vm(), &path, || SteelVal::IntV(0)).unwrap();
    }
}

/// Build: push -> typed_counter, call push 3 times, verify state = 3.
#[test]
fn test_stateful_typed_counter() {
    let mut g = petgraph::graph::DiGraph::new();

    let push = node_push();
    let counter = RustTypedCounter;

    let push = g.add_node(Box::new(push) as Box<dyn DebugNode>);
    let counter = g.add_node(Box::new(counter) as Box<_>);
    g.add_edge(push, counter, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 3 times.
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    let fn_name = eval_fn_name(&ep.id());
    for _ in 0..3 {
        vm.call_function_by_name_with_args(&fn_name, vec![])
            .unwrap();
    }

    // Verify counter state is 3.
    let val = node::state::extract_value(&vm, &[counter.index()])
        .unwrap()
        .unwrap();
    assert_eq!(val, SteelVal::IntV(3));
}

// ---------------------------------------------------------------------------
// Closure with captured state
// ---------------------------------------------------------------------------

/// Test that closures with captured variables work with `register`.
#[test]
fn test_closure_with_capture() {
    use gantz_core::node::WithPullEval;

    let multiplier: isize = 10;

    /// A node that multiplies its input by a captured constant.
    #[derive(Debug)]
    struct RustMul;

    impl Node for RustMul {
        fn n_inputs(&self, _: node::MetaCtx) -> usize {
            1
        }
        fn n_outputs(&self, _: node::MetaCtx) -> usize {
            1
        }
        fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
            match ctx.inputs().first() {
                Some(Some(x)) => node::rust::expr("rust-mul", &[x]),
                _ => node::parse_expr("'()"),
            }
        }
        fn register(&self, _ctx: node::RegCtx<'_, '_>) {
            // Registration of the closure happens externally in this test.
        }
    }

    let mut g = petgraph::graph::DiGraph::new();

    let five = node::expr("5").unwrap();
    let mul = RustMul;
    let expected = node::expr("50").unwrap();
    let assert_eq = node::expr("(assert! (equal? $l $r))")
        .unwrap()
        .with_pull_eval();

    let five_ix = g.add_node(Box::new(five) as Box<dyn DebugNode>);
    let mul_ix = g.add_node(Box::new(mul) as Box<_>);
    let exp_ix = g.add_node(Box::new(expected) as Box<_>);
    let aeq_ix = g.add_node(Box::new(assert_eq) as Box<_>);

    g.add_edge(five_ix, mul_ix, Edge::from((0, 0)));
    g.add_edge(mul_ix, aeq_ix, Edge::from((0, 0)));
    g.add_edge(exp_ix, aeq_ix, Edge::from((0, 1)));

    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());

    // Register the closure externally (captures `multiplier`).
    node::rust::register(&mut vm, "rust-mul", move |x: SteelVal| -> SteelVal {
        match x {
            SteelVal::IntV(v) => SteelVal::IntV(v * multiplier),
            _ => SteelVal::Void,
        }
    });

    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    let ctx = node::MetaCtx::new(&no_lookup);
    for f in module {
        vm.run(format!("{f}")).unwrap();
    }
    let ep = entrypoint::pull(vec![aeq_ix.index()], g[aeq_ix].n_inputs(ctx) as u8);
    vm.call_function_by_name_with_args(&eval_fn_name(&ep.id()), vec![])
        .unwrap();
}
