// Tests for the graph module.

use gantz_core::compile::{entry_fn_name, entrypoint, push_pull_entrypoints, push_source};
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

// Stores the received number in state and returns it.
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

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
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
fn test_graph_push_eval() {
    let mut g = petgraph::graph::DiGraph::new();

    let push = node_push();
    let one = node_int(1);
    let add = node_add();
    let two = node_int(2);
    let assert_eq = node_assert_eq();

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

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();
    // Function per node alongside the single push eval function.
    assert_eq!(module.len(), g.node_count() + 1);

    let mut vm = Engine::new_base();

    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in module {
        vm.run(format!("{f}")).unwrap();
    }
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}

// A `0`-var `expr` exposes a single trigger input whose value is ignored. A push
// into that input still forces the expression to evaluate. Here two constant
// exprs fire through their trigger inputs and are compared. This proves the
// connected but unreferenced trigger param compiles and the constants
// propagate.
#[test]
fn test_expr_trigger_input() {
    let mut g = petgraph::graph::DiGraph::new();

    // `left` and `right` have no `$vars`, so each has a single trigger input.
    let push = node_push();
    let left = node::expr("(+ 1 2)").unwrap();
    let right = node::expr("3").unwrap();
    let assert_eq = node_assert_eq();

    let push = g.add_node(Box::new(push) as Box<dyn DebugNode>);
    let left = g.add_node(Box::new(left) as Box<_>);
    let right = g.add_node(Box::new(right) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);

    // The push fires both constants via their trigger inputs.
    g.add_edge(push, left, Edge::from((0, 0)));
    g.add_edge(push, right, Edge::from((0, 0)));
    g.add_edge(left, assert_eq, Edge::from((0, 0)));
    g.add_edge(right, assert_eq, Edge::from((0, 1)));

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
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
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

    let one = node_int(1);
    let add = node_add();
    let two = node_int(2);
    let assert_eq = node_assert_eq().with_pull_eval();

    let one = g.add_node(Box::new(one) as Box<dyn DebugNode>);
    let add = g.add_node(Box::new(add) as Box<_>);
    let two = g.add_node(Box::new(two) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);
    g.add_edge(one, add, Edge::from((0, 0)));
    g.add_edge(one, add, Edge::from((0, 1)));
    g.add_edge(add, assert_eq, Edge::from((0, 0)));
    g.add_edge(two, assert_eq, Edge::from((0, 1)));

    let ctx = node::MetaCtx::new(&no_lookup);

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();

    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for expr in module {
        vm.run(expr.to_pretty(100)).unwrap();
    }

    let ep = entrypoint::pull(vec![assert_eq.index()], g[assert_eq].n_inputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}

// A simple test graph that checks conditional runtime evaluation.
//
//    ---------- ----------
//    | push_0 | | push_1 |
//    -+-------- -+--------
//     |          |
//     |-----------
//     |
//    -+------------
//    | select_0_1 | // pushes left on 0, right on 1
//    -+----------+-
//     |          |
//    -+-----    -+-------
//    | six |    | seven |
//    -+-----    -+-------
//     |          |
//     |-----------
//     |
//    -+--------
//    | number |
//    ----------
#[test]
fn test_graph_push_cond_eval() {
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
            let expr = format!(
                r#"
                (if (equal? 0 {x})
                  (list 0 '())  ; 0 index for left branch, '() for empty value
                  (list 1 '())) ; 1 index for right branch, '() for empty value
            "#
            );
            node::parse_expr(&expr)
        }
    }

    let mut g = petgraph::graph::DiGraph::new();

    let push_0 = node_int(0).with_push_eval();
    let push_1 = node_int(1).with_push_eval();
    let select = Select;
    let six = node_int(6);
    let seven = node_int(7);
    let number = node_number();

    let push_0 = g.add_node(Box::new(push_0) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(push_1) as Box<_>);
    let select = g.add_node(Box::new(select) as Box<_>);
    let six = g.add_node(Box::new(six) as Box<_>);
    let seven = g.add_node(Box::new(seven) as Box<_>);
    let number = g.add_node(Box::new(number) as Box<_>);
    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(push_1, select, Edge::from((0, 0)));
    g.add_edge(select, six, Edge::from((0, 0)));
    g.add_edge(select, seven, Edge::from((1, 0)));
    g.add_edge(six, number, Edge::from((0, 0)));
    g.add_edge(seven, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();
    // Function per node alongside the two push eval functions.
    assert_eq!(module.len(), g.node_count() + 2);

    let mut vm = Engine::new_base();

    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was `None`");
    assert_eq!(number_state, 6);

    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was `None`");
    assert_eq!(number_state, 7);
}

// The conditional eval codegen must not duplicate the join node's function
// call in the generated Scheme. The "number" node sits after a
// branch-and-join and must appear exactly once in each entry function body.
#[test]
fn test_graph_cond_eval_no_join_duplication() {
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
            node::parse_expr(&format!("(if (equal? 0 {x}) (list 0 '()) (list 1 '()))"))
        }
    }

    let mut g = petgraph::graph::DiGraph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let _push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<dyn DebugNode>);
    let select = g.add_node(Box::new(Select) as Box<_>);
    let six = g.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(select, six, Edge::from((0, 0)));
    g.add_edge(select, seven, Edge::from((1, 0)));
    g.add_edge(six, number, Edge::from((0, 0)));
    g.add_edge(seven, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);

    // Build the entrypoint for push_0 only.
    let ep =
        gantz_core::compile::entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    let module = gantz_core::compile::module(&no_lookup, &g, &[ep], &Default::default()).unwrap();

    // The number node's function name contains its index. Count its
    // occurrences across all generated expressions.
    let module_str = module
        .iter()
        .map(|e| format!("{e}"))
        .collect::<Vec<_>>()
        .join("\n");
    let number_fn_prefix = format!("node-fn-{}", number.index());
    let count = module_str.matches(&number_fn_prefix).count();
    // Once in the node function definition and once in the entry fn body
    // call. A duplicated join would put two or more calls in the entry fn
    // body.
    assert!(
        count <= 2,
        "number node fn '{}' appears {} times - join is duplicated!\nGenerated module:\n{}",
        number_fn_prefix,
        count,
        module_str,
    );
}

// One branch goes directly to the join node with no intermediate nodes. The
// other branch goes through an intermediate node first.
//
//    ---------- ----------
//    | push_0 | | push_1 |
//    -+-------- -+--------
//     |          |
//     |-----------
//     |
//    -+--------
//    | select | // br0 passes 6 directly, br1 passes through
//    -+-----+-+
//     |       |
//     |      -+-------
//     |      | seven |
//     |      -+-------
//     |       |
//    -+-------+-
//    | number  |
//    -----------
//
// On branch 0, select's output 0 goes directly to number, the join. On
// branch 1, select's output 1 goes through seven, then to number.
#[test]
fn test_graph_branch_target_is_join() {
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
            // Branch 0 passes 6 as the value. Branch 1 passes the input through.
            node::parse_expr(&format!("(if (equal? 0 {x}) (list 0 6) (list 1 {x}))"))
        }
    }

    let mut g = petgraph::graph::DiGraph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<dyn DebugNode>);
    let select = g.add_node(Box::new(Select) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(push_1, select, Edge::from((0, 0)));
    g.add_edge(select, number, Edge::from((0, 0))); // br0: direct to join
    g.add_edge(select, seven, Edge::from((1, 0))); // br1: through seven
    g.add_edge(seven, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 sends select left and passes 6 directly to number.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("state was None");
    assert_eq!(state, 6);

    // Push 1 sends select right, through seven to number.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("state was None");
    assert_eq!(state, 7);

    // The generated code must not duplicate the join.
    let module_str = module
        .iter()
        .map(|e| format!("{e}"))
        .collect::<Vec<_>>()
        .join("\n");
    let number_fn_prefix = format!("node-fn-{}", number.index());
    let count = module_str.matches(&number_fn_prefix).count();
    assert!(
        count <= 3, // 1 fn def + up to 1 call per entry fn
        "number node fn appears {} times - join duplicated!\n{}",
        count,
        module_str,
    );
}

/// Both branch outputs feed the same target input. The join must bind the
/// correct output variable for each arm.
///
/// ```text
///   push_0 --\          /-- output 0 --\
///              select --<               number
///   push_1 --/          \-- output 1 --/
/// ```
#[test]
fn test_graph_branch_both_outputs_same_target() {
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
            node::parse_expr(&format!("(if (equal? 0 {x}) (list 0 42) (list 1 99))"))
        }
    }

    let mut g = petgraph::graph::DiGraph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let select = g.add_node(Box::new(Select) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);

    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(push_1, select, Edge::from((0, 0)));
    // Both branch outputs go to number's single input.
    g.add_edge(select, number, Edge::from((0, 0)));
    g.add_edge(select, number, Edge::from((1, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 takes arm 0, so number receives 42.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("number was None");
    assert_eq!(val, 42);

    // Push 1 takes arm 1, so number receives 99.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("number was None");
    assert_eq!(val, 99);
}

// A nested diamond. The outer branch contains an inner branch. Both have
// distinct reconvergence points.
//
//    ----------   ----------
//    | push_0 |   | push_1 |
//    -+--------   -+--------
//     |            |
//     |------ ------
//     |
//    -+--------------
//    | select_outer | // br0=left, br1=right
//    -+-------+-----+
//     |              |
//    -+------------  |
//    |select_inner|  |
//    -+-----+-----  |
//     |     |        |
//    -+-  --+--     -+-------
//    |6|  |  7|     | eight |
//    -+-  ----      -+-------
//     |     |        |
//    -+-----+-       |
//    |inner_res|     |
//    -+--------      |
//     |              |
//    -+--------------+
//    |  outer_result  |
//    ------------------
//
// Push 0 takes the outer left arm. select_inner receives '(), which is not
// 0, so it takes the inner right arm. seven feeds inner_result and
// outer_result with 7. Push 1 takes the outer right arm. eight feeds
// outer_result with 8.
#[test]
fn test_graph_nested_diamond() {
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
            node::parse_expr(&format!("(if (equal? 0 {x}) (list 0 '()) (list 1 '()))"))
        }
    }

    let mut g = petgraph::graph::DiGraph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<dyn DebugNode>);
    let select_outer = g.add_node(Box::new(Select) as Box<_>);
    let select_inner = g.add_node(Box::new(Select) as Box<_>);
    let six = g.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let inner_result = g.add_node(Box::new(node_number()) as Box<_>);
    let eight = g.add_node(Box::new(node_int(8)) as Box<_>);
    let outer_result = g.add_node(Box::new(node_number()) as Box<_>);
    // Outer structure.
    g.add_edge(push_0, select_outer, Edge::from((0, 0)));
    g.add_edge(push_1, select_outer, Edge::from((0, 0)));
    g.add_edge(select_outer, select_inner, Edge::from((0, 0)));
    g.add_edge(select_outer, eight, Edge::from((1, 0)));
    // Inner diamond.
    g.add_edge(select_inner, six, Edge::from((0, 0)));
    g.add_edge(select_inner, seven, Edge::from((1, 0)));
    g.add_edge(six, inner_result, Edge::from((0, 0)));
    g.add_edge(seven, inner_result, Edge::from((0, 0)));
    // Outer reconvergence.
    g.add_edge(inner_result, outer_result, Edge::from((0, 0)));
    g.add_edge(eight, outer_result, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 takes outer left then inner right, since '() is not 0. seven
    // gives 7, so inner_result and outer_result store 7.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let inner = node::state::extract::<u32>(&vm, &[inner_result.index()])
        .expect("failed to extract")
        .expect("inner_result state was None");
    assert_eq!(inner, 7);
    let outer = node::state::extract::<u32>(&vm, &[outer_result.index()])
        .expect("failed to extract")
        .expect("outer_result state was None");
    assert_eq!(outer, 7);

    // Push 1 takes outer right. eight gives 8, so outer_result stores 8.
    // inner_result is not evaluated and stays at 7.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let inner = node::state::extract::<u32>(&vm, &[inner_result.index()])
        .expect("failed to extract")
        .expect("inner_result state was None");
    assert_eq!(inner, 7); // unchanged
    let outer = node::state::extract::<u32>(&vm, &[outer_result.index()])
        .expect("failed to extract")
        .expect("outer_result state was None");
    assert_eq!(outer, 8);

    // Neither join node's function call may be duplicated.
    let module_str = module
        .iter()
        .map(|e| format!("{e}"))
        .collect::<Vec<_>>()
        .join("\n");
    for (name, ix) in [
        ("inner_result", inner_result.index()),
        ("outer_result", outer_result.index()),
    ] {
        let prefix = format!("node-fn-{ix}");
        let count = module_str.matches(&prefix).count();
        assert!(
            count <= 3, // 1 fn def + up to 1 call per entry fn
            "{name} fn '{prefix}' appears {count} times - join duplicated!\n{module_str}",
        );
    }
}

// A lattice. Both outer branch targets share the same inner branching
// structure. The post-dominator approach finds number as the reconvergence.
//
//      ----------       ----------
//      | push_0 |       | push_1 |
//      ----+-----       ----+-----
//          |                |
//          |-------  -------|
//                 |  |
//            -----+--+-------
//            | select_outer  |  br0=left, br1=right
//            ---+--------+----
//               |        |
//         ------+--   ---+-------
//         | sel_L |   |  sel_R  |  both branch to six and seven
//         --+---+--   ---+---+---
//           |   |        |   |
//           |   +-----+--+   |
//           +-----+   |  +---+
//                 |   |
//            -----+---+---
//            |   six      |
//            ------+-------
//                  |
//            ------+-------
//            |   seven    |
//            ------+-------
//                  |
//            ------+-------
//            |   number   |  join - all paths converge here
//            --------------
//
// Push 0 takes outer left to sel_L. sel_L takes right since '() is not 0, so
// seven gives number 7. Push 1 takes outer right to sel_R. sel_R takes right
// as well, so number gets 7 again.
#[test]
fn test_graph_lattice_reconvergence() {
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
            node::parse_expr(&format!("(if (equal? 0 {x}) (list 0 '()) (list 1 '()))"))
        }
    }

    let mut g = petgraph::graph::DiGraph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<dyn DebugNode>);
    let select_outer = g.add_node(Box::new(Select) as Box<_>);
    let sel_l = g.add_node(Box::new(Select) as Box<_>);
    let sel_r = g.add_node(Box::new(Select) as Box<_>);
    let six = g.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = g.add_node(Box::new(node_int(7)) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);

    // Outer structure.
    g.add_edge(push_0, select_outer, Edge::from((0, 0)));
    g.add_edge(push_1, select_outer, Edge::from((0, 0)));
    g.add_edge(select_outer, sel_l, Edge::from((0, 0)));
    g.add_edge(select_outer, sel_r, Edge::from((1, 0)));

    // Both inner selects branch to the same six/seven nodes.
    g.add_edge(sel_l, six, Edge::from((0, 0)));
    g.add_edge(sel_l, seven, Edge::from((1, 0)));
    g.add_edge(sel_r, six, Edge::from((0, 0)));
    g.add_edge(sel_r, seven, Edge::from((1, 0)));

    // Both converge at number.
    g.add_edge(six, number, Edge::from((0, 0)));
    g.add_edge(seven, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 takes outer left. sel_L defaults right, so seven gives number 7.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("state was None");
    assert_eq!(state, 7);

    // Push 1 takes outer right. sel_R defaults right, so seven gives number 7.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("state was None");
    assert_eq!(state, 7);

    // number's fn call must not be duplicated per outer branch.
    let module_str = module
        .iter()
        .map(|e| format!("{e}"))
        .collect::<Vec<_>>()
        .join("\n");
    let number_fn_prefix = format!("node-fn-{}", number.index());
    let count = module_str.matches(&number_fn_prefix).count();
    assert!(
        count <= 3, // 1 fn def + up to 1 call per entry fn
        "number fn '{number_fn_prefix}' appears {count} times - join duplicated!\n{module_str}",
    );
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

    let one = node_int(1);
    let add = node_add();
    let assert_eq = node_assert_eq().with_pull_eval();

    let one = g.add_node(Box::new(one) as Box<dyn DebugNode>);
    let add = g.add_node(Box::new(add) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);
    g.add_edge(one, add, Edge::from((0, 0)));
    g.add_edge(one, add, Edge::from((0, 1)));
    g.add_edge(add, assert_eq, Edge::from((0, 0)));
    g.add_edge(one, assert_eq, Edge::from((0, 1)));

    let ctx = node::MetaCtx::new(&no_lookup);

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();

    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for expr in module {
        vm.run(expr.to_pretty(100)).unwrap();
    }
    let ep = entrypoint::pull(vec![assert_eq.index()], g[assert_eq].n_inputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}

// Push evaluation with a subset of outputs enabled.
#[test]
#[ignore = "Originally attempted to get this working with push/pull eval \
    configurations, but realising it would be cleaner to get general conditional \
    eval working first."]
fn test_graph_push_eval_subset() {
    let mut g = petgraph::graph::DiGraph::new();

    // Source node with two outputs, one for each value.
    #[derive(Debug)]
    struct Src(u32, u32);

    impl Node for Src {
        fn push_eval(&self, _ctx: node::MetaCtx) -> Vec<node::EvalConf> {
            vec![
                // Push only the first output.
                node::EvalConf::Set([true, false].try_into().unwrap()),
                // Push only the second output.
                node::EvalConf::Set([false, true].try_into().unwrap()),
                // Push both outputs.
                node::EvalConf::Set([true, true].try_into().unwrap()),
            ]
        }

        fn n_outputs(&self, _ctx: node::MetaCtx) -> usize {
            2
        }

        fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
            let Src(a, b) = *self;
            let outputs = ctx.outputs();
            let expr = match (outputs.get(0).unwrap(), outputs.get(1).unwrap()) {
                // Only return left if only left is connected.
                (true, false) => format!("(begin {a})"),
                // Only return right if only right is connected.
                (false, true) => format!("(begin {b})"),
                // Otherwise return both in a list.
                _ => format!("(list {a} {b})"),
            };
            node::parse_expr(&expr)
        }
    }

    let source = Src(6, 7);
    let store_a = node::expr("(begin (set! state $x) state)").unwrap();
    let store_b = node::expr("(begin (set! state $x) state)").unwrap();

    let source = g.add_node(Box::new(source) as Box<dyn DebugNode>);
    let store_a = g.add_node(Box::new(store_a) as Box<_>);
    let store_b = g.add_node(Box::new(store_b) as Box<_>);

    g.add_edge(source, store_a, Edge::from((0, 0)));
    g.add_edge(source, store_b, Edge::from((1, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();

    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    let ep = &eps[0]; // first push eval conf: only first output
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    let store_a_val = node::state::extract::<i32>(&vm, &[store_a.index()]).unwrap();
    let store_b_val = node::state::extract::<i32>(&vm, &[store_b.index()]).unwrap();

    // The first output was enabled for push, so its state is 6.
    assert_eq!(store_a_val, Some(6));

    // The second output was not enabled for push, so it was never evaluated
    // and its state is None.
    assert_eq!(store_b_val, None);
}

// A multi-source entrypoint combines two push nodes into one eval fn.
//
//    ----------   ----------
//    | push_a |   | push_b |
//    -+--------   -+--------
//     |            |
//    -+--------   -+--------
//    | 42     |   | 7      |
//    -+--------   -+--------
//     |            |
//    -+--------   -+--------
//    | num_a  |   | num_b  |
//    ----------   ----------
//
// Two independent chains. A combined entrypoint evaluates both in one call.
#[test]
fn test_graph_multi_source_push() {
    let mut g = petgraph::graph::DiGraph::new();

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
        push_source(vec![push_a.index()], g[push_a].n_outputs(ctx) as u8),
        push_source(vec![push_b.index()], g[push_b].n_outputs(ctx) as u8),
    ]);
    let module =
        gantz_core::compile::module(&no_lookup, &g, &[combined.clone()], &Default::default())
            .unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Calling the combined entrypoint evaluates both chains.
    let fn_name = entry_fn_name(&combined.id());
    vm.call_function_by_name_with_args(&fn_name, vec![])
        .unwrap();

    let a = node::state::extract::<u32>(&vm, &[num_a.index()])
        .expect("failed to extract num_a state")
        .expect("num_a state was None");
    let b = node::state::extract::<u32>(&vm, &[num_b.index()])
        .expect("failed to extract num_b state")
        .expect("num_b state was None");
    assert_eq!(a, 42);
    assert_eq!(b, 7);
}

// `entrypoint::push` must produce the same EntrypointId as
// `push_pull_entrypoints` for the same node.
#[test]
fn test_entrypoint_naming_consistency() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int = g.add_node(Box::new(node_int(1)) as Box<_>);
    g.add_edge(push, int, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let manual = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);

    // The default planner produces a singleton push entrypoint for the push
    // node. Its id must match a manually constructed one.
    let default_ep = eps
        .iter()
        .find(|ep| ep.0.iter().any(|s| s.path == vec![push.index()]))
        .expect("push_pull_entrypoints should contain push node");
    assert_eq!(default_ep.id(), manual.id());
    assert_eq!(entry_fn_name(&default_ep.id()), entry_fn_name(&manual.id()));
}

// A 2-output expr node returns `(list 6 7)`. Each output is wired to a
// separate stateful store node. After push evaluation, each store holds the
// corresponding value.
//
//    --------
//    | push |
//    --------
//       |
//    ----------
//    | pair   |  outputs=2, expr: (begin $push (list 6 7))
//    ----------
//     |      |
//     o0     o1
//     |      |
// ---------  ---------
// | num_a |  | num_b |
// ---------  ---------
#[test]
fn test_graph_multi_output_expr() {
    let mut g = petgraph::graph::DiGraph::new();

    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let pair = node::expr("(begin $push (list 6 7))")
        .unwrap()
        .with_outputs(2);
    let pair = g.add_node(Box::new(pair) as Box<_>);
    let num_a = g.add_node(Box::new(node_number()) as Box<_>);
    let num_b = g.add_node(Box::new(node_number()) as Box<_>);

    g.add_edge(push, pair, Edge::from((0, 0)));
    g.add_edge(pair, num_a, Edge::from((0, 0))); // output 0 -> num_a
    g.add_edge(pair, num_b, Edge::from((1, 0))); // output 1 -> num_b

    let ctx = node::MetaCtx::new(&no_lookup);

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    let a = node::state::extract::<u32>(&vm, &[num_a.index()])
        .expect("failed to extract num_a state")
        .expect("num_a state was None");
    let b = node::state::extract::<u32>(&vm, &[num_b.index()])
        .expect("failed to extract num_b state")
        .expect("num_b state was None");
    assert_eq!(a, 6);
    assert_eq!(b, 7);
}

// Nodes with 0 outputs, such as side-effect-only Log nodes, must work when
// multiple appear in the same evaluation path.
//
//    ----------
//    | push   |  (push_eval, 1 output)
//    -+--------
//     |\
//     | \
//    -+------  -+------
//    |effect1|  |effect2|  (each: 1 input, 0 outputs)
//    --------  ---------
//
// Both effect nodes end up in the same basic block. The first one is not
// last in the block, so the emitter destructures its outputs. With 0 outputs
// this must be a no-op. It must not emit an invalid `(define-values ()
// node-X)` that references an undefined binding.
#[test]
fn test_graph_zero_output_leaf_nodes() {
    /// A node with 1 input and 0 outputs. It is a pure side effect.
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

    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let effect1 = g.add_node(Box::new(Effect) as Box<dyn DebugNode>);
    let effect2 = g.add_node(Box::new(Effect) as Box<dyn DebugNode>);
    g.add_edge(push, effect1, Edge::from((0, 0)));
    g.add_edge(push, effect2, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // The push entrypoint must not crash.
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}

/// The `Branch` node type in a graph with push evaluation.
///
/// ```text
///   push_0 (emits 0) ---\
///                         branch --- out0 -> six -> number
///   push_1 (emits 1) ---/       \-- out1 -> seven -> number
/// ```
///
/// When push_0 fires with input 0, branch selects index 0. six feeds number,
/// which stores 6. When push_1 fires with input 1, branch selects index 1.
/// seven feeds number, which stores 7.
#[test]
fn test_graph_branch_node() {
    let branch = node::Branch::new(
        "(if (equal? 0 $x) (list 0 '()) (list 1 '()))",
        vec![
            node::Conns::try_from([true, false]).unwrap(),
            node::Conns::try_from([false, true]).unwrap(),
        ],
    )
    .unwrap();

    let mut g = petgraph::graph::DiGraph::new();

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

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 makes branch take index 0, so number stores 6.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 6);

    // Push 1 makes branch take index 1, so number stores 7.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 7);
}

// Multiple unconditional edges to the same input produce a list.
//
//    --------
//    | push |
//    -+------
//     |
//     |----------
//     |         |
//    -+------- -+------
//    | three | | four |
//    -+------- -+------
//     |         |
//     |---------- (both connect to sum.i0)
//     |
//    -+-------
//    |  sum  |  expr: (apply + $x) - sums all list elements
//    -+-------
//     |
//    -+-------
//    | store |
//    ---------
//
// Both `three` and `four` connect unconditionally to `sum`'s single input.
// With multi-edge list bindings, `sum` receives `(list 3 4)` and
// `(apply + (list 3 4))` = 7.
#[test]
fn test_graph_multi_edge_input_list() {
    let mut g = petgraph::graph::DiGraph::new();

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

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    // The generated code must contain a list binding.
    let module_str = module
        .iter()
        .map(|e| format!("{e}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        module_str.contains("(list"),
        "expected a (list ...) binding for multi-edge input\n{module_str}",
    );

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    let val = node::state::extract::<u32>(&vm, &[store.index()])
        .expect("failed to extract")
        .expect("was None");
    // 3 + 4 = 7, not just 4 from last-write-wins.
    assert_eq!(val, 7);
}

/// Branching to independent terminal nodes with no reconvergence. There is
/// no join and no shared variables.
///
/// ```text
///   push_0 (emits 0) ---\              /--- store_a (stateful leaf)
///                         select -----<
///   push_1 (emits 1) ---/              \--- store_b (stateful leaf)
/// ```
#[test]
fn test_graph_branch_divergent_terminal() {
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
            node::parse_expr(&format!("(if (equal? 0 {x}) (list 0 42) (list 1 99))"))
        }
    }

    let mut g = petgraph::graph::DiGraph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let select = g.add_node(Box::new(Select) as Box<_>);
    let store_a = g.add_node(Box::new(node_number()) as Box<_>);
    let store_b = g.add_node(Box::new(node_number()) as Box<_>);

    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(push_1, select, Edge::from((0, 0)));
    g.add_edge(select, store_a, Edge::from((0, 0)));
    g.add_edge(select, store_b, Edge::from((1, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 takes arm 0, so store_a receives 42.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val_a = node::state::extract::<u32>(&vm, &[store_a.index()])
        .expect("failed to extract")
        .expect("store_a was None");
    assert_eq!(val_a, 42);
    // store_b is untouched and still holds its initial value.
    let val_b = node::state::extract::<u32>(&vm, &[store_b.index()])
        .ok()
        .flatten();
    assert!(val_b.is_none(), "store_b should not have been evaluated");

    // Push 1 takes arm 1, so store_b receives 99 and store_a is unchanged.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let val_a = node::state::extract::<u32>(&vm, &[store_a.index()])
        .expect("failed to extract")
        .expect("store_a was None");
    assert_eq!(val_a, 42, "store_a should be unchanged");
    let val_b = node::state::extract::<u32>(&vm, &[store_b.index()])
        .expect("failed to extract")
        .expect("store_b was None");
    assert_eq!(val_b, 99);
}

/// Multi-edge list binding within a branch arm. Inside arm 0, `three` and
/// `four` both feed `sum`'s single input. This must produce a `(list ...)`
/// binding that uses only in-scope sources. Arm 1 takes a separate path
/// through `eight`.
///
/// ```text
///   push_0 ---\              /--arm0--> three \
///               select -----<            four  +--> sum --\
///   push_1 ---/              \--arm1--> eight ------------ number
/// ```
#[test]
fn test_graph_multi_edge_in_branch_arm() {
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
            node::parse_expr(&format!(
                "(if (equal? 0 {x}) (list 0 '() '()) (list 1 '() '()))"
            ))
        }
    }

    let mut g = petgraph::graph::DiGraph::new();
    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let select = g.add_node(Box::new(Select) as Box<_>);
    let three = g.add_node(Box::new(node_int(3)) as Box<_>);
    let four = g.add_node(Box::new(node_int(4)) as Box<_>);
    let sum = g.add_node(Box::new(node::expr("(apply + $x)").unwrap()) as Box<_>);
    let eight = g.add_node(Box::new(node_int(8)) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);

    g.add_edge(push_0, select, Edge::from((0, 0)));
    g.add_edge(push_1, select, Edge::from((0, 0)));
    // On arm 0, select output 0 fans out to three and four.
    g.add_edge(select, three, Edge::from((0, 0)));
    g.add_edge(select, four, Edge::from((0, 0)));
    // Both feed sum's input 0 as a multi-edge list within the arm.
    g.add_edge(three, sum, Edge::from((0, 0)));
    g.add_edge(four, sum, Edge::from((0, 0)));
    // On arm 1, select output 1 goes to eight.
    g.add_edge(select, eight, Edge::from((1, 0)));
    // Both arms converge at number.
    g.add_edge(sum, number, Edge::from((0, 0)));
    g.add_edge(eight, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 takes arm 0. sum receives (list 3 4) and yields 7, so number
    // stores 7.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 7);

    // Push 1 takes arm 1. eight gives 8, so number stores 8.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 8);
}

/// An Expr node with an optional `$?var` input.
///
/// ```text
///   push ----> add_opt ----> store
/// ```
///
/// `add_opt` uses `(if (Some? $?b) (Some->value $?b) 0)` to default to 0
/// when `$?b` is unconnected. When `$?b` is unconnected, `(None)` is
/// substituted. `(Some? (None))` is false, so the result is `5 + 0 = 5`.
#[test]
fn test_graph_optional_input_unconnected() {
    let mut g = petgraph::graph::DiGraph::new();

    let push = g.add_node(Box::new(node_int(5).with_push_eval()) as Box<dyn DebugNode>);
    let add_opt = g.add_node(Box::new(
        node::expr("(+ $a (if (Some? $?b) (Some->value $?b) 0))").unwrap(),
    ) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);

    // Only connect $a at input 0. $?b at input 1 stays unconnected.
    g.add_edge(push, add_opt, Edge::from((0, 0)));
    g.add_edge(add_opt, store, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    // 5 + 0 = 5.
    let val = node::state::extract::<u32>(&vm, &[store.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 5);
}

/// An Expr node with an optional `$?var` input that is connected.
///
/// ```text
///   push ----> three ----> add_opt ----> store
///                    \--/
/// ```
///
/// When `$?b` is connected, `(Some 3)` is substituted. `(Some? (Some 3))`
/// is true, so the result is `5 + 3 = 8`.
#[test]
fn test_graph_optional_input_connected() {
    let mut g = petgraph::graph::DiGraph::new();

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

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    // 5 + 3 = 8.
    let val = node::state::extract::<u32>(&vm, &[store.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 8);
}

/// A three-way branch with reconvergence. The other branch tests use 2-arm
/// branches. This exercises the branch loop and join handling with 3 arms.
///
/// ```text
///   push_0 ---\                 /--arm0--> six   \
///   push_1 ----+-- select3 ---<---arm1--> seven  +--> number
///   push_2 ---/                 \--arm2--> eight /
/// ```
#[test]
fn test_graph_three_way_branch() {
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

    let mut g = petgraph::graph::DiGraph::new();
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

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 takes arm 0, so number stores 6.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 6);

    // Push 1 takes arm 1, so number stores 7.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 7);

    // Push 2 takes arm 2, so number stores 8.
    let ep_2 = entrypoint::push(vec![push_2.index()], g[push_2].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_2.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 8);
}

/// Branch with 1 output and 2 branches. Branch 0 is active and branch 1 is
/// dead. When branch 1 is taken at runtime, evaluation must terminate at the
/// branch node. Downstream `number` must not execute.
///
/// ```text
///   push_0 (emits 0) ---\
///                         branch ---(out 0, branch 0)--> number
///   push_1 (emits 1) ---/
/// ```
///
/// Branch 0 is `[true]`, so output 0 is active and downstream runs. Branch 1
/// is `[false]`, so output 0 is dead and downstream does not run.
#[test]
fn test_graph_branch_single_output_dead_branch() {
    let branch = node::Branch::new(
        "(if (equal? 0 $x) (list 0 42) (list 1 99))",
        vec![
            node::Conns::try_from([true]).unwrap(),
            node::Conns::try_from([false]).unwrap(),
        ],
    )
    .unwrap();

    let mut g = petgraph::graph::DiGraph::new();

    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let branch_ix = g.add_node(Box::new(branch) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);

    g.add_edge(push_0, branch_ix, Edge::from((0, 0)));
    g.add_edge(push_1, branch_ix, Edge::from((0, 0)));
    g.add_edge(branch_ix, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 takes the active branch 0, so number stores 42.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 42);

    // Push 1 takes the dead branch 1, so number is not updated.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(
        val, 42,
        "number should still be 42 - dead branch must not propagate"
    );
}

/// Branch with 2 outputs and 2 branches. Branch 0 is fully active and branch
/// 1 is fully dead.
///
/// ```text
///   push_0 (emits 0) ---\                /---(out 0)--> store_a
///                         branch --------<
///   push_1 (emits 1) ---/                \---(out 1)--> store_b
/// ```
///
/// Branch 0 is `[true, true]`, so both outputs are active. Branch 1 is
/// `[false, false]`, so both outputs are dead and evaluation terminates.
#[test]
fn test_graph_branch_two_outputs_one_dead() {
    let branch = node::Branch::new(
        "(if (equal? 0 $x) (list 0 (list 42 43)) (list 1 (list 99 100)))",
        vec![
            node::Conns::try_from([true, true]).unwrap(),
            node::Conns::try_from([false, false]).unwrap(),
        ],
    )
    .unwrap();

    let mut g = petgraph::graph::DiGraph::new();

    let push_0 = g.add_node(Box::new(node_int(0).with_push_eval()) as Box<dyn DebugNode>);
    let push_1 = g.add_node(Box::new(node_int(1).with_push_eval()) as Box<_>);
    let branch_ix = g.add_node(Box::new(branch) as Box<_>);
    let store_a = g.add_node(Box::new(node_number()) as Box<_>);
    let store_b = g.add_node(Box::new(node_number()) as Box<_>);

    g.add_edge(push_0, branch_ix, Edge::from((0, 0)));
    g.add_edge(push_1, branch_ix, Edge::from((0, 0)));
    g.add_edge(branch_ix, store_a, Edge::from((0, 0)));
    g.add_edge(branch_ix, store_b, Edge::from((1, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 takes the active branch 0, so store_a is 42 and store_b is 43.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val_a = node::state::extract::<u32>(&vm, &[store_a.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val_a, 42);
    let val_b = node::state::extract::<u32>(&vm, &[store_b.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val_b, 43);

    // Push 1 takes the dead branch 1, so the stores remain unchanged.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let val_a = node::state::extract::<u32>(&vm, &[store_a.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val_a, 42, "store_a should be unchanged after dead branch");
    let val_b = node::state::extract::<u32>(&vm, &[store_b.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val_b, 43, "store_b should be unchanged after dead branch");
}

/// Branch where all branches have zero active outputs. The branch node's
/// expression still evaluates, so side effects such as state updates run.
/// Nothing propagates downstream.
///
/// ```text
///   push ----> branch ---x---> number (should never run)
/// ```
///
/// Branch 0 and branch 1 are both `[false]`, so both are dead.
#[test]
fn test_graph_branch_all_dead() {
    // A stateful branch. It stores the value in state, then returns branch
    // info.
    let branch = node::Branch::new(
        "(begin (set! state $x) (if (equal? 0 $x) (list 0 '()) (list 1 '())))",
        vec![
            node::Conns::try_from([false]).unwrap(),
            node::Conns::try_from([false]).unwrap(),
        ],
    )
    .unwrap();

    let mut g = petgraph::graph::DiGraph::new();

    let push = g.add_node(Box::new(node_int(42).with_push_eval()) as Box<dyn DebugNode>);
    let branch_ix = g.add_node(Box::new(branch) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);

    g.add_edge(push, branch_ix, Edge::from((0, 0)));
    g.add_edge(branch_ix, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // The push makes the branch evaluate and store 42 in state, but number is
    // unreachable.
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    // The branch's own state is updated.
    let branch_state = node::state::extract::<u32>(&vm, &[branch_ix.index()])
        .expect("failed to extract")
        .expect("branch state was None");
    assert_eq!(branch_state, 42);

    // number is never evaluated.
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .ok()
        .flatten();
    assert!(
        number_state.is_none(),
        "number should not have been evaluated"
    );
}

/// A Pd-style `+` node. The hot left inlet triggers output. The cold right
/// inlet only updates state. It uses a `$?var` optional input and a Branch.
///
/// ```text
///   push_hot (5) ----> [hot_inlet] +_node [cold_inlet] <---- push_cold (3)
///                                    |
///                                  number (stores result)
/// ```
///
/// When the hot inlet fires, the node adds the stored cold value and emits.
/// When the cold inlet fires, it stores the new value in state but does not
/// emit.
#[test]
fn test_graph_branch_optional_input_pd_add() {
    // The hot inlet `$a` is required and the cold inlet `$?b` is optional.
    // When `$a` is a number, update state from `$?b` if present, then emit
    // `$a` + state on branch 0. Otherwise `$a` is '(), so update state from
    // `$?b` and take the dead branch 1.
    let pd_add = node::Branch::new(
        "(begin \
           (if (Some? $?b) (set! state (Some->value $?b)) '()) \
           (if (number? $a) \
               (list 0 (+ $a (if (number? state) state 0))) \
               (list 1 '())))",
        vec![
            node::Conns::try_from([true]).unwrap(),  // branch 0: emit
            node::Conns::try_from([false]).unwrap(), // branch 1: dead (cold only)
        ],
    )
    .unwrap();

    let mut g = petgraph::graph::DiGraph::new();

    let push_hot = g.add_node(Box::new(node_int(5).with_push_eval()) as Box<dyn DebugNode>);
    let push_cold = g.add_node(Box::new(node_int(3).with_push_eval()) as Box<_>);
    let pd_add_ix = g.add_node(Box::new(pd_add) as Box<_>);
    let number = g.add_node(Box::new(node_number()) as Box<_>);

    // The cold inlet is input 0 since `$?b` appears first in the expression.
    g.add_edge(push_cold, pd_add_ix, Edge::from((0, 0)));
    // The hot inlet is input 1 since `$a` appears second.
    g.add_edge(push_hot, pd_add_ix, Edge::from((0, 1)));
    g.add_edge(pd_add_ix, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push cold 3 to set state. number is not updated.
    let ep_cold = entrypoint::push(vec![push_cold.index()], g[push_cold].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_cold.id()), vec![])
        .unwrap();
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .ok()
        .flatten();
    assert!(
        number_state.is_none(),
        "number should not run on cold inlet push"
    );

    // Push hot 5. pd_add computes 5 + 3 = 8 and emits.
    let ep_hot = entrypoint::push(vec![push_hot.index()], g[push_hot].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_hot.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 8, "hot inlet should emit 5 + 3 = 8");
}
