// Tests for the Fn and Apply nodes. They provide first-class functions in gantz.

use gantz_core::compile::{entry_fn_name, entrypoint, push_pull_entrypoints};
use gantz_core::node::{self, Apply, Fn, Node, Ref, WithPullEval, graph};
use gantz_core::{Edge, ROOT_STATE};
use std::collections::HashMap;
use std::fmt::Debug;
use steel::SteelVal;
use steel::steel_vm::engine::Engine;

fn node_bang() -> node::expr::Expr {
    node::expr("'bang").unwrap()
}

fn node_int(i: i32) -> node::expr::Expr {
    node::expr(format!("{}", i)).unwrap()
}

fn node_list_single() -> node::expr::Expr {
    node::expr("(list $x)").unwrap()
}

fn node_assert_eq() -> node::expr::Expr {
    node::expr("(assert! (equal? $l $r))").unwrap()
}

trait DebugNode: Debug + Node {}
impl<T> DebugNode for T where T: Debug + Node {}

// Fn wraps the identity function and Apply calls it.
//
//    --------
//    | bang |
//    -+------
//     |
//     |
//    -+------
//    |  fn  |  ------
//    | (id) |  | 42 |
//    -+------  -+----
//     |         |
//     |     -----
//     |     |
//    -+-----+-
//    | apply |
//    -+-------
//     |
//     |
//    ----------
//    | result |
//    ----------
#[test]
fn test_fn_apply_identity() {
    let mut g = petgraph::graph::DiGraph::new();

    let mut nodes: HashMap<gantz_ca::ContentAddr, Box<dyn DebugNode>> = HashMap::new();

    // Register the identity node under its erased content address. All
    // registry addresses use the data-layer scheme.
    let id = gantz_core::node::Identity;
    let id_ca = gantz_core::data::erase_node_typed(&id)
        .unwrap()
        .content_addr();
    nodes.insert(id_ca, Box::new(id) as Box<dyn DebugNode>);

    let get_node = |ca: &gantz_ca::ContentAddr| -> Option<&dyn Node> {
        nodes.get(ca).map(|b| &**b as &dyn Node)
    };

    let bang = node_bang();
    let fn_node = Fn::new(Ref::new(id_ca));
    let apply_node = Apply;
    let value = node_int(42);
    let list = node_list_single(); // Wrap value in list for apply
    let expected = node_int(42);
    let assert_eq = node_assert_eq().with_pull_eval();

    let bang = g.add_node(Box::new(bang) as Box<dyn DebugNode>);
    let fn_node = g.add_node(Box::new(fn_node) as Box<_>);
    let apply_node = g.add_node(Box::new(apply_node) as Box<_>);
    let value = g.add_node(Box::new(value) as Box<_>);
    let list = g.add_node(Box::new(list) as Box<_>);
    let expected = g.add_node(Box::new(expected) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);

    // The bang triggers fn to emit the lambda.
    g.add_edge(bang, fn_node, Edge::from((0, 0)));
    // The lambda goes to apply's function input.
    g.add_edge(fn_node, apply_node, Edge::from((0, 0)));
    g.add_edge(value, list, Edge::from((0, 0)));
    // The list goes to apply's argument input.
    g.add_edge(list, apply_node, Edge::from((0, 1)));
    g.add_edge(apply_node, assert_eq, Edge::from((0, 0)));
    g.add_edge(expected, assert_eq, Edge::from((0, 1)));

    let ctx = node::MetaCtx::new(&get_node);
    let eps = push_pull_entrypoints(&get_node, &g);
    let module = gantz_core::compile::module(&get_node, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&get_node, &g, &[], &mut vm);

    for expr in module {
        vm.run(expr.to_pretty(80)).unwrap();
    }

    let ep = entrypoint::pull(vec![assert_eq.index()], g[assert_eq].n_inputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}

// Fn wraps a graph node and Apply calls it. The NodeFns visitor must recurse
// into the graph through Fn's `visit` delegation to generate the nested node
// fns.
//
// The "double" graph feeds one inlet to both inputs of an add node and then
// to an outlet. So double(x) = x + x.
//
//    --------
//    | bang |
//    -+------
//     |
//     |
//    -+--------
//    |   fn   |  ------
//    | (dbl)  |  | 21 |
//    -+--------  -+----
//     |           |
//     |       -----
//     |       |
//    -+-------+-
//    |  apply  |
//    -+---------
//     |
//     |
//    ----------
//    | result |  (should be 42)
//    ----------
#[test]
fn test_fn_apply_graph() {
    let inlet_node = graph::Inlet::default();
    let add_node = node::expr("(+ $l $r)").unwrap();
    let outlet_node = graph::Outlet::default();

    // The registry address of the "double" graph comes from its erased
    // data-layer form.
    let mut double_data = gantz_ca::DataGraph::default();
    let d_inlet = double_data.add_node(gantz_core::data::erase_node_typed(&inlet_node).unwrap());
    let d_add = double_data.add_node(gantz_core::data::erase_node_typed(&add_node).unwrap());
    let d_outlet = double_data.add_node(gantz_core::data::erase_node_typed(&outlet_node).unwrap());
    double_data.add_edge(d_inlet, d_add, Edge::from((0, 0)));
    double_data.add_edge(d_inlet, d_add, Edge::from((0, 1)));
    double_data.add_edge(d_add, d_outlet, Edge::from((0, 0)));

    let mut double_graph = graph::Graph::<Box<dyn DebugNode>>::default();

    let inlet = double_graph.add_node(Box::new(inlet_node) as Box<dyn DebugNode>);
    let add = double_graph.add_node(Box::new(add_node) as Box<_>);
    let outlet = double_graph.add_node(Box::new(outlet_node) as Box<_>);

    double_graph.add_edge(inlet, add, Edge::from((0, 0)));
    double_graph.add_edge(inlet, add, Edge::from((0, 1)));
    double_graph.add_edge(add, outlet, Edge::from((0, 0)));

    // The nested "double" graph is referenced by content address. A bare
    // `Graph` implements `Node`, so the `get_node` lookup returns it directly.
    let double_ca: gantz_ca::ContentAddr = gantz_ca::graph_addr(&double_data).into();
    let get_node = |ca: &gantz_ca::ContentAddr| -> Option<&dyn Node> {
        (*ca == double_ca).then_some(&double_graph as &dyn Node)
    };

    // The main graph wraps the double graph in `Fn<Ref>`.
    let mut g = petgraph::graph::DiGraph::new();

    let bang = node_bang();
    let fn_node = Fn::new(Ref::new(double_ca));
    let apply_node = Apply;
    let value = node_int(21);
    let list = node_list_single();
    let expected = node_int(42);
    let assert_eq = node_assert_eq().with_pull_eval();

    let bang = g.add_node(Box::new(bang) as Box<dyn DebugNode>);
    let fn_node = g.add_node(Box::new(fn_node) as Box<_>);
    let apply_node = g.add_node(Box::new(apply_node) as Box<_>);
    let value = g.add_node(Box::new(value) as Box<_>);
    let list = g.add_node(Box::new(list) as Box<_>);
    let expected = g.add_node(Box::new(expected) as Box<_>);
    let assert_eq = g.add_node(Box::new(assert_eq) as Box<_>);

    // The bang triggers fn to emit the lambda.
    g.add_edge(bang, fn_node, Edge::from((0, 0)));
    // The lambda goes to apply's function input.
    g.add_edge(fn_node, apply_node, Edge::from((0, 0)));
    g.add_edge(value, list, Edge::from((0, 0)));
    // The list goes to apply's argument input.
    g.add_edge(list, apply_node, Edge::from((0, 1)));
    g.add_edge(apply_node, assert_eq, Edge::from((0, 0)));
    g.add_edge(expected, assert_eq, Edge::from((0, 1)));

    let ctx = node::MetaCtx::new(&get_node);
    let eps = push_pull_entrypoints(&get_node, &g);
    let module = gantz_core::compile::module(&get_node, &g, &eps, &Default::default()).unwrap();

    let mut vm = Engine::new_base();
    vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(&get_node, &g, &[], &mut vm);

    for expr in module {
        vm.run(expr.to_pretty(80)).unwrap();
    }

    let ep = entrypoint::pull(vec![assert_eq.index()], g[assert_eq].n_inputs(ctx) as u8);
    vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
        .unwrap();
}
