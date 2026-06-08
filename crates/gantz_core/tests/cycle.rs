//! Tests for cyclic graphs (feedback loops).
//!
//! gantz lowers a cycle as an iterate-until-branch loop. This file covers the
//! ill-formed cases that must be rejected at compile time. End-to-end loop
//! evaluation tests are added once loop codegen lands.

use gantz_core::compile::error::{CodegenError, LoopError, ModuleError, NodeConnsError};
use gantz_core::compile::{entry_fn_name, entrypoint, push_pull_entrypoints};
use gantz_core::node::{self, Node, WithPushEval};
use gantz_core::{Edge, ROOT_STATE};
use std::fmt::Debug;
use steel::SteelVal;
use steel::steel_vm::engine::Engine;

/// A push-eval trigger node.
fn node_push() -> node::Push<node::Expr> {
    node::expr("'()").unwrap().with_push_eval()
}

/// A stateful node that stores the received number and returns it (so a test can
/// read the loop's result via `node::state::extract_value`).
fn node_number() -> node::Expr {
    node::expr("(let ((x $x)) (set! state (if (number? x) x state)) state)").unwrap()
}

/// A stateful passthrough that counts how many times it is evaluated (its state
/// increments each call), returning its input unchanged.
fn node_iter_counter() -> node::Expr {
    node::expr("(begin (set! state (+ (if (number? state) state 0) 1)) $x)").unwrap()
}

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

/// A cycle with no branch node can never terminate, so it is rejected.
#[test]
fn no_branch_cycle_errors() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let a = g.add_node(Box::new(node::expr("$x").unwrap()) as Box<_>);
    let b = g.add_node(Box::new(node::expr("$y").unwrap()) as Box<_>);
    g.add_edge(push, a, Edge::from((0, 0)));
    g.add_edge(a, b, Edge::from((0, 0)));
    g.add_edge(b, a, Edge::from((0, 0))); // back-edge closes a -> b -> a

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let err = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap_err();
    assert!(
        matches!(
            err,
            ModuleError::NodeConns(NodeConnsError::Loop(LoopError::InfiniteFeedbackLoop { .. }))
        ),
        "expected InfiniteFeedbackLoop, got {err:?}"
    );
}

/// A loop entered at two distinct nodes is irreducible, so it is rejected.
#[test]
fn irreducible_loop_errors() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let a = g.add_node(Box::new(node::expr("$x").unwrap()) as Box<_>);
    let b = g.add_node(Box::new(node::expr("$y").unwrap()) as Box<_>);
    // `push` fans out to BOTH a and b, so the SCC {a, b} has two external
    // entries - an irreducible (multi-entry) loop.
    g.add_edge(push, a, Edge::from((0, 0)));
    g.add_edge(push, b, Edge::from((0, 0)));
    g.add_edge(a, b, Edge::from((0, 0)));
    g.add_edge(b, a, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let err = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap_err();
    assert!(
        matches!(
            err,
            ModuleError::NodeConns(NodeConnsError::Loop(LoopError::IrreducibleLoop { .. }))
        ),
        "expected IrreducibleLoop, got {err:?}"
    );
}

/// Compile + run a graph, push from `source` once, and return the VM.
fn run_once(
    g: &petgraph::graph::DiGraph<Box<dyn DebugNode>, Edge>,
    source: petgraph::graph::NodeIndex,
) -> Engine {
    let eps = push_pull_entrypoints(&no_lookup, g);
    let module = gantz_core::compile::module(&no_lookup, g, &eps).unwrap();
    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&no_lookup, g, &[], &mut vm);
    for f in &module {
        vm.run(format!("{f}")).unwrap();
    }
    let ctx = node::MetaCtx::new(&no_lookup);
    let ep = entrypoint::push(vec![source.index()], g[source].n_outputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
    vm
}

/// The headline counter: a cycle through an adder, terminated by a branch.
///
/// ```text
///   start(0) --> add(+ $acc 1) --> branch(< $sum 3 ?)
///                  ^                  | arm0 (continue): back to add
///                  +------------------+
///                                     | arm1 (exit): -> out (stores result)
/// ```
///
/// One push runs the loop to completion: acc 0->1->2->3, exiting when sum >= 3.
#[test]
fn counter_to_n() {
    let mut g = petgraph::graph::DiGraph::new();
    let start = g.add_node(Box::new(node::expr("0").unwrap().with_push_eval()) as Box<dyn DebugNode>);
    let add = g.add_node(Box::new(node::expr("(+ $acc 1)").unwrap()) as Box<_>);
    let branch = g.add_node(Box::new(
        node::branch(
            "(if (< $sum 3) (list 0 $sum) (list 1 $sum))",
            vec!["10".parse().unwrap(), "01".parse().unwrap()],
        )
        .unwrap(),
    ) as Box<_>);
    let out = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(start, add, Edge::from((0, 0))); // initial acc = 0
    g.add_edge(add, branch, Edge::from((0, 0))); // sum -> branch
    g.add_edge(branch, add, Edge::from((0, 0))); // arm0 continue: back-edge to acc
    g.add_edge(branch, out, Edge::from((1, 0))); // arm1 exit: result -> out

    // The generated module should define and call a loop fn for the header.
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap();
    let text = module
        .iter()
        .map(|e| e.to_pretty(80))
        .collect::<Vec<_>>()
        .join("\n");
    let loop_fn = format!("loopfn-{}", add.index());
    assert!(
        text.contains(&format!("({loop_fn} ")) || text.contains(&format!("({loop_fn})")),
        "expected a `{loop_fn}` definition/call in:\n{text}"
    );

    let vm = run_once(&g, start);
    let result = node::state::extract_value(&vm, &[out.index()])
        .unwrap()
        .unwrap();
    assert_eq!(result, SteelVal::IntV(3), "counter should reach 3");
}

/// A single branch node that both increments and decides, with a self back-edge
/// (the header *is* the deciding branch). Counts up to `limit`.
fn self_loop_counter_graph(
    limit: i64,
) -> (
    petgraph::graph::DiGraph<Box<dyn DebugNode>, Edge>,
    petgraph::graph::NodeIndex,
    petgraph::graph::NodeIndex,
) {
    let mut g = petgraph::graph::DiGraph::new();
    let start = g.add_node(Box::new(node::expr("0").unwrap().with_push_eval()) as Box<dyn DebugNode>);
    let counter = g.add_node(Box::new(
        node::branch(
            format!("(let ((n (+ $acc 1))) (if (< n {limit}) (list 0 n) (list 1 n)))"),
            vec!["10".parse().unwrap(), "01".parse().unwrap()],
        )
        .unwrap(),
    ) as Box<_>);
    let out = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(start, counter, Edge::from((0, 0))); // initial acc = 0
    g.add_edge(counter, counter, Edge::from((0, 0))); // self back-edge (continue)
    g.add_edge(counter, out, Edge::from((1, 0))); // exit -> out
    (g, start, out)
}

/// A self-loop (header == deciding branch) counts to the limit in one push.
#[test]
fn self_loop_counter() {
    let (g, start, out) = self_loop_counter_graph(3);
    let vm = run_once(&g, start);
    let result = node::state::extract_value(&vm, &[out.index()])
        .unwrap()
        .unwrap();
    assert_eq!(result, SteelVal::IntV(3));
}

/// A stateful node inside the loop body accumulates across iterations - the loop
/// fn closes over the single mutable `graph-state`, so state persists between
/// tail-recursive calls (the basis for delays/counters with internal memory).
#[test]
fn stateful_accumulator_in_loop() {
    let mut g = petgraph::graph::DiGraph::new();
    let start = g.add_node(Box::new(node::expr("0").unwrap().with_push_eval()) as Box<dyn DebugNode>);
    let add = g.add_node(Box::new(node::expr("(+ $acc 1)").unwrap()) as Box<_>);
    let iter = g.add_node(Box::new(node_iter_counter()) as Box<_>);
    let branch = g.add_node(Box::new(
        node::branch(
            "(if (< $sum 3) (list 0 $sum) (list 1 $sum))",
            vec!["10".parse().unwrap(), "01".parse().unwrap()],
        )
        .unwrap(),
    ) as Box<_>);
    let out = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(start, add, Edge::from((0, 0)));
    g.add_edge(add, iter, Edge::from((0, 0))); // count this iteration, pass sum on
    g.add_edge(iter, branch, Edge::from((0, 0)));
    g.add_edge(branch, add, Edge::from((0, 0))); // continue
    g.add_edge(branch, out, Edge::from((1, 0))); // exit

    let vm = run_once(&g, start);
    let result = node::state::extract_value(&vm, &[out.index()])
        .unwrap()
        .unwrap();
    assert_eq!(result, SteelVal::IntV(3), "loop result");
    let iters = node::state::extract_value(&vm, &[iter.index()])
        .unwrap()
        .unwrap();
    assert_eq!(iters, SteelVal::IntV(3), "state should accumulate 3 iterations");
}

/// A loop whose body contains an inner (forward) branch is multi-block, which
/// v1 codegen does not yet handle - it must error clearly, not mis-compile.
#[test]
fn inner_branch_loop_unsupported() {
    let mut g = petgraph::graph::DiGraph::new();
    let start = g.add_node(Box::new(node::expr("0").unwrap().with_push_eval()) as Box<dyn DebugNode>);
    let add = g.add_node(Box::new(node::expr("(+ $acc 1)").unwrap()) as Box<_>);
    // An inner forward branch whose two arms reconverge at the deciding branch.
    let inner = g.add_node(Box::new(
        node::branch(
            "(if (< $x 1000) (list 0 $x) (list 1 $x))",
            vec!["10".parse().unwrap(), "01".parse().unwrap()],
        )
        .unwrap(),
    ) as Box<_>);
    let decide = g.add_node(Box::new(
        node::branch(
            "(if (< $sum 3) (list 0 $sum) (list 1 $sum))",
            vec!["10".parse().unwrap(), "01".parse().unwrap()],
        )
        .unwrap(),
    ) as Box<_>);
    let out = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(start, add, Edge::from((0, 0)));
    g.add_edge(add, inner, Edge::from((0, 0)));
    g.add_edge(inner, decide, Edge::from((0, 0))); // inner arm0 -> decide
    g.add_edge(inner, decide, Edge::from((1, 0))); // inner arm1 -> decide (reconverge)
    g.add_edge(decide, add, Edge::from((0, 0))); // continue (back-edge)
    g.add_edge(decide, out, Edge::from((1, 0))); // exit

    // F1: analysis now allows an inner branch (no overflow); F2 codegen is not
    // yet wired, so the multi-block body is rejected at codegen instead.
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let err = gantz_core::compile::module(&no_lookup, &g, &eps).unwrap_err();
    assert!(
        matches!(
            err,
            ModuleError::Codegen(CodegenError::UnsupportedLoopShape { .. })
        ),
        "expected UnsupportedLoopShape, got {err:?}"
    );
}

/// Compiling a cyclic graph is reproducible (required for content addressing).
#[test]
fn cyclic_codegen_is_deterministic() {
    let (g, _start, _out) = self_loop_counter_graph(5);
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let pretty = |g: &petgraph::graph::DiGraph<Box<dyn DebugNode>, Edge>| {
        gantz_core::compile::module(&no_lookup, g, &eps)
            .unwrap()
            .iter()
            .map(|e| e.to_pretty(80))
            .collect::<Vec<_>>()
    };
    assert_eq!(pretty(&g), pretty(&g));
}
