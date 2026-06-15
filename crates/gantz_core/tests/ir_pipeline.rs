//! Shape coverage for the IR pipeline (`compile::module`).
//!
//! These shapes were developed as a differential suite against the old
//! flow-graph pipeline before the cutover. Each compiles and runs a call
//! sequence end-to-end: in-graph `assert!` nodes, explicit state
//! assertions, and runtime errors carry the verification (`tests/graph.rs`
//! and `tests/nested.rs` assert overlapping shapes' behavior in detail).

use gantz_core::compile::{Entrypoint, entry_fn_name, entrypoint, push_pull_entrypoints};
use gantz_core::node::{self, Node, WithPullEval, WithPushEval};
use gantz_core::{Edge, ROOT_STATE};
use std::fmt::Debug;
use steel::SteelVal;
use steel::steel_vm::engine::Engine;

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

type Graph = petgraph::graph::DiGraph<Box<dyn DebugNode>, Edge>;

fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

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

/// Stores the received number in state and returns it.
fn node_number() -> node::Expr {
    node::expr(
        "
        (let ((x $x))
          (set! state (if (number? x) x state))
          state)
    ",
    )
    .unwrap()
}

/// A 1-in 2-out select: input 0 takes arm 0, anything else arm 1.
#[derive(Debug)]
struct Select;

impl Node for Select {
    fn n_inputs(&self, _ctx: node::MetaCtx) -> usize {
        1
    }
    fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
        2
    }
    fn branches(&self, _ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        vec![
            node::EvalConf::Set([true, false].try_into().unwrap()),
            node::EvalConf::Set([false, true].try_into().unwrap()),
        ]
    }
    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        let x = ctx.inputs()[0].as_deref().expect("must have one input");
        node::parse_expr(&format!("(if (equal? 0 {x}) (list 0 {x}) (list 1 {x}))"))
    }
}

/// Run `module` in a fresh VM, call the entrypoints in `calls` order, and
/// return the final root state.
fn run_pipeline(
    label: &str,
    module: &[steel::parser::ast::ExprKind],
    g: &Graph,
    calls: &[&Entrypoint],
) -> SteelVal {
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, g, &[], &mut vm);
    for f in module {
        vm.run(f.to_pretty(100))
            .unwrap_or_else(|e| panic!("{label}: failed to load module: {e}"));
    }
    for ep in calls {
        vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
            .unwrap_or_else(|e| panic!("{label}: entry fn call failed: {e}"));
    }
    vm.extract_value(ROOT_STATE).unwrap()
}

/// Compile `g` and run the call sequence end-to-end.
fn assert_pipelines_agree(g: &Graph, eps: &[Entrypoint], calls: &[&Entrypoint]) {
    let m = gantz_core::compile::module(&no_lookup, g, eps, &Default::default())
        .expect("IR pipeline failed");
    if std::env::var("GANTZ_DUMP").is_ok() {
        for e in &m {
            eprintln!("{}\n", e.to_pretty(100));
        }
    }
    let _ = run_pipeline("ir", &m, g, calls);
}

/// All push/pull entrypoints, each called once in order.
fn agree_on_all_entrypoints(g: &Graph) {
    let eps = push_pull_entrypoints(&no_lookup, g);
    let calls: Vec<&Entrypoint> = eps.iter().collect();
    assert_pipelines_agree(g, &eps, &calls);
}

/// push -> one -> add(x2) plus push -> two, asserting 1 + 1 == 2.
#[test]
fn push_eval() {
    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let one = g.add_node(Box::new(node_int(1)) as Box<_>);
    let add = g.add_node(Box::new(node_add()) as Box<_>);
    let two = g.add_node(Box::new(node_int(2)) as Box<_>);
    let assert_eq = g.add_node(Box::new(node_assert_eq()) as Box<_>);
    g.add_edge(push, one, Edge::from((0, 0)));
    g.add_edge(push, two, Edge::from((0, 0)));
    g.add_edge(one, add, Edge::from((0, 0)));
    g.add_edge(one, add, Edge::from((0, 1)));
    g.add_edge(add, assert_eq, Edge::from((0, 0)));
    g.add_edge(two, assert_eq, Edge::from((0, 1)));
    agree_on_all_entrypoints(&g);
}

/// Pull evaluation of the same diamond.
#[test]
fn pull_eval() {
    let mut g = Graph::new();
    let one = g.add_node(Box::new(node_int(1)) as Box<dyn DebugNode>);
    let add = g.add_node(Box::new(node_add()) as Box<_>);
    let two = g.add_node(Box::new(node_int(2)) as Box<_>);
    let assert_eq = g.add_node(Box::new(node_assert_eq().with_pull_eval()) as Box<_>);
    g.add_edge(one, add, Edge::from((0, 0)));
    g.add_edge(one, add, Edge::from((0, 1)));
    g.add_edge(add, assert_eq, Edge::from((0, 0)));
    g.add_edge(two, assert_eq, Edge::from((0, 1)));
    agree_on_all_entrypoints(&g);
}

/// Two pushes into a select whose arms reconverge at a stateful number.
#[test]
fn push_cond_eval() {
    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let select = g.add_node(Box::new(Select) as Box<_>);
    let six = g.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(push_1, select, Edge::from((0, 0)));
    g.add_edge(select, six, Edge::from((0, 0)));
    g.add_edge(select, seven, Edge::from((1, 0)));
    g.add_edge(six, number, Edge::from((0, 0)));
    g.add_edge(seven, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// One arm goes directly to the join, the other through an intermediate.
#[test]
fn branch_target_is_join() {
    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let select = g.add_node(Box::new(Select) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(push_1, select, Edge::from((0, 0)));
    g.add_edge(select, number, Edge::from((0, 0)));
    g.add_edge(select, seven, Edge::from((1, 0)));
    g.add_edge(seven, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// Both branch outputs feed the same target input (arm-varying scalar).
#[test]
fn branch_both_outputs_same_target() {
    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let select = g.add_node(Box::new(Select) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(push_1, select, Edge::from((0, 0)));
    g.add_edge(select, number, Edge::from((0, 0)));
    g.add_edge(select, number, Edge::from((1, 0)));
    agree_on_all_entrypoints(&g);
}

/// Nested diamond: an inner branch inside the outer branch's first arm.
#[test]
fn nested_diamond() {
    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let select_outer = g.add_node(Box::new(Select) as Box<_>);
    let select_inner = g.add_node(Box::new(Select) as Box<_>);
    let six = g.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let inner_result = g.add_node(Box::new(node_number()) as Box<_>);
    let eight = g.add_node(Box::new(node_int(8)) as Box<_>);
    let outer_result = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, select_outer, Edge::from((0, 0)));
    g.add_edge(push_1, select_outer, Edge::from((0, 0)));
    g.add_edge(select_outer, select_inner, Edge::from((0, 0)));
    g.add_edge(select_outer, eight, Edge::from((1, 0)));
    g.add_edge(select_inner, six, Edge::from((0, 0)));
    g.add_edge(select_inner, seven, Edge::from((1, 0)));
    g.add_edge(six, inner_result, Edge::from((0, 0)));
    g.add_edge(seven, inner_result, Edge::from((0, 0)));
    g.add_edge(inner_result, outer_result, Edge::from((0, 0)));
    g.add_edge(eight, outer_result, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// Lattice: two inner branches share arm targets, all converging at one join.
#[test]
fn lattice_reconvergence() {
    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let select_outer = g.add_node(Box::new(Select) as Box<_>);
    let sel_l = g.add_node(Box::new(Select) as Box<_>);
    let sel_r = g.add_node(Box::new(Select) as Box<_>);
    let six = g.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, select_outer, Edge::from((0, 0)));
    g.add_edge(push_1, select_outer, Edge::from((0, 0)));
    g.add_edge(select_outer, sel_l, Edge::from((0, 0)));
    g.add_edge(select_outer, sel_r, Edge::from((1, 0)));
    g.add_edge(sel_l, six, Edge::from((0, 0)));
    g.add_edge(sel_l, seven, Edge::from((1, 0)));
    g.add_edge(sel_r, six, Edge::from((0, 0)));
    g.add_edge(sel_r, seven, Edge::from((1, 0)));
    g.add_edge(six, number, Edge::from((0, 0)));
    g.add_edge(seven, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// A combined entrypoint evaluating two independent chains in one call.
#[test]
fn multi_source_push() {
    let mut g = Graph::new();
    let push_a = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int_a = g.add_node(Box::new(node_int(42)) as Box<_>);
    let num_a = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_a, int_a, Edge::from((0, 0)));
    g.add_edge(int_a, num_a, Edge::from((0, 0)));
    let push_b = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int_b = g.add_node(Box::new(node_int(7)) as Box<_>);
    let num_b = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_b, int_b, Edge::from((0, 0)));
    g.add_edge(int_b, num_b, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let combined = entrypoint::from_sources([
        entrypoint::push_source(vec![push_a.index()], g[push_a].n_outputs(ctx) as u8),
        entrypoint::push_source(vec![push_b.index()], g[push_b].n_outputs(ctx) as u8),
    ]);
    assert_pipelines_agree(&g, std::slice::from_ref(&combined), &[&combined]);
}

/// A 2-output expr node feeding two separate stores.
#[test]
fn multi_output_expr() {
    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let pair = node::expr("(begin $push (list 6 7))")
        .unwrap()
        .with_outputs(2);
    let pair = g.add_node(Box::new(pair) as Box<_>);
    let num_a = g.add_node(Box::new(node_number()) as Box<_>);
    let num_b = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, pair, Edge::from((0, 0)));
    g.add_edge(pair, num_a, Edge::from((0, 0)));
    g.add_edge(pair, num_b, Edge::from((1, 0)));
    agree_on_all_entrypoints(&g);
}

/// Multiple zero-output side-effect leaves in one evaluation.
#[test]
fn zero_output_leaf_nodes() {
    #[derive(Debug)]
    struct Effect;
    impl Node for Effect {
        fn n_inputs(&self, _ctx: node::MetaCtx) -> usize {
            1
        }
        fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
            let input = ctx.inputs()[0].as_deref().unwrap_or("'()");
            node::parse_expr(&format!("(begin {input} '())"))
        }
    }
    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let effect1 = g.add_node(Box::new(Effect) as Box<dyn DebugNode>);
    let effect2 = g.add_node(Box::new(Effect) as Box<dyn DebugNode>);
    g.add_edge(push, effect1, Edge::from((0, 0)));
    g.add_edge(push, effect2, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// The `Branch` node type with reconvergence.
#[test]
fn branch_node() {
    let branch = node::Branch::new(
        "(if (equal? 0 $x) (list 0 '()) (list 1 '()))",
        vec![
            node::Conns::try_from([true, false]).unwrap(),
            node::Conns::try_from([false, true]).unwrap(),
        ],
    )
    .unwrap();
    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let branch_ix = g.add_node(Box::new(branch) as Box<_>);
    let six = g.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, branch_ix, Edge::from((0, 0)));
    g.add_edge(push_1, branch_ix, Edge::from((0, 0)));
    g.add_edge(branch_ix, six, Edge::from((0, 0)));
    g.add_edge(branch_ix, seven, Edge::from((1, 0)));
    g.add_edge(six, number, Edge::from((0, 0)));
    g.add_edge(seven, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// Multiple unconditional edges to one input pass as a list.
#[test]
fn multi_edge_input_list() {
    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let three = g.add_node(Box::new(node_int(3)) as Box<_>);
    let four = g.add_node(Box::new(node_int(4)) as Box<_>);
    let sum = g.add_node(Box::new(node::expr("(apply + $x)").unwrap()) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, three, Edge::from((0, 0)));
    g.add_edge(push, four, Edge::from((0, 0)));
    g.add_edge(three, sum, Edge::from((0, 0)));
    g.add_edge(four, sum, Edge::from((0, 0)));
    g.add_edge(sum, store, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// Branch arms ending at independent stateful terminals (no reconvergence).
#[test]
fn branch_divergent_terminal() {
    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let select = g.add_node(Box::new(Select) as Box<_>);
    let store_a = g.add_node(Box::new(node_number()) as Box<_>);
    let store_b = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(push_1, select, Edge::from((0, 0)));
    g.add_edge(select, store_a, Edge::from((0, 0)));
    g.add_edge(select, store_b, Edge::from((1, 0)));
    agree_on_all_entrypoints(&g);
}

/// Multi-edge list binding entirely within one branch arm.
#[test]
fn multi_edge_in_branch_arm() {
    #[derive(Debug)]
    struct Select2;
    impl Node for Select2 {
        fn n_inputs(&self, _ctx: node::MetaCtx) -> usize {
            1
        }
        fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
            2
        }
        fn branches(&self, _ctx: node::MetaCtx) -> Vec<node::EvalConf> {
            vec![
                node::EvalConf::Set([true, false].try_into().unwrap()),
                node::EvalConf::Set([false, true].try_into().unwrap()),
            ]
        }
        fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
            let x = ctx.inputs()[0].as_deref().expect("must have one input");
            node::parse_expr(&format!(
                "(if (equal? 0 {x}) (list 0 '() '()) (list 1 '() '()))"
            ))
        }
    }
    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let select = g.add_node(Box::new(Select2) as Box<_>);
    let three = g.add_node(Box::new(node_int(3)) as Box<_>);
    let four = g.add_node(Box::new(node_int(4)) as Box<_>);
    let sum = g.add_node(Box::new(node::expr("(apply + $x)").unwrap()) as Box<_>);
    let eight = g.add_node(Box::new(node_int(8)) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(push_1, select, Edge::from((0, 0)));
    g.add_edge(select, three, Edge::from((0, 0)));
    g.add_edge(select, four, Edge::from((0, 0)));
    g.add_edge(three, sum, Edge::from((0, 0)));
    g.add_edge(four, sum, Edge::from((0, 0)));
    g.add_edge(select, eight, Edge::from((1, 0)));
    g.add_edge(sum, number, Edge::from((0, 0)));
    g.add_edge(eight, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// Optional `$?` input, unconnected and connected.
#[test]
fn optional_input() {
    // Unconnected.
    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_int(5).with_push_eval()) as Box<dyn DebugNode>);
    let add_opt = g.add_node(Box::new(
        node::expr("(+ $a (if (Some? $?b) (Some->value $?b) 0))").unwrap(),
    ) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, add_opt, Edge::from((0, 0)));
    g.add_edge(add_opt, store, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);

    // Connected.
    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_int(5).with_push_eval()) as Box<dyn DebugNode>);
    let three = g.add_node(Box::new(node_int(3)) as Box<_>);
    let add_opt = g.add_node(Box::new(
        node::expr("(+ $a (if (Some? $?b) (Some->value $?b) 0))").unwrap(),
    ) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, add_opt, Edge::from((0, 0)));
    g.add_edge(push, three, Edge::from((0, 0)));
    g.add_edge(three, add_opt, Edge::from((0, 1)));
    g.add_edge(add_opt, store, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// A three-way branch with reconvergence.
#[test]
fn three_way_branch() {
    #[derive(Debug)]
    struct Select3;
    impl Node for Select3 {
        fn n_inputs(&self, _ctx: node::MetaCtx) -> usize {
            1
        }
        fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
            3
        }
        fn branches(&self, _ctx: node::MetaCtx) -> Vec<node::EvalConf> {
            vec![
                node::EvalConf::Set([true, false, false].try_into().unwrap()),
                node::EvalConf::Set([false, true, false].try_into().unwrap()),
                node::EvalConf::Set([false, false, true].try_into().unwrap()),
            ]
        }
        fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
            let x = ctx.inputs()[0].as_deref().expect("must have one input");
            node::parse_expr(&format!(
                "(if (equal? 0 {x}) (list 0 '() '() '()) \
                   (if (equal? 1 {x}) (list 1 '() '() '()) \
                     (list 2 '() '() '())))"
            ))
        }
    }
    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let push_2 = g.add_node(Box::new(node_int(2).with_push_eval()) as Box<_>);
    let select = g.add_node(Box::new(Select3) as Box<_>);
    let six = g.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let eight = g.add_node(Box::new(node_int(8)) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(push_1, select, Edge::from((0, 0)));
    g.add_edge(push_2, select, Edge::from((0, 0)));
    g.add_edge(select, six, Edge::from((0, 0)));
    g.add_edge(select, seven, Edge::from((1, 0)));
    g.add_edge(select, eight, Edge::from((2, 0)));
    g.add_edge(six, number, Edge::from((0, 0)));
    g.add_edge(seven, number, Edge::from((0, 0)));
    g.add_edge(eight, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// A dead arm (no active outputs) must not run downstream work.
#[test]
fn branch_single_output_dead_branch() {
    let branch = node::Branch::new(
        "(if (equal? 0 $x) (list 0 42) (list 1 99))",
        vec![
            node::Conns::try_from([true]).unwrap(),
            node::Conns::try_from([false]).unwrap(),
        ],
    )
    .unwrap();
    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let branch_ix = g.add_node(Box::new(branch) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, branch_ix, Edge::from((0, 0)));
    g.add_edge(push_1, branch_ix, Edge::from((0, 0)));
    g.add_edge(branch_ix, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// Two outputs, one fully dead arm.
#[test]
fn branch_two_outputs_one_dead() {
    let branch = node::Branch::new(
        "(if (equal? 0 $x) (list 0 (list 42 43)) (list 1 (list 99 100)))",
        vec![
            node::Conns::try_from([true, true]).unwrap(),
            node::Conns::try_from([false, false]).unwrap(),
        ],
    )
    .unwrap();
    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let branch_ix = g.add_node(Box::new(branch) as Box<_>);
    let store_a = g.add_node(Box::new(node_number()) as Box<_>);
    let store_b = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, branch_ix, Edge::from((0, 0)));
    g.add_edge(push_1, branch_ix, Edge::from((0, 0)));
    g.add_edge(branch_ix, store_a, Edge::from((0, 0)));
    g.add_edge(branch_ix, store_b, Edge::from((1, 0)));
    agree_on_all_entrypoints(&g);
}

/// Every arm dead: the branch's side effects run, nothing propagates.
#[test]
fn branch_all_dead() {
    let branch = node::Branch::new(
        "(begin (set! state $x) (if (equal? 0 $x) (list 0 '()) (list 1 '())))",
        vec![
            node::Conns::try_from([false]).unwrap(),
            node::Conns::try_from([false]).unwrap(),
        ],
    )
    .unwrap();
    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_int(42).with_push_eval()) as Box<dyn DebugNode>);
    let branch_ix = g.add_node(Box::new(branch) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, branch_ix, Edge::from((0, 0)));
    g.add_edge(branch_ix, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// Pd-style hot/cold `+`: cold push stores state without emitting, hot push
/// emits. Order matters: cold first, then hot.
#[test]
fn branch_optional_input_pd_add() {
    let pd_add = node::Branch::new(
        "(begin \
           (if (Some? $?b) (set! state (Some->value $?b)) '()) \
           (if (number? $a) \
               (list 0 (+ $a (if (number? state) state 0))) \
               (list 1 '())))",
        vec![
            node::Conns::try_from([true]).unwrap(),
            node::Conns::try_from([false]).unwrap(),
        ],
    )
    .unwrap();
    let mut g = Graph::new();
    let push_hot = g.add_node(Box::new(node_int(5).with_push_eval()) as Box<dyn DebugNode>);
    let push_cold = g.add_node(Box::new(node_int(3).with_push_eval()) as Box<_>);
    let pd_add_ix = g.add_node(Box::new(pd_add) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_cold, pd_add_ix, Edge::from((0, 0)));
    g.add_edge(push_hot, pd_add_ix, Edge::from((0, 1)));
    g.add_edge(pd_add_ix, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let ep_cold = entrypoint::push(vec![push_cold.index()], g[push_cold].n_outputs(ctx) as u8);
    let ep_hot = entrypoint::push(vec![push_hot.index()], g[push_hot].n_outputs(ctx) as u8);
    assert_pipelines_agree(&g, &eps, &[&ep_cold, &ep_hot]);
}

/// A stateful counter pushed several times.
#[test]
fn stateful_counter() {
    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let counter = g.add_node(Box::new(
        node::expr("(begin $push (set! state (+ (if (number? state) state 0) 1)) state)").unwrap(),
    ) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, counter, Edge::from((0, 0)));
    g.add_edge(counter, store, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    assert_pipelines_agree(&g, &eps, &[&ep, &ep, &ep]);
}

/// Two branches in one multi-source entrypoint reconverging at a shared add:
/// the join of the first branch depends on the second branch's result, so
/// the first branch must export its arm value past its own dispatch (the
/// shape behind the old cross-component root-ordering fix).
///
/// NOTE: this is asserted against the IR pipeline only - the flow pipeline
/// (`compile::module`) miscompiles this shape at runtime (`+` receives `'()`:
/// the second branch's join reads the first branch's arm value before it is
/// defined), a pre-existing bug beyond what `order_roots` fixed.
#[test]
fn two_branch_shared_join() {
    let mut g = Graph::new();
    let push_p = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_q = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let sel_p = g.add_node(Box::new(Select) as Box<_>);
    let sel_q = g.add_node(Box::new(Select) as Box<_>);
    let six = g.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let eight = g.add_node(Box::new(node_int(8)) as Box<_>);
    let nine = g.add_node(Box::new(node_int(9)) as Box<_>);
    let add = g.add_node(Box::new(node_add()) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_p, sel_p, Edge::from((0, 0)));
    g.add_edge(push_q, sel_q, Edge::from((0, 0)));
    g.add_edge(sel_p, six, Edge::from((0, 0)));
    g.add_edge(sel_p, seven, Edge::from((1, 0)));
    g.add_edge(sel_q, eight, Edge::from((0, 0)));
    g.add_edge(sel_q, nine, Edge::from((1, 0)));
    // Both of sel_p's arms feed add input 0; both of sel_q's feed input 1.
    g.add_edge(six, add, Edge::from((0, 0)));
    g.add_edge(seven, add, Edge::from((0, 0)));
    g.add_edge(eight, add, Edge::from((0, 1)));
    g.add_edge(nine, add, Edge::from((0, 1)));
    g.add_edge(add, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let combined = entrypoint::from_sources([
        entrypoint::push_source(vec![push_p.index()], g[push_p].n_outputs(ctx) as u8),
        entrypoint::push_source(vec![push_q.index()], g[push_q].n_outputs(ctx) as u8),
    ]);
    let eps = std::slice::from_ref(&combined);
    let m2 = gantz_core::compile::module(&no_lookup, &g, eps, &Default::default())
        .expect("IR pipeline failed");

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &m2 {
        vm.run(f.to_pretty(100)).unwrap();
    }
    vm.call_function_by_name_with_args(&entry_fn_name(&combined.id()), vec![])
        .unwrap();

    // push_p emits 0 -> sel_p arm 0 -> six; push_q emits 1 -> sel_q arm 1 ->
    // nine; add = 6 + 9 = 15.
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 15);
}

// ===========================================================================
// Nested-graph shapes (mirroring tests/nested.rs).
// ===========================================================================

use gantz_core::node::graph::{Inlet, Outlet};

// A nested graph: an ordinary `Graph` (which implements `Node`) boxed into its
// parent, in place of the removed `GraphNode` wrapper. (`Graph` here is the
// outer DiGraph alias, so the nested type is spelled out in full.)
type Nested = gantz_core::node::graph::Graph<Box<dyn DebugNode>>;

/// inlet x2 -> mul -> outlet.
fn graph_mul() -> Nested {
    let mut ga = Nested::default();
    let inlet_a = ga.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let inlet_b = ga.add_node(Box::new(Inlet) as Box<_>);
    let mul = ga.add_node(Box::new(node::expr("(* $l $r)").unwrap()) as Box<_>);
    let outlet = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(inlet_a, mul, Edge::from((0, 0)));
    ga.add_edge(inlet_b, mul, Edge::from((0, 1)));
    ga.add_edge(mul, outlet, Edge::from((0, 0)));
    ga
}

/// A stateless nested graph: 6 * 7 == 42 asserted in the parent.
#[test]
fn nested_stateless() {
    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let six = g.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let graph_a = g.add_node(Box::new(graph_mul()) as Box<_>);
    let forty_two = g.add_node(Box::new(node_int(42)) as Box<_>);
    let assert_eq = g.add_node(Box::new(node_assert_eq()) as Box<_>);
    g.add_edge(push, six, Edge::from((0, 0)));
    g.add_edge(push, seven, Edge::from((0, 0)));
    g.add_edge(push, forty_two, Edge::from((0, 0)));
    g.add_edge(six, graph_a, Edge::from((0, 0)));
    g.add_edge(seven, graph_a, Edge::from((0, 1)));
    g.add_edge(graph_a, assert_eq, Edge::from((0, 0)));
    g.add_edge(forty_two, assert_eq, Edge::from((0, 1)));
    agree_on_all_entrypoints(&g);
}

/// A stateful nested counter, pushed twice; inner state and the propagated
/// value must agree.
#[test]
fn nested_counter() {
    let counter =
        node::expr("(begin $bang (set! state (if (number? state) (+ state 1) 0)) state)").unwrap();
    let mut ga = Nested::default();
    let inlet = ga.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let counter = ga.add_node(Box::new(counter) as Box<_>);
    let outlet = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(inlet, counter, Edge::from((0, 0)));
    ga.add_edge(counter, outlet, Edge::from((0, 0)));

    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let graph_a = g.add_node(Box::new(ga) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, graph_a, Edge::from((0, 0)));
    g.add_edge(graph_a, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    assert_pipelines_agree(&g, &eps, &[&ep, &ep]);
}

/// An entrypoint sourced inside a nested graph, not reaching any outlet.
#[test]
fn nested_inner_push() {
    let mut ga = Nested::default();
    let push = ga.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let ten = ga.add_node(Box::new(node_int(10)) as Box<_>);
    let number = ga.add_node(Box::new(node_number()) as Box<_>);
    ga.add_edge(push, ten, Edge::from((0, 0)));
    ga.add_edge(ten, number, Edge::from((0, 0)));

    let mut g = Graph::new();
    let _graph_a = g.add_node(Box::new(ga) as Box<dyn DebugNode>);
    agree_on_all_entrypoints(&g);
}

/// An inner push propagating through the outlet to parent downstream.
#[test]
fn nested_push_through_outlet() {
    let mut ga = Nested::default();
    let push = ga.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let v = ga.add_node(Box::new(node_int(42)) as Box<_>);
    let outlet = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(push, v, Edge::from((0, 0)));
    ga.add_edge(v, outlet, Edge::from((0, 0)));

    let mut g = Graph::new();
    let graph_a = g.add_node(Box::new(ga) as Box<dyn DebugNode>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(graph_a, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// An inner push two levels deep, propagating through both outlets.
#[test]
fn nested_push_through_outlet_deep() {
    let mut inner = Nested::default();
    let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let v = inner.add_node(Box::new(node_int(33)) as Box<_>);
    let outlet = inner.add_node(Box::new(Outlet) as Box<_>);
    inner.add_edge(push, v, Edge::from((0, 0)));
    inner.add_edge(v, outlet, Edge::from((0, 0)));

    let mut mid = Nested::default();
    let inner_ix = mid.add_node(Box::new(inner) as Box<dyn DebugNode>);
    let double = mid.add_node(Box::new(node::expr("(* 2 $x)").unwrap()) as Box<_>);
    let mid_outlet = mid.add_node(Box::new(Outlet) as Box<_>);
    mid.add_edge(inner_ix, double, Edge::from((0, 0)));
    mid.add_edge(double, mid_outlet, Edge::from((0, 0)));

    let mut g = Graph::new();
    let mid_ix = g.add_node(Box::new(mid) as Box<dyn DebugNode>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(mid_ix, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// A nested graph with two outlets feeding two parent stores.
#[test]
fn nested_multi_outlet() {
    let mut ga = Nested::default();
    let inlet = ga.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let add1 = ga.add_node(Box::new(node::expr("(+ 1 $x)").unwrap()) as Box<_>);
    let add2 = ga.add_node(Box::new(node::expr("(+ 2 $x)").unwrap()) as Box<_>);
    let out1 = ga.add_node(Box::new(Outlet) as Box<_>);
    let out2 = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(inlet, add1, Edge::from((0, 0)));
    ga.add_edge(inlet, add2, Edge::from((0, 0)));
    ga.add_edge(add1, out1, Edge::from((0, 0)));
    ga.add_edge(add2, out2, Edge::from((0, 0)));

    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_int(10).with_push_eval()) as Box<dyn DebugNode>);
    let graph_a = g.add_node(Box::new(ga) as Box<_>);
    let num_a = g.add_node(Box::new(node_number()) as Box<_>);
    let num_b = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, graph_a, Edge::from((0, 0)));
    g.add_edge(graph_a, num_a, Edge::from((0, 0)));
    g.add_edge(graph_a, num_b, Edge::from((1, 0)));
    agree_on_all_entrypoints(&g);
}

/// Two nested pushes combined into one multi-source entrypoint, each
/// propagating through its graph's outlet.
#[test]
fn nested_multi_source_outlet_propagation() {
    let make_inner = || {
        let mut inner = Nested::default();
        let push = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
        let ten = inner.add_node(Box::new(node_int(10)) as Box<_>);
        let outlet = inner.add_node(Box::new(Outlet) as Box<_>);
        inner.add_edge(push, ten, Edge::from((0, 0)));
        inner.add_edge(ten, outlet, Edge::from((0, 0)));
        (inner, push)
    };
    let (inner_a, push_a) = make_inner();
    let (inner_b, push_b) = make_inner();

    let mut g = Graph::new();
    let graph_a = g.add_node(Box::new(inner_a) as Box<dyn DebugNode>);
    let graph_b = g.add_node(Box::new(inner_b) as Box<dyn DebugNode>);
    let num_a = g.add_node(Box::new(node_number()) as Box<_>);
    let num_b = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(graph_a, num_a, Edge::from((0, 0)));
    g.add_edge(graph_b, num_b, Edge::from((0, 0)));

    let combined = entrypoint::from_sources([
        entrypoint::push_source(vec![graph_a.index(), push_a.index()], 1),
        entrypoint::push_source(vec![graph_b.index(), push_b.index()], 1),
    ]);
    assert_pipelines_agree(&g, std::slice::from_ref(&combined), &[&combined]);
}

/// One entrypoint mixing a nested push (through an outlet) with a direct
/// root push, both converging at an add.
#[test]
fn nested_mixed_level_multi_source() {
    let mut inner = Nested::default();
    let push_inner = inner.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let ten = inner.add_node(Box::new(node_int(10)) as Box<_>);
    let outlet = inner.add_node(Box::new(Outlet) as Box<_>);
    inner.add_edge(push_inner, ten, Edge::from((0, 0)));
    inner.add_edge(ten, outlet, Edge::from((0, 0)));

    let mut g = Graph::new();
    let graph_node = g.add_node(Box::new(inner) as Box<dyn DebugNode>);
    let push_outer = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let twenty = g.add_node(Box::new(node_int(20)) as Box<_>);
    let add = g.add_node(Box::new(node_add()) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(graph_node, add, Edge::from((0, 0)));
    g.add_edge(push_outer, twenty, Edge::from((0, 0)));
    g.add_edge(twenty, add, Edge::from((0, 1)));
    g.add_edge(add, number, Edge::from((0, 0)));

    let combined = entrypoint::from_sources([
        entrypoint::push_source(vec![graph_node.index(), push_inner.index()], 1),
        entrypoint::push_source(vec![push_outer.index()], 1),
    ]);
    assert_pipelines_agree(&g, std::slice::from_ref(&combined), &[&combined]);
}

/// A 1-in 2-out select for nested-branch shapes: 0 -> o0(42), else o1(99).
fn node_select2() -> node::Branch {
    node::branch(
        "(if (= 0 $x) (list 0 42) (list 1 99))",
        vec![
            node::Conns::try_from([true, false]).unwrap(),
            node::Conns::try_from([false, true]).unwrap(),
        ],
    )
    .unwrap()
}

/// A divergent inner branch: each arm feeds its own outlet, so the graph
/// node branches externally and the parent stores per arm.
#[test]
fn nested_divergent_branch() {
    let mut ga = Nested::default();
    let inlet = ga.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let sel = ga.add_node(Box::new(node_select2()) as Box<_>);
    let out_a = ga.add_node(Box::new(Outlet) as Box<_>);
    let out_b = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(inlet, sel, Edge::from((0, 0)));
    ga.add_edge(sel, out_a, Edge::from((0, 0)));
    ga.add_edge(sel, out_b, Edge::from((1, 0)));

    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let graph_a = g.add_node(Box::new(ga) as Box<_>);
    let store_a = g.add_node(Box::new(node_number()) as Box<_>);
    let store_b = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, graph_a, Edge::from((0, 0)));
    g.add_edge(push_1, graph_a, Edge::from((0, 0)));
    g.add_edge(graph_a, store_a, Edge::from((0, 0)));
    g.add_edge(graph_a, store_b, Edge::from((1, 0)));
    agree_on_all_entrypoints(&g);
}

/// A reconvergent inner branch: both arms reach the single outlet, so the
/// graph node does NOT branch externally.
#[test]
fn nested_reconvergent_branch() {
    let mut ga = Nested::default();
    let inlet = ga.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let sel = ga.add_node(Box::new(node_select2()) as Box<_>);
    let out = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(inlet, sel, Edge::from((0, 0)));
    ga.add_edge(sel, out, Edge::from((0, 0)));
    ga.add_edge(sel, out, Edge::from((1, 0)));

    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let graph_a = g.add_node(Box::new(ga) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, graph_a, Edge::from((0, 0)));
    g.add_edge(push_1, graph_a, Edge::from((0, 0)));
    g.add_edge(graph_a, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// An inner branch with a dead arm: one pattern produces the outlet, the
/// other produces nothing, so downstream must not run on the dead arm.
#[test]
fn nested_dead_arm() {
    let dead_sel = node::branch(
        "(if (= 0 $x) (list 0 42) (list 1 '()))",
        vec![
            node::Conns::try_from([true]).unwrap(),
            node::Conns::try_from([false]).unwrap(),
        ],
    )
    .unwrap();
    let mut ga = Nested::default();
    let inlet = ga.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let sel = ga.add_node(Box::new(dead_sel) as Box<_>);
    let out = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(inlet, sel, Edge::from((0, 0)));
    ga.add_edge(sel, out, Edge::from((0, 0)));

    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let graph_a = g.add_node(Box::new(ga) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, graph_a, Edge::from((0, 0)));
    g.add_edge(push_1, graph_a, Edge::from((0, 0)));
    g.add_edge(graph_a, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// Branch arms passing through intermediates before their outlets, plus a
/// constant-fed outlet that fires on every pattern.
#[test]
fn nested_branch_intermediates_and_constant_outlet() {
    let mut ga = Nested::default();
    let inlet = ga.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let sel = ga.add_node(Box::new(node_select2()) as Box<_>);
    let double = ga.add_node(Box::new(node::expr("(* 2 $x)").unwrap()) as Box<_>);
    let triple = ga.add_node(Box::new(node::expr("(* 3 $x)").unwrap()) as Box<_>);
    let constant = ga.add_node(Box::new(node::expr("7").unwrap()) as Box<_>);
    let out_a = ga.add_node(Box::new(Outlet) as Box<_>);
    let out_b = ga.add_node(Box::new(Outlet) as Box<_>);
    let out_c = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(inlet, sel, Edge::from((0, 0)));
    ga.add_edge(sel, double, Edge::from((0, 0)));
    ga.add_edge(sel, triple, Edge::from((1, 0)));
    ga.add_edge(double, out_a, Edge::from((0, 0)));
    ga.add_edge(triple, out_b, Edge::from((0, 0)));
    ga.add_edge(constant, out_c, Edge::from((0, 0)));

    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let graph_a = g.add_node(Box::new(ga) as Box<_>);
    let store_a = g.add_node(Box::new(node_number()) as Box<_>);
    let store_b = g.add_node(Box::new(node_number()) as Box<_>);
    let store_c = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, graph_a, Edge::from((0, 0)));
    g.add_edge(push_1, graph_a, Edge::from((0, 0)));
    g.add_edge(graph_a, store_a, Edge::from((0, 0)));
    g.add_edge(graph_a, store_b, Edge::from((1, 0)));
    g.add_edge(graph_a, store_c, Edge::from((2, 0)));
    agree_on_all_entrypoints(&g);
}

/// An inner push reaching the outlets through a branch: the parent only
/// evaluates downstream of the outlet the taken arm produced.
#[test]
fn nested_push_through_divergent_branch() {
    let mut ga = Nested::default();
    let push = ga.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let sel = ga.add_node(Box::new(node_select2()) as Box<_>);
    let out_a = ga.add_node(Box::new(Outlet) as Box<_>);
    let out_b = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(push, sel, Edge::from((0, 0)));
    ga.add_edge(sel, out_a, Edge::from((0, 0)));
    ga.add_edge(sel, out_b, Edge::from((1, 0)));
    let inner_push = push;

    let mut g = Graph::new();
    let graph_a = g.add_node(Box::new(ga) as Box<dyn DebugNode>);
    let store_a = g.add_node(Box::new(node_number()) as Box<_>);
    let store_b = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(graph_a, store_a, Edge::from((0, 0)));
    g.add_edge(graph_a, store_b, Edge::from((1, 0)));

    let ep = entrypoint::push(vec![graph_a.index(), inner_push.index()], 1);
    assert_pipelines_agree(&g, std::slice::from_ref(&ep), &[&ep]);
}

/// A cold/hot nested graph: pushing only the cold inlet must not fire the
/// hot path (the reduced active-input-set variant).
#[test]
fn nested_cold_hot_inlets() {
    let mut ga = Nested::default();
    let inlet_hot = ga.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let inlet_cold = ga.add_node(Box::new(Inlet) as Box<_>);
    let add = ga.add_node(Box::new(
        node::expr(
            "(+ (if (Some? $?a) (Some->value $?a) 0) \
                (if (Some? $?b) (Some->value $?b) 0))",
        )
        .unwrap(),
    ) as Box<_>);
    let out = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(inlet_hot, add, Edge::from((0, 0)));
    ga.add_edge(inlet_cold, add, Edge::from((0, 1)));
    ga.add_edge(add, out, Edge::from((0, 0)));

    let mut g = Graph::new();
    let push_hot = g.add_node(Box::new(node_int(5).with_push_eval()) as Box<dyn DebugNode>);
    let push_cold = g.add_node(Box::new(node_int(3).with_push_eval()) as Box<_>);
    let graph_a = g.add_node(Box::new(ga) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_hot, graph_a, Edge::from((0, 0)));
    g.add_edge(push_cold, graph_a, Edge::from((0, 1)));
    g.add_edge(graph_a, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// A three-arm inner branch with one outlet per arm.
#[test]
fn nested_three_arm_branch() {
    let sel3 = node::branch(
        "(if (= 0 $x) (list 0 6) (if (= 1 $x) (list 1 7) (list 2 8)))",
        vec![
            node::Conns::try_from([true, false, false]).unwrap(),
            node::Conns::try_from([false, true, false]).unwrap(),
            node::Conns::try_from([false, false, true]).unwrap(),
        ],
    )
    .unwrap();
    let mut ga = Nested::default();
    let inlet = ga.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let sel = ga.add_node(Box::new(sel3) as Box<_>);
    let outs: Vec<_> = (0..3)
        .map(|_| ga.add_node(Box::new(Outlet) as Box<_>))
        .collect();
    ga.add_edge(inlet, sel, Edge::from((0, 0)));
    for (o, &out) in outs.iter().enumerate() {
        ga.add_edge(sel, out, Edge::from((o as u16, 0)));
    }

    let mut g = Graph::new();
    let pushes: Vec<_> = (0..3)
        .map(|i| g.add_node(Box::new(node_int(i).with_push_eval()) as Box<dyn DebugNode>))
        .collect();
    let graph_a = g.add_node(Box::new(ga) as Box<_>);
    for &p in &pushes {
        g.add_edge(p, graph_a, Edge::from((0, 0)));
    }
    for o in 0..3u16 {
        let store = g.add_node(Box::new(node_number()) as Box<_>);
        g.add_edge(graph_a, store, Edge::from((o, 0)));
    }
    agree_on_all_entrypoints(&g);
}

/// A branching graph nested inside another branching graph: the outer graph
/// node's external branches compose from two levels of dispatch.
#[test]
fn nested_branch_two_levels() {
    // Innermost: divergent select.
    let mut inner = Nested::default();
    let in_inlet = inner.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let in_sel = inner.add_node(Box::new(node_select2()) as Box<_>);
    let in_out_a = inner.add_node(Box::new(Outlet) as Box<_>);
    let in_out_b = inner.add_node(Box::new(Outlet) as Box<_>);
    inner.add_edge(in_inlet, in_sel, Edge::from((0, 0)));
    inner.add_edge(in_sel, in_out_a, Edge::from((0, 0)));
    inner.add_edge(in_sel, in_out_b, Edge::from((1, 0)));

    // Middle: passes through the inner branching graph to its own outlets.
    let mut mid = Nested::default();
    let mid_inlet = mid.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let inner_ix = mid.add_node(Box::new(inner) as Box<_>);
    let mid_out_a = mid.add_node(Box::new(Outlet) as Box<_>);
    let mid_out_b = mid.add_node(Box::new(Outlet) as Box<_>);
    mid.add_edge(mid_inlet, inner_ix, Edge::from((0, 0)));
    mid.add_edge(inner_ix, mid_out_a, Edge::from((0, 0)));
    mid.add_edge(inner_ix, mid_out_b, Edge::from((1, 0)));

    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let mid_ix = g.add_node(Box::new(mid) as Box<_>);
    let store_a = g.add_node(Box::new(node_number()) as Box<_>);
    let store_b = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, mid_ix, Edge::from((0, 0)));
    g.add_edge(push_1, mid_ix, Edge::from((0, 0)));
    g.add_edge(mid_ix, store_a, Edge::from((0, 0)));
    g.add_edge(mid_ix, store_b, Edge::from((1, 0)));
    agree_on_all_entrypoints(&g);
}

/// A stateful node inside an inner branch arm: it must run only on its arm.
#[test]
fn nested_branch_stateful_arm() {
    let mut ga = Nested::default();
    let inlet = ga.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let sel = ga.add_node(Box::new(node_select2()) as Box<_>);
    let store_arm = ga.add_node(Box::new(node_number()) as Box<_>);
    let out = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(inlet, sel, Edge::from((0, 0)));
    // Arm 0 passes through the stateful store; arm 1 goes straight out.
    ga.add_edge(sel, store_arm, Edge::from((0, 0)));
    ga.add_edge(store_arm, out, Edge::from((0, 0)));
    ga.add_edge(sel, out, Edge::from((1, 0)));

    let mut g = Graph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let graph_a = g.add_node(Box::new(ga) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, graph_a, Edge::from((0, 0)));
    g.add_edge(push_1, graph_a, Edge::from((0, 0)));
    g.add_edge(graph_a, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

/// Inlet/outlet ids interleaved with other nodes: input i must map to the
/// i-th inlet in id order regardless of insertion order.
#[test]
fn nested_non_sequential_inlets() {
    let mut ga = Nested::default();
    let mul = ga.add_node(Box::new(node::expr("(* $l $r)").unwrap()) as Box<dyn DebugNode>);
    let inlet_a = ga.add_node(Box::new(Inlet) as Box<_>);
    let outlet = ga.add_node(Box::new(Outlet) as Box<_>);
    let inlet_b = ga.add_node(Box::new(Inlet) as Box<_>);
    ga.add_edge(inlet_a, mul, Edge::from((0, 0)));
    ga.add_edge(inlet_b, mul, Edge::from((0, 1)));
    ga.add_edge(mul, outlet, Edge::from((0, 0)));

    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let six = g.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let graph_a = g.add_node(Box::new(ga) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, six, Edge::from((0, 0)));
    g.add_edge(push, seven, Edge::from((0, 0)));
    g.add_edge(six, graph_a, Edge::from((0, 0)));
    g.add_edge(seven, graph_a, Edge::from((0, 1)));
    g.add_edge(graph_a, number, Edge::from((0, 0)));
    agree_on_all_entrypoints(&g);
}

// ===========================================================================
// Delay-cell feedback (IR pipeline only - the flow pipeline does not support
// cyclic graphs, so these assert expected behavior directly).
// ===========================================================================

/// Compile through the IR pipeline, run, and call each entrypoint in order.
fn run_v2(g: &Graph, eps: &[Entrypoint], calls: &[&Entrypoint]) -> Engine {
    let m2 = gantz_core::compile::module(&no_lookup, g, eps, &Default::default())
        .expect("IR pipeline failed");
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, g, &[], &mut vm);
    for f in &m2 {
        vm.run(f.to_pretty(100)).unwrap();
    }
    for ep in calls {
        vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
            .unwrap();
    }
    vm
}

/// The classic feedback accumulator: `add` sums its input with the delayed
/// previous sum. The cycle is legal because it passes through the delay; the
/// value crosses *between* evaluations.
///
///   push(5) -> add <----- delay
///               |  \------^
///               v
///             number
#[test]
fn delay_feedback_accumulator() {
    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_int(5).with_push_eval()) as Box<dyn DebugNode>);
    let add = g.add_node(Box::new(node::expr("(+ $x (if (number? $d) $d 0))").unwrap()) as Box<_>);
    let delay = g.add_node(Box::new(node::Delay) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, add, Edge::from((0, 0)));
    g.add_edge(delay, add, Edge::from((0, 1)));
    g.add_edge(add, delay, Edge::from((0, 0))); // the feedback edge
    g.add_edge(add, number, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let ep = entrypoint::push(vec![push.index()], 1);
    let vm = run_v2(&g, &eps, &[&ep, &ep, &ep]);

    // 5, then 5+5, then 5+10.
    let val = node::state::extract::<i32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 15);
}

/// The same accumulator inside a nested graph: the delay's state lives in
/// the nested level's state map and the feedback survives across pushes.
#[test]
fn delay_feedback_in_nested_graph() {
    let mut ga = Nested::default();
    let inlet = ga.add_node(Box::new(Inlet) as Box<dyn DebugNode>);
    let add = ga.add_node(Box::new(node::expr("(+ $x (if (number? $d) $d 0))").unwrap()) as Box<_>);
    let delay = ga.add_node(Box::new(node::Delay) as Box<_>);
    let outlet = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(inlet, add, Edge::from((0, 0)));
    ga.add_edge(delay, add, Edge::from((0, 1)));
    ga.add_edge(add, delay, Edge::from((0, 0)));
    ga.add_edge(add, outlet, Edge::from((0, 0)));

    let mut g = Graph::new();
    let push = g.add_node(Box::new(node_int(5).with_push_eval()) as Box<dyn DebugNode>);
    let graph_a = g.add_node(Box::new(ga) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, graph_a, Edge::from((0, 0)));
    g.add_edge(graph_a, number, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let ep = entrypoint::push(vec![push.index()], 1);
    let vm = run_v2(&g, &eps, &[&ep, &ep, &ep]);

    let val = node::state::extract::<i32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 15);
}

/// A delay read without any write this evaluation: a separate entrypoint
/// stores into the delay; the reader sees the previous stored value only.
#[test]
fn delay_read_and_write_in_separate_entrypoints() {
    let mut g = Graph::new();
    // Writer chain: push_w(7) -> delay.
    let push_w = g.add_node(Box::new(node_int(7).with_push_eval()) as Box<dyn DebugNode>);
    let delay = g.add_node(Box::new(node::Delay) as Box<_>);
    g.add_edge(push_w, delay, Edge::from((0, 0)));
    // Reader chain: push_r -> get(reads delay) -> number.
    let push_r = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let get = g
        .add_node(Box::new(node::expr("(begin $bang (if (number? $d) $d -1))").unwrap()) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_r, get, Edge::from((0, 0)));
    g.add_edge(delay, get, Edge::from((0, 1)));
    g.add_edge(get, number, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let ep_w = entrypoint::push(vec![push_w.index()], 1);
    let ep_r = entrypoint::push(vec![push_r.index()], 1);

    // Read before any write: -1. Write 7, read again: 7.
    let vm = run_v2(&g, &eps, &[&ep_r]);
    let val = node::state::extract::<i32>(&vm, &[number.index()])
        .unwrap()
        .unwrap();
    assert_eq!(val, -1, "read before any write sees the initial value");

    let vm = run_v2(&g, &eps, &[&ep_w, &ep_r]);
    let val = node::state::extract::<i32>(&vm, &[number.index()])
        .unwrap()
        .unwrap();
    assert_eq!(val, 7, "read after a write sees the stored value");
}

/// An inner push whose value circulates through a delay cycle AND reaches
/// the outlet: push-through-outlet bridging must work for cyclic interiors.
/// (The pre-cutover flow analysis used a cycle-blind topological walk and
/// would have missed the outlet reach here.)
#[test]
fn delay_feedback_push_through_outlet() {
    let mut ga = Nested::default();
    let push = ga.add_node(Box::new(node_int(5).with_push_eval()) as Box<dyn DebugNode>);
    let add = ga.add_node(Box::new(node::expr("(+ $x (if (number? $d) $d 0))").unwrap()) as Box<_>);
    let delay = ga.add_node(Box::new(node::Delay) as Box<_>);
    let outlet = ga.add_node(Box::new(Outlet) as Box<_>);
    ga.add_edge(push, add, Edge::from((0, 0)));
    ga.add_edge(delay, add, Edge::from((0, 1)));
    ga.add_edge(add, delay, Edge::from((0, 0)));
    ga.add_edge(add, outlet, Edge::from((0, 0)));
    let inner_push = push;

    let mut g = Graph::new();
    let graph_a = g.add_node(Box::new(ga) as Box<dyn DebugNode>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(graph_a, number, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let ep = entrypoint::push(vec![graph_a.index(), inner_push.index()], 1);
    let vm = run_v2(&g, &eps, &[&ep, &ep, &ep]);

    // The accumulating value propagates through the outlet each push.
    let val = node::state::extract::<i32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 15);
}
