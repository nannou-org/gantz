use crate::node::Node;

pub type Graph = gantz_core::node::graph::Graph<Box<dyn Node>>;
pub type GraphNode = gantz_core::node::GraphNode<Box<dyn Node>>;

/// Short-hand for using `dyn-clone` to clone the graph.
pub fn clone(graph: &Graph) -> Graph {
    graph.map(|_, n| dyn_clone::clone_box(&**n), |_, e| e.clone())
}
