// Tests for the graph module.

use gantz_core::compile::{default_entrypoints, entry_fn_name, entrypoint, push_source};
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

// Helper trait for debugging the graph.
trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A no-op node lookup function for tests that don't need it.
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

    // Instantiate the nodes.
    let push = node_push();
    let one = node_int(1);
    let add = node_add();
    let two = node_int(2);
    let assert_eq = node_assert_eq();

    // Add the nodes to the project.
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

    // Generate the module, which should have just one top-level expr for `push`.
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();
    // Function per node alongside the single push eval function.
    assert_eq!(module.len(), g.node_count() + 1);

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    // Register the functions, then call push_eval.
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

    // Instantiate the nodes.
    let one = node_int(1);
    let add = node_add();
    let two = node_int(2);
    let assert_eq = node_assert_eq().with_pull_eval();

    // Add the nodes to the project.
    let one = g.add_node(Box::new(one) as Box<dyn DebugNode>);
    let add = g.add_node(Box::new(add) as Box<_>);
    let two = g.add_node(Box::new(two) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);
    g.add_edge(one, add, Edge::from((0, 0)));
    g.add_edge(one, add, Edge::from((0, 1)));
    g.add_edge(add, assert_eq, Edge::from((0, 0)));
    g.add_edge(two, assert_eq, Edge::from((0, 1)));

    let ctx = node::MetaCtx::new(&no_lookup);

    // Generate the steel module.
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    // Prepare the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    // Prepare the eval fn.
    for expr in module {
        vm.run(expr.to_pretty(100)).unwrap();
    }

    // Call the eval fn.
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

    // Instantiate the nodes.
    let push_0 = node_int(0).with_push_eval();
    let push_1 = node_int(1).with_push_eval();
    let select = Select;
    let six = node_int(6);
    let seven = node_int(7);
    let number = node_number();

    // Create the graph.
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

    // Generate the module.
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();
    // Function per node alongside the two push eval functions.
    assert_eq!(module.len(), g.node_count() + 2);

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    // Register the functions, then call push_eval.
    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    // First, call `push_0` and check the result is `6`.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was `None`");
    assert_eq!(number_state, 6);

    // First, call `push_1` and check the result is `7`.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was `None`");
    assert_eq!(number_state, 7);
}

// Verify that the conditional eval codegen does not duplicate the join
// node's function call in the generated Scheme. The "number" node sits
// after a branch-and-join and should appear exactly once in each entry
// function body.
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
    let module = gantz_core::compile::module(&no_lookup, &g, &[ep]).unwrap();

    // The number node's function name contains its index (5).
    // Count how many times it appears across all generated expressions.
    let module_str = module
        .iter()
        .map(|e| format!("{e}"))
        .collect::<Vec<_>>()
        .join("\n");
    let number_fn_prefix = format!("node-fn-{}", number.index());
    let count = module_str.matches(&number_fn_prefix).count();
    // Once in the node function definition, once in the entry fn body call.
    // If the join is duplicated, the entry fn body would contain 2+ calls.
    assert!(
        count <= 2,
        "number node fn '{}' appears {} times - join is duplicated!\nGenerated module:\n{}",
        number_fn_prefix,
        count,
        module_str,
    );
}

// Edge case: one branch goes *directly* to the join node (no intermediate
// nodes), while the other branch goes through an intermediate node first.
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
// Branch 0 (left): select's output 0 goes directly to number (the join).
// Branch 1 (right): select's output 1 goes through seven, then to number.
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
            // Branch 0: pass 6 as the value. Branch 1: pass input through.
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

    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0: select goes left, passes 6 directly to number.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("state was None");
    assert_eq!(state, 6);

    // Push 1: select goes right, through seven (constant 7) to number.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("state was None");
    assert_eq!(state, 7);

    // Verify no join duplication in the generated code.
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

/// Both branch outputs feed the same target input - phi_set_stmts must
/// reference the correct output variable for each arm.
///
///   push_0 --\          /-- output 0 --\
///              select --<               number
///   push_1 --/          \-- output 1 --/
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
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 -> arm 0 -> number receives 42.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("number was None");
    assert_eq!(val, 42);

    // Push 1 -> arm 1 -> number receives 99.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("number was None");
    assert_eq!(val, 99);
}

// Nested diamond: outer branch contains an inner branch, both with
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
// Push 0 → outer-left → inner-right (since select_inner gets '()
// which != 0) → seven(7) → inner_result(7) → outer_result(7).
// Push 1 → outer-right → eight(8) → outer_result(8).
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

    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0: outer-left → inner-right (since '() != 0) → seven(7)
    // → inner_result stores 7, outer_result stores 7.
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

    // Push 1: outer-right → eight(8) → outer_result stores 8.
    // inner_result is not evaluated, stays at 7.
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

    // Verify neither join node's function call is duplicated.
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

// Lattice: both outer branch targets share the same inner branching
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
// Push 0 -> outer-left -> sel_L -> right (since '() != 0) -> seven(7) -> number(7).
// Push 1 -> outer-right -> sel_R -> right (since '() != 0) -> seven(7) -> number(7).
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

    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0: outer-left → sel_L → right (default) → seven(7) → number(7).
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("state was None");
    assert_eq!(state, 7);

    // Push 1: outer-right → sel_R → right (default) → seven(7) → number(7).
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("state was None");
    assert_eq!(state, 7);

    // Verify number's fn call is not duplicated per outer branch.
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

    // Instantiate the nodes.
    let one = node_int(1);
    let add = node_add();
    let assert_eq = node_assert_eq().with_pull_eval();

    // Add the nodes to the project.
    let one = g.add_node(Box::new(one) as Box<dyn DebugNode>);
    let add = g.add_node(Box::new(add) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);
    g.add_edge(one, add, Edge::from((0, 0)));
    g.add_edge(one, add, Edge::from((0, 1)));
    g.add_edge(add, assert_eq, Edge::from((0, 0)));
    g.add_edge(one, assert_eq, Edge::from((0, 1)));

    let ctx = node::MetaCtx::new(&no_lookup);

    // Generate the steel module.
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    // Prepare the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    // Run the module.
    for expr in module {
        vm.run(expr.to_pretty(100)).unwrap();
    }
    let ep = entrypoint::pull(vec![assert_eq.index()], g[assert_eq].n_inputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}

// Test for pushing evaluation with a subset of outputs enabled
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
            // Generate 3 push eval fns.
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

    // Add nodes to the graph.
    let source = g.add_node(Box::new(source) as Box<dyn DebugNode>);
    let store_a = g.add_node(Box::new(store_a) as Box<_>);
    let store_b = g.add_node(Box::new(store_b) as Box<_>);

    // Connect outputs to store nodes
    g.add_edge(source, store_a, Edge::from((0, 0)));
    g.add_edge(source, store_b, Edge::from((1, 0)));

    // Generate the module
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    // Create the VM
    let mut vm = Engine::new_base();

    // Initialize the state
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    // Register all functions
    for f in module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Call the push_eval function - should only evaluate the first output path
    let ep = &eps[0]; // first push eval conf: only first output
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();

    // Check the state of each store node
    let store_a_val = node::state::extract::<i32>(&vm, &[store_a.index()]).unwrap();
    let store_b_val = node::state::extract::<i32>(&vm, &[store_b.index()]).unwrap();

    // First output was enabled for push, so its state should be 6
    assert_eq!(store_a_val, Some(6));

    // Second output was not enabled for push, so its state should be None
    // (never evaluated)
    assert_eq!(store_b_val, None);
}

// Test that a multi-source entrypoint combines two push nodes into one eval fn.
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

    // Build a single combined entrypoint from both push nodes.
    let combined = entrypoint::from_sources([
        push_source(vec![push_a.index()], g[push_a].n_outputs(ctx) as u8),
        push_source(vec![push_b.index()], g[push_b].n_outputs(ctx) as u8),
    ]);
    let module = gantz_core::compile::module(&no_lookup, &g, &[combined.clone()]).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Calling the combined entrypoint should evaluate BOTH chains.
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

// Verify that push_entrypoint produces the same EntrypointId as
// default_entrypoints for the same node, confirming naming consistency.
#[test]
fn test_entrypoint_naming_consistency() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int = g.add_node(Box::new(node_int(1)) as Box<_>);
    g.add_edge(push, int, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);

    let eps = default_entrypoints(&no_lookup, &g);
    let manual = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);

    // The default planner should produce a singleton push entrypoint for
    // the push node. Its ID should match a manually-constructed one.
    let default_ep = eps
        .iter()
        .find(|ep| ep.0.iter().any(|s| s.path == vec![push.index()]))
        .expect("default_entrypoints should contain push node");
    assert_eq!(default_ep.id(), manual.id());
    assert_eq!(entry_fn_name(&default_ep.id()), entry_fn_name(&manual.id()));
}

// A 2-output expr node returns `(values 6 7)`. Each output is wired to a
// separate stateful store node. After push evaluation, each store should hold
// the corresponding value.
//
//    --------
//    | push |
//    --------
//       |
//    ----------
//    | pair   |  outputs=2, expr: (begin $push (values 6 7))
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

    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

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

// Test that nodes with 0 outputs (side-effect-only nodes like Log) work
// correctly even when multiple appear in the same evaluation path.
//
// The graph:
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
// Both effect nodes end up in the same basic block. The first one
// is NOT last in the block, so destructure_node_outputs_stmt is
// called on it. With 0 outputs this must be a no-op, not an
// invalid (define-values () node-X) referencing an undefined binding.
#[test]
fn test_graph_zero_output_leaf_nodes() {
    /// A node with 1 input and 0 outputs (pure side-effect).
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
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in &module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Execute the push entrypoint - should not crash.
    let ep = entrypoint::push(vec![push.index()], g[push].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}

/// Test using the `Branch` node type in a graph with push evaluation.
///
/// Graph layout:
///
///   push_0 (emits 0) ---\
///                         branch --- out0 -> six -> number
///   push_1 (emits 1) ---/       \-- out1 -> seven -> number
///
/// When push_0 fires (input=0), branch selects index 0 -> six -> number stores 6.
/// When push_1 fires (input=1), branch selects index 1 -> seven -> number stores 7.
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
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);

    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 -> branch takes index 0 -> six -> number stores 6.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 6);

    // Push 1 -> branch takes index 1 -> seven -> number stores 7.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 7);
}

// Test that multiple unconditional edges to the same input produce a list.
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
    // Sum all elements in the input list.
    let sum = g.add_node(Box::new(node::expr("(apply + $x)").unwrap()) as Box<_>);
    let store = g.add_node(Box::new(node_number()) as Box<_>);

    g.add_edge(push, three, Edge::from((0, 0)));
    g.add_edge(push, four, Edge::from((0, 0)));
    // Both connect to sum's input 0.
    g.add_edge(three, sum, Edge::from((0, 0)));
    g.add_edge(four, sum, Edge::from((0, 0)));
    g.add_edge(sum, store, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    // Verify the generated code contains a list binding.
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
    // 3 + 4 = 7 (not just 4 from last-write-wins).
    assert_eq!(val, 7);
}

/// Test branching to independent terminal nodes (no reconvergence).
///
/// Exercises the code-duplication fallback in `flow_node_stmts` directly,
/// with `in_scope` cloned per arm and no join or phi variables.
///
///   push_0 (emits 0) ---\              /--- store_a (stateful leaf)
///                         select -----<
///   push_1 (emits 1) ---/              \--- store_b (stateful leaf)
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
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 -> arm 0 -> store_a receives 42.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val_a = node::state::extract::<u32>(&vm, &[store_a.index()])
        .expect("failed to extract")
        .expect("store_a was None");
    assert_eq!(val_a, 42);
    // store_b should be untouched (still void/initial).
    let val_b = node::state::extract::<u32>(&vm, &[store_b.index()])
        .ok()
        .flatten();
    assert!(val_b.is_none(), "store_b should not have been evaluated");

    // Push 1 -> arm 1 -> store_b receives 99, store_a unchanged.
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

/// Test multi-edge list binding within a branch arm.
///
/// Inside arm 0, two nodes (`three` and `four`) both feed `sum`'s single
/// input, which should produce a `(list ...)` binding using only in-scope
/// sources. Arm 1 takes a separate path through `eight`.
///
///   push_0 ---\              /--arm0--> three \
///               select -----<            four  +--> sum --\
///   push_1 ---/              \--arm1--> eight ------------ number
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
    // Arm 0: select output 0 fans out to three and four.
    g.add_edge(select, three, Edge::from((0, 0)));
    g.add_edge(select, four, Edge::from((0, 0)));
    // Both feed sum's input 0 (multi-edge list within the arm).
    g.add_edge(three, sum, Edge::from((0, 0)));
    g.add_edge(four, sum, Edge::from((0, 0)));
    // Arm 1: select output 1 to eight.
    g.add_edge(select, eight, Edge::from((1, 0)));
    // Both arms converge at number.
    g.add_edge(sum, number, Edge::from((0, 0)));
    g.add_edge(eight, number, Edge::from((0, 0)));

    let ctx = node::MetaCtx::new(&no_lookup);
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 -> arm 0 -> sum receives (list 3 4) -> (apply + ...) = 7 -> number stores 7.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 7);

    // Push 1 -> arm 1 -> eight = 8 -> number stores 8.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 8);
}

/// Test a three-way branch with reconvergence.
///
/// All existing branch tests use binary (2-arm) branches. This exercises
/// the generality of the branch loop and phi handling with 3 arms.
///
///   push_0 ---\                 /--arm0--> six   \
///   push_1 ----+-- select3 ---<---arm1--> seven  +--> number
///   push_2 ---/                 \--arm2--> eight /
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
    let eps = default_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, &g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }

    // Push 0 -> arm 0 -> six -> number stores 6.
    let ep_0 = entrypoint::push(vec![push_0.index()], g[push_0].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_0.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 6);

    // Push 1 -> arm 1 -> seven -> number stores 7.
    let ep_1 = entrypoint::push(vec![push_1.index()], g[push_1].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_1.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 7);

    // Push 2 -> arm 2 -> eight -> number stores 8.
    let ep_2 = entrypoint::push(vec![push_2.index()], g[push_2].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep_2.id()), vec![])
        .unwrap();
    let val = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract")
        .expect("was None");
    assert_eq!(val, 8);
}
