//! Tests mapping emitted module source text back to graph nodes via the
//! compile::SourceMap.

use gantz_core::{
    Edge,
    compile::{Name, SourceMap, push_pull_entrypoints},
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

fn node_add() -> node::Expr {
    node::expr("(+ $l $r)").unwrap()
}

// Helper trait for debugging the graph.
trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// A nested graph: an ordinary `Graph` (which implements `Node`) boxed into its
// parent, in place of the removed `GraphNode` wrapper.
type Nested = node::graph::Graph<Box<dyn DebugNode>>;

// A no-op node lookup function for tests that don't need it.
fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
    None
}

// A nested graph (mul of two inlets) driven by a push at the root:
//
//    push -> 6 -> [inlet -> mul <- inlet] -> add <- 42
//         -> 7 ---^                  ^------ 42 --|
//
// Asserts that the source map built from the emitted module resolves node
// fn defs, call sites and value bindings back to full node paths.
#[test]
fn source_map_roundtrip() {
    // The nested graph: mul of two inlets.
    let mut ga = Nested::default();
    let inlet_a = ga.add_node(Box::new(node::graph::Inlet) as Box<dyn DebugNode>);
    let inlet_b = ga.add_node(Box::new(node::graph::Inlet) as Box<_>);
    let mul = ga.add_node(Box::new(node_mul()) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet) as Box<_>);
    ga.add_edge(inlet_a, mul, Edge::from((0, 0)));
    ga.add_edge(inlet_b, mul, Edge::from((0, 1)));
    ga.add_edge(mul, outlet, Edge::from((0, 0)));

    // The root graph.
    let mut gb = petgraph::graph::DiGraph::new();
    let push = gb.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let six = gb.add_node(Box::new(node_int(6)) as Box<_>);
    let seven = gb.add_node(Box::new(node_int(7)) as Box<_>);
    let graph_a = gb.add_node(Box::new(ga) as Box<_>);
    let forty_two = gb.add_node(Box::new(node_int(42)) as Box<_>);
    let add = gb.add_node(Box::new(node_add()) as Box<_>);
    gb.add_edge(push, six, Edge::from((0, 0)));
    gb.add_edge(push, seven, Edge::from((0, 0)));
    gb.add_edge(push, forty_two, Edge::from((0, 0)));
    gb.add_edge(six, graph_a, Edge::from((0, 0)));
    gb.add_edge(seven, graph_a, Edge::from((0, 1)));
    gb.add_edge(graph_a, add, Edge::from((0, 0)));
    gb.add_edge(forty_two, add, Edge::from((0, 1)));

    let eps = push_pull_entrypoints(&no_lookup, &gb);
    let module = gantz_core::compile::module(&no_lookup, &gb, &eps, &Default::default()).unwrap();
    let src = gantz_core::vm::fmt_module(&module);
    let map = SourceMap::parse(&src);

    // One def per module expression, each a recognised emitted name.
    assert_eq!(map.defs().len(), module.len());
    for def in map.defs() {
        assert!(
            def.name.is_some(),
            "unrecognised define `{}`",
            &src[def.name_range.clone()]
        );
        assert!(src[def.range.clone()].starts_with("(define"));
    }
    // At least one def of each expected kind.
    let kind_count = |f: &dyn Fn(&Name) -> bool| {
        map.defs()
            .iter()
            .filter(|d| d.name.as_ref().is_some_and(f))
            .count()
    };
    assert!(kind_count(&|n| matches!(n, Name::NodeFn { .. })) >= 4);
    assert_eq!(kind_count(&|n| matches!(n, Name::GraphFn { .. })), 1);
    assert_eq!(kind_count(&|n| matches!(n, Name::EntryFn { .. })), 1);

    // The nested mul node: its node fn def, and its call + bindings inside
    // the graph fn, all resolve to its full path.
    let mul_path = vec![graph_a.index(), mul.index()];
    let mul_spans = map.node_spans(&mul_path);
    assert_eq!(mul_spans.defs.len(), 1, "one node fn def for mul");
    assert!(!mul_spans.refs.is_empty(), "mul referenced in its level");
    for range in mul_spans.defs.iter().chain(&mul_spans.refs) {
        assert_eq!(map.node_at(range.clone()), Some(mul_path.clone()));
    }
    // The def is the mul node fn and contains the node's expr.
    assert!(src[mul_spans.defs[0].clone()].starts_with("(define node-fn-"));
    assert!(src[mul_spans.defs[0].clone()].contains("*"));

    // The nested graph node itself owns two defs - its node fn wrapper and
    // the graph fn it calls - and is called from the entry fn.
    let ga_path = vec![graph_a.index()];
    let ga_spans = map.node_spans(&ga_path);
    assert_eq!(ga_spans.defs.len(), 2, "node fn wrapper + graph fn");
    assert!(!ga_spans.refs.is_empty(), "graph fn called from entry fn");
    for range in &ga_spans.refs {
        assert_eq!(map.node_at(range.clone()), Some(ga_path.clone()));
    }

    // A root node's bindings inside the entry fn resolve to its path.
    let add_path = vec![add.index()];
    let add_spans = map.node_spans(&add_path);
    assert_eq!(add_spans.defs.len(), 1);
    assert!(!add_spans.refs.is_empty());
    for range in &add_spans.refs {
        assert_eq!(map.node_at(range.clone()), Some(add_path.clone()));
    }
}

// A push *inside* the nested graph produces a per-entrypoint level fn whose
// def and call site both resolve to the nested graph node's path.
#[test]
fn source_map_level_fn() {
    let mut ga = Nested::default();
    let push = ga.add_node(Box::new(node_push()) as Box<dyn DebugNode>);
    let int = ga.add_node(Box::new(node_int(7)) as Box<_>);
    let outlet = ga.add_node(Box::new(node::graph::Outlet) as Box<_>);
    ga.add_edge(push, int, Edge::from((0, 0)));
    ga.add_edge(int, outlet, Edge::from((0, 0)));

    let mut gb = petgraph::graph::DiGraph::new();
    let graph_a = gb.add_node(Box::new(ga) as Box<dyn DebugNode>);
    let add = gb.add_node(Box::new(node_add()) as Box<_>);
    gb.add_edge(graph_a, add, Edge::from((0, 0)));

    let eps = push_pull_entrypoints(&no_lookup, &gb);
    let module = gantz_core::compile::module(&no_lookup, &gb, &eps, &Default::default()).unwrap();
    let src = gantz_core::vm::fmt_module(&module);
    let map = SourceMap::parse(&src);

    let ga_path = vec![graph_a.index()];
    let lvl_defs: Vec<_> = map
        .defs()
        .iter()
        .filter(|d| matches!(&d.name, Some(Name::LvlFn { path, .. }) if *path == ga_path))
        .collect();
    assert_eq!(lvl_defs.len(), 1, "one level fn for the nested push");
    let spans = map.node_spans(&ga_path);
    assert!(spans.defs.iter().any(|r| *r == lvl_defs[0].range));
    assert!(!spans.refs.is_empty(), "level fn called from entry fn");
    assert_eq!(
        map.node_at(lvl_defs[0].range.clone()),
        Some(ga_path.clone())
    );

    // The push node inside the nested level resolves through the level fn.
    let push_path = vec![graph_a.index(), push.index()];
    let int_path = vec![graph_a.index(), int.index()];
    for path in [&push_path, &int_path] {
        let spans = map.node_spans(path);
        assert!(
            !(spans.defs.is_empty() && spans.refs.is_empty()),
            "no spans for {path:?}"
        );
        for range in spans.defs.iter().chain(&spans.refs) {
            assert_eq!(map.node_at(range.clone()), Some(path.clone()));
        }
    }
}
