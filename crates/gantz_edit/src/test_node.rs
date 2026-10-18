//! A minimal node set for the format and export round-trip paths in unit
//! tests. It has `Expr` leaves plus `NamedRef` for graph references. Tests
//! build graphs in their stored data form.

use crate::NamedRef;
use crate::codec::DataCodec;
use gantz_ca::{DataGraph, NodeData};

/// The node set's sugar carrier. See [`codec`].
pub struct TestNodes;

pub type TestGraph = DataGraph;

impl gantz_format::NodeSugar for TestNodes {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![&gantz_format::CoreSugar])
    }
}

/// The data codec for the test node set.
pub fn codec() -> DataCodec {
    let codec: crate::codec::NodeCodec<Box<dyn gantz_core::Node>> = crate::node_codec! {
        Box<dyn gantz_core::Node>, |node| {
            let node: &dyn gantz_core::Node = &**node;
            node as &dyn std::any::Any
        };
        TestNodes {
            gantz_core::node::Expr,
            NamedRef,
        }
    };
    *codec.data()
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
