//! A minimal node set for the format and export round-trip paths in unit
//! tests. It has `Expr` and `Bang` leaves plus `NamedRef` for graph
//! references. Tests build graphs in their stored data form.

use gantz_ca::{DataGraph, NodeData};
use gantz_edit::NamedRef;

/// The node set's sugar carrier. See [`codec`].
pub struct TestNodes;

pub type TestGraph = DataGraph;

impl gantz_format::NodeSugar for TestNodes {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![&gantz_format::CoreSugar])
    }
}

/// The value-level codec for the test node set.
pub fn codec() -> crate::node::NodeCodec {
    crate::ui_node_codec! {
        TestNodes {
            gantz_core::node::Expr,
            crate::node::Bang,
            NamedRef,
        }
    }
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

/// `node_data` as a newer gantz might store it, with a `future` field that
/// this build does not recognise.
pub fn with_unknown_field(mut node_data: NodeData) -> NodeData {
    let gantz_ca::Datum::Map(fields) = &mut node_data.data else {
        panic!("node data is a map");
    };
    fields.push(("future".to_string(), gantz_ca::Datum::Bool(true)));
    node_data.canonicalize();
    node_data
}

/// Whether `node_data` still holds the field that [`with_unknown_field`] adds.
pub fn has_unknown_field(node_data: &NodeData) -> bool {
    node_data.data.get("future").is_some()
}
