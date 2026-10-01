//! A minimal node set for the format and export round-trip paths in unit
//! tests. It has `Expr` and `Bang` leaves plus `NamedRef` for graph
//! references.

use crate::node::NamedRef;
use dyn_clone::DynClone;
use gantz_core::node::graph::Graph;
use std::any::Any;

pub trait TestNode: Any + DynClone + gantz_core::Node {}

pub type TestGraph = Graph<Box<dyn TestNode>>;

dyn_clone::clone_trait_object!(TestNode);

impl TestNode for gantz_core::node::Expr {}
impl TestNode for crate::node::Bang {}
impl TestNode for NamedRef {}
impl TestNode for Box<dyn TestNode> {}

gantz_format::impl_node_set_serde! {
    dyn TestNode {
        gantz_core::node::Expr,
        crate::node::Bang,
        crate::node::NamedRef,
    }
}

impl gantz_format::NodeSugar for Box<dyn TestNode> {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![&gantz_format::CoreSugar])
    }
}

/// The value-level codec for the test node set. It must list the same
/// manifest as the `impl_node_set_serde!` invocation above.
pub fn codec() -> crate::node::NodeCodec {
    crate::ui_node_codec! {
        Box<dyn TestNode> {
            gantz_core::node::Expr,
            crate::node::Bang,
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

/// Erase `graph` and commit it under `name`. Returns the new commit and the
/// erased graph's address. The address is the registry's identity for the
/// graph.
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

/// `node_data` as a newer gantz might store it, with a `future` field that
/// this build does not recognise.
pub fn with_unknown_field(mut node_data: gantz_ca::NodeData) -> gantz_ca::NodeData {
    let gantz_ca::Datum::Map(fields) = &mut node_data.data else {
        panic!("node data is a map");
    };
    fields.push(("future".to_string(), gantz_ca::Datum::Bool(true)));
    node_data.canonicalize();
    node_data
}

/// Whether `node_data` still holds the field that [`with_unknown_field`] adds.
pub fn has_unknown_field(node_data: &gantz_ca::NodeData) -> bool {
    node_data.data.get("future").is_some()
}
