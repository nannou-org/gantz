//! **Gantz** is a programming execution representation.
//!
//! **Gantz** uses a directed graph for this representation. **Node**s represent expressions, while
//! the edges between nodes define the order of evaluation for each of these expressions.
//!
//! - **Inlet**s of a node describe the inputs to the expression.
//! - **Outlet**s of a node describe the outputs of the evaluated expression.
//!
//! **Gantz** allows for triggering evaluation of the graph in two ways:
//!
//! 1. **Push evaluation**. The graph allows for "pushing" evaluation from one or more outlets of a
//!    single node. This causes the "pushed" outlets to begin evaluation in visit-order of a
//!    breadth-first-search that ends when nodes are reached that either 1. only have outlets
//!    connecting to nodes that have already been evaluated or 2. have no outlets at all.
//!
//! 2. **Pull evaluation**. The graph allows for "pulling" evaluation from one or more inlets of a
//!    single node. This causes the "pulled" inlets to perform a depth-first search in order to
//!    find all connected nodes that either 1. Have no inlets or 2. have inlets that connect to
//!    already visited nodes. Once these "starting" nodes are found, evaluation is "pushed" from
//!    each of these nodes in the order in which they were visited.
//!
//! ## Current Questions

#[macro_use] extern crate gantz_derive;
extern crate petgraph;

pub mod node;

pub use node::Node;

/// The edge connecting two nodes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    outlet: node::Outlet,
    inlet: node::Inlet,
}

/// The index type used within the graph to uniquely identify a node or edge.
pub type Index = usize;

/// The **petgraph** graph data structure used to represent the execution graph.
pub type PetGraph = petgraph::stable_graph::StableGraph<Node, Edge, petgraph::Directed, Index>;

/// The graph type
pub struct Graph {
    graph: PetGraph,
}

impl Graph {
    /// Push evaluation of the graph from the specified outlets on the given node.
    /// 
    /// 1. For each outlet on the given node, evaluate the output and use it to evaluate the input
    ///    for each child inlet connected to this outlet.
    /// 2. Repeat this for each child in BFS order.
    pub fn push_evaluation(&mut self, node: node::Index) {
        let mut bfs = petgraph::visit::Bfs::new(&self.graph, node);
        while let Some(parent) = bfs.next(&self.graph) {
            let n_outlets = self.graph[parent].n_outlets();
            for outlet in (0..n_outlets).map(node::Outlet) {
                // For every outgoing neighbour of the parent, if it has an input that is connected
                // to this outlet, process it.
                self.graph[parent].state.proc_outlet_at_index(outlet);
                let mut children = self.graph.neighbors(parent).detach();
                while let Some((e, child)) = children.next(&self.graph) {
                    let edge = self.graph[e];
                    if edge.outlet == outlet {
                        let (parent, child) = self.graph.index_twice_mut(parent, child);
                        child.proc_inlet_at_index(edge.inlet, parent.outlet_ref(outlet));
                    }
                }
            }
        }
    }

    /// Pull evaluation of the graph from the specified outlets on the given node.
    ///
    /// This causes the graph to performa DFS through each of the inlets to find the deepest nodes.
    /// Evaluation occurs as though a "push" was sent simultaneously to each of the deepest nodes.
    /// Outlets are only calculated once per node. If an outlet value is required more than once,
    /// it will be borrowed accordingly.
    pub fn pull_evaluation(&mut self, _node: node::Index, _inlets: &[node::Inlet]) {
        unimplemented!();
    }
}
