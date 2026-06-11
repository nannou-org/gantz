//! Tests for the log node's provenance: the emitted expression carries the
//! node's own path so log entries identify their emitting node.

use gantz_core::{
    Edge, Node,
    compile::push_pull_entrypoints,
    node::{self, WithPushEval},
};
use std::fmt::Debug;

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A no-op node lookup function for tests that don't need it.
fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

// The emitted module passes the log node's path as a quoted literal to the
// registered log fn.
#[test]
fn log_expr_carries_node_path() {
    let mut g = petgraph::graph::DiGraph::new();
    let push = g.add_node(Box::new(node::expr("'()").unwrap().with_push_eval()) as Box<dyn DebugNode>);
    let int = g.add_node(Box::new(node::expr("(begin $push 7)").unwrap()) as Box<_>);
    let log = g.add_node(Box::new(gantz_std::Log::default()) as Box<_>);
    g.add_edge(push, int, Edge::from((0, 0)));
    g.add_edge(int, log, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &g);
    let module = gantz_core::compile::module(&no_lookup, &g, &eps, &Default::default()).unwrap();
    let src = gantz_core::vm::fmt_module(&module);
    let expected = format!("(log/info (quote ({})) ", log.index());
    assert!(
        src.contains(&expected) || src.contains(&format!("(log/info '({})", log.index())),
        "module does not pass the log node's path:\n{src}"
    );
}
