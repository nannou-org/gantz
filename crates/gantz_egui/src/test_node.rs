//! A minimal node set for exercising the format/export round-trip paths in
//! unit tests: an `Expr` leaf plus `NamedRef` for graph references.

use crate::node::NamedRef;
use dyn_clone::DynClone;
use gantz_core::node::graph::Graph;
use std::any::Any;

pub trait TestNode: Any + DynClone + gantz_core::Node {}

pub type TestGraph = Graph<Box<dyn TestNode>>;

dyn_clone::clone_trait_object!(TestNode);

impl TestNode for gantz_core::node::Expr {}
impl TestNode for NamedRef {}
impl TestNode for Box<dyn TestNode> {}

gantz_format::impl_node_set_serde! {
    dyn TestNode {
        gantz_core::node::Expr,
        crate::node::NamedRef,
    }
}

impl gantz_format::NodeSugar for Box<dyn TestNode> {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![&gantz_format::CoreSugar])
    }
}

/// The value-level codec for the test node set: the SAME manifest as the
/// `impl_node_set_serde!` invocation above.
pub fn codec() -> crate::node::NodeCodec {
    crate::ui_node_codec! {
        Box<dyn TestNode> {
            gantz_core::node::Expr,
            crate::node::NamedRef,
        }
    }
}

pub fn expr(src: &str) -> Box<dyn TestNode> {
    Box::new(gantz_core::node::Expr::new(src).unwrap())
}

pub fn named_ref(name: &str, graph_ca: gantz_ca::GraphAddr) -> Box<dyn TestNode> {
    let ref_ = gantz_core::node::Ref::new(graph_ca.into());
    Box::new(NamedRef::new(name.parse().unwrap(), ref_))
}

/// Erase `graph` and commit it under `name`, returning the new commit and the
/// erased graph's address (the registry's identity for the graph).
pub fn commit_named(
    reg: &mut gantz_ca::Registry,
    timestamp: std::time::Duration,
    graph: &TestGraph,
    name: &gantz_ca::Name,
) -> (gantz_ca::CommitAddr, gantz_ca::GraphAddr) {
    let (dg, ga) = gantz_core::data::erase_with_addr(graph).unwrap();
    let ca = reg.commit_graph_to_name(timestamp, ga, || dg, name);
    (ca, ga)
}
