//! A minimal node set for the format and export round-trip paths in unit
//! tests. It has `Expr` leaves plus `NamedRef` for graph references. Tests
//! build graphs in their stored data form.

use crate::NamedRef;
use crate::codec::{DataCodec, Kind, NodeCodec};
use gantz_ca::{DataGraph, NodeData};
use std::any::Any;

pub type TestGraph = DataGraph;

/// The data codec for the test node set.
pub fn codec() -> DataCodec {
    type Node = Box<dyn gantz_core::Node>;
    let as_any: fn(&Node) -> &dyn Any = |node| &**node as &dyn Any;
    let sugars = gantz_format::Sugars::default().with(gantz_format::CoreSugar);
    let codec: NodeCodec<Node> = NodeCodec::new(
        sugars,
        [
            Kind::typed::<gantz_core::node::Expr>(|node| Box::new(node), as_any),
            Kind::typed::<NamedRef>(|node| Box::new(node), as_any),
        ],
    );
    codec.data().clone()
}

pub fn expr(src: &str) -> NodeData {
    let expr = gantz_core::node::Expr::new(src).unwrap();
    gantz_core::data::erase_node_typed(&expr).unwrap()
}

pub fn named_ref(name: &str, graph_ca: gantz_ca::GraphAddr) -> NodeData {
    let ref_ = gantz_core::node::Ref::new(graph_ca.into());
    gantz_core::data::erase_node_typed(&NamedRef::new(name.parse().unwrap(), ref_)).unwrap()
}

/// Commit `graph` under `name`. Returns the new commit and the graph's
/// address.
pub fn commit_named(
    reg: &mut gantz_ca::Registry,
    timestamp: std::time::Duration,
    graph: &TestGraph,
    name: &gantz_ca::Name,
) -> (gantz_ca::CommitAddr, gantz_ca::GraphAddr) {
    let ga = gantz_ca::graph_addr(graph);
    let ca = reg.commit_graph_to_name(timestamp, ga, || graph.clone(), name);
    (ca, ga)
}
