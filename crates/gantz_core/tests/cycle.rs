//! Tests for cyclic graphs (feedback loops).
//!
//! gantz lowers a cycle as an iterate-until-branch loop. This file covers the
//! ill-formed cases that must be rejected at compile time. End-to-end loop
//! evaluation tests are added once loop codegen lands.

use gantz_core::Edge;
use gantz_core::compile::error::{LoopError, ModuleError, NodeConnsError};
use gantz_core::compile::push_pull_entrypoints;
use gantz_core::node::{self, Node, WithPushEval};
use std::fmt::Debug;

/// A push-eval trigger node.
fn node_push() -> node::Push<node::Expr> {
    node::expr("'()").unwrap().with_push_eval()
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
