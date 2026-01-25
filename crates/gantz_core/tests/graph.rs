// Tests for the graph module.

use gantz_core::compile::{pull_eval_fn_name, push_eval_fn_name};
use gantz_core::node::{self, Node, WithPullEval, WithPushEval};
use gantz_core::{Edge, ROOT_STATE};
use std::fmt::Debug;
use steel::SteelVal;
use steel::parser::ast::ExprKind;
use steel::steel_vm::engine::Engine;

fn node_push() -> node::Push<(), node::Expr> {
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
trait DebugNode: Debug + Node<()> {}
impl<T> DebugNode for T where T: Debug + Node<()> {}

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

    // No need to share an environment between nodes for this test.
    let env = ();

    // Generate the module, which should have just one top-level expr for `push`.
    let module = gantz_core::compile::module(&env, &g).unwrap();
    // Function per node alongside the single push eval function.
    assert_eq!(module.len(), g.node_count() + 1);

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&env, &g, &[], &mut vm);

    // Register the functions, then call push_eval.
    for f in module {
        vm.run(format!("{f}")).unwrap();
    }
    vm.call_function_by_name_with_args(&push_eval_fn_name(&[push.index()]), vec![])
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

    // No need to share an environment between nodes for this test.
    let env = ();

    // Generate the steel module.
    let module = gantz_core::compile::module(&env, &g).unwrap();

    // Prepare the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&env, &g, &[], &mut vm);

    // Prepare the eval fn.
    for expr in module {
        vm.run(expr.to_pretty(100)).unwrap();
    }

    // Call the eval fn.
    vm.call_function_by_name_with_args(&pull_eval_fn_name(&[assert_eq.index()]), vec![])
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

    impl<Env> Node<Env> for Select {
        fn n_inputs(&self, _: &Env) -> usize {
            1
        }

        fn n_outputs(&self, _: &Env) -> usize {
            2
        }

        fn branches(&self, _: &Env) -> Vec<node::EvalConf> {
            vec![
                node::EvalConf::Set([true, false].try_into().unwrap()),
                node::EvalConf::Set([false, true].try_into().unwrap()),
            ]
        }

        fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
            let x = ctx.inputs()[0].as_deref().expect("must have one input");
            let expr = format!(
                r#"
                (if (equal? 0 {x})
                  (list 0 '())  ; 0 index for left branch, '() for empty value
                  (list 1 '())) ; 1 index for right branch, '() for empty value
            "#
            );
            Engine::emit_ast(&expr).unwrap().into_iter().next().unwrap()
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

    // No need to share an environment between nodes for this test.
    let env = ();

    // Generate the module.
    let module = gantz_core::compile::module(&env, &g).unwrap();
    // Function per node alongside the two push eval functions.
    assert_eq!(module.len(), g.node_count() + 2);

    // Create the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&env, &g, &[], &mut vm);

    // Register the functions, then call push_eval.
    for f in module {
        vm.run(format!("{f}")).unwrap();
    }

    // First, call `push_0` and check the result is `6`.
    vm.call_function_by_name_with_args(&push_eval_fn_name(&[push_0.index()]), vec![])
        .unwrap();
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was `None`");
    assert_eq!(number_state, 6);

    // First, call `push_1` and check the result is `7`.
    vm.call_function_by_name_with_args(&push_eval_fn_name(&[push_1.index()]), vec![])
        .unwrap();
    let number_state = node::state::extract::<u32>(&vm, &[number.index()])
        .expect("failed to extract number state")
        .expect("number state was `None`");
    assert_eq!(number_state, 7);
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

    // No need to share an environment between nodes for this test.
    let env = ();

    // Generate the steel module.
    let module = gantz_core::compile::module(&env, &g).unwrap();

    // Prepare the VM.
    let mut vm = Engine::new_base();

    // Initialise the node state.
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&env, &g, &[], &mut vm);

    // Run the module.
    for expr in module {
        vm.run(expr.to_pretty(100)).unwrap();
    }
    vm.call_function_by_name_with_args(&pull_eval_fn_name(&[assert_eq.index()]), vec![])
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

    impl<Env> Node<Env> for Src {
        fn push_eval(&self, _: &Env) -> Vec<node::EvalConf> {
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

        fn n_outputs(&self, _: &Env) -> usize {
            2
        }

        fn expr(&self, ctx: node::ExprCtx<Env>) -> ExprKind {
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
            Engine::emit_ast(&expr).unwrap().into_iter().next().unwrap()
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

    // No need to share an environment between nodes for this test.
    let env = ();

    // Generate the module
    let module = gantz_core::compile::module(&env, &g).unwrap();

    // Create the VM
    let mut vm = Engine::new_base();

    // Initialize the state
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&env, &g, &[], &mut vm);

    // Register all functions
    for f in module {
        vm.run(f.to_pretty(100)).unwrap();
    }

    // Call the push_eval function - should only evaluate the first output path
    // FIXME: Update push_eval_fn_name
    vm.call_function_by_name_with_args(&push_eval_fn_name(&[source.index()]), vec![])
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
