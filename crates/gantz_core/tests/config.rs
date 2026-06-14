//! Tests for the [`compile::Config`] toggles.

use gantz_core::{
    Edge,
    compile::{Config, entry_fn_name, push_pull_entrypoints},
    node::{self, Node, WithPushEval},
};
use std::fmt::Debug;

fn node_push() -> node::Push<node::Expr> {
    node::expr("'()").unwrap().with_push_eval()
}

fn node_int(i: i32) -> node::Expr {
    node::expr(format!("(begin $push {})", i)).unwrap()
}

fn node_mul() -> node::Expr {
    node::expr("(* $l $r)").unwrap()
}

fn node_assert_eq() -> node::Expr {
    node::expr("(begin (assert! (equal? $l $r)))").unwrap()
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

type TestGraph = petgraph::graph::DiGraph<Box<dyn DebugNode>, Edge, usize>;
// A nested graph: an ordinary `Graph` (which implements `Node`) boxed into its
// parent, in place of the removed `GraphNode` wrapper.
type Nested = node::graph::Graph<Box<dyn DebugNode>>;

// A graph exercising both toggles: a reachable push chain through a nested
// graph, an unreachable node at the root level (`mul`, id 4), an unreachable
// *stateful* node at the root level (`number`, id 5), and an unreachable node
// inside the nested graph (`mul`, id 2 at level 2).
//
//    --------
//    | push |                      // id 0
//    -+------
//     |
//    -+-----
//    | one |                       // id 1
//    -+-----
//     |---------------
//     |              |
//    -+---------     |             ----------------------
//    | GRAPH    |    |             | GRAPH interior:    |
//    | inlet  0 |    |             |  inlet -> outlet   |
//    | outlet 1 |    |             |  mul (unreachable) |
//    | mul    2 |    |             ----------------------
//    -+----------    |
//     |              |
//    -+--------------+-   -+-----   -+--------
//    | assert_eq       |  | mul |   | number |
//    -------------------  -------   ----------
fn test_graph() -> TestGraph {
    let mut inner = Nested::default();
    let inlet = inner.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let outlet = inner.add_node(Box::new(node::graph::Outlet) as Box<_>);
    let _orphan = inner.add_node(Box::new(node_mul()) as Box<_>);
    inner.add_edge(inlet, outlet, Edge::from((0, 0)));

    let mut g = TestGraph::default();
    let push = g.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let one = g.add_node(Box::new(node_int(1)) as Box<_>);
    let nested = g.add_node(Box::new(inner) as Box<_>);
    let assert_eq = g.add_node(Box::new(node_assert_eq()) as Box<_>);
    let _orphan = g.add_node(Box::new(node_mul()) as Box<_>);
    let _stateful_orphan = g.add_node(Box::new(node_number()) as Box<_>);
    g.add_edge(push, one, Edge::from((0, 0)));
    g.add_edge(one, nested, Edge::from((0, 0)));
    g.add_edge(nested, assert_eq, Edge::from((0, 0)));
    g.add_edge(one, assert_eq, Edge::from((0, 1)));
    g
}

// The all-connected node fn names of the unreachable nodes in `test_graph`.
const ORPHAN_FNS: &[&str] = &["node-fn-4-i11-o1", "node-fn-5-i1-o1", "node-fn-2:2-i11-o1"];

// By default, node fns are emitted on demand: nodes unreachable from every
// entrypoint produce no node fns.
#[test]
fn default_omits_uncalled_node_fns() {
    let g = test_graph();
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Config::default()).unwrap();
    let module_str = gantz_core::vm::fmt_module(&module);
    for fn_name in ORPHAN_FNS {
        assert!(!module_str.contains(fn_name), "unexpected {fn_name}");
    }
}

// With `emit_all_node_fns`, every node's all-connected variant is emitted -
// root level orphans (pure and stateful) and nested level orphans alike -
// and the extra definitions do not disturb evaluation.
#[test]
fn emit_all_node_fns_includes_uncalled_nodes() {
    let config = Config {
        emit_all_node_fns: true,
        ..Default::default()
    };
    let g = test_graph();
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let (mut vm, module) = gantz_core::vm::init(&no_lookup, &g, &eps, &config).unwrap();
    let module_str = module.src;
    for fn_name in ORPHAN_FNS {
        assert!(module_str.contains(fn_name), "missing {fn_name}");
    }

    // The reachable chain still evaluates (the assert_eq node throws on
    // failure), with the dead definitions loaded alongside.
    vm.call_function_by_name_with_args(&entry_fn_name(&eps[0].id()), vec![])
        .unwrap();
}

// Skipping IR validation only skips the check: the emitted module is
// identical to a validated build's.
#[test]
fn no_validate_ir_emits_identical_module() {
    let config = Config {
        validate_ir: false,
        ..Default::default()
    };
    let g = test_graph();
    let eps = push_pull_entrypoints(&no_lookup, &g);
    let validated = gantz_core::compile::module(&no_lookup, &g, &eps, &Config::default()).unwrap();
    let unvalidated = gantz_core::compile::module(&no_lookup, &g, &eps, &config).unwrap();
    assert_eq!(
        gantz_core::vm::fmt_module(&validated),
        gantz_core::vm::fmt_module(&unvalidated),
    );
}
