use crate::node;

/// The type used to represent node and edge indices.
pub type Index = usize;

pub type EdgeIndex = petgraph::graph::EdgeIndex<Index>;
pub type NodeIndex = petgraph::graph::NodeIndex<Index>;

/// Describes a connection between two nodes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Edge {
    /// The output of the node at the source of this edge.
    pub output: node::Output,
    /// The input of the node at the destination of this edge.
    pub input: node::Input,
}

/// The petgraph type used to represent the **Graph**.
pub type Petgraph<N> = petgraph::stable_graph::StableGraph<N, Edge, petgraph::Directed, Index>;

/// The graph type used to represent evaluation.
#[derive(Debug)]
pub struct Graph<N> {
    graph: Petgraph<N>,
}

pub mod codegen {
    use crate::node::{self, Node};
    use super::{Graph, NodeIndex};

    /// An evaluation step ready for translation to rust code.
    pub struct EvalStep {
        /// The node to be evaluated.
        pub node: NodeIndex,
        /// Arguments to the node's function call.
        pub args: Vec<FnCallArg>,
    }

    /// An argument ot a node's function call.
    pub struct FnCallArg {
        /// The node from which the value was generated.
        pub node: NodeIndex,
        /// The outlet on the source node associated with the generated value.
        pub outlet: node::Output,
        /// Whether or not using the value in this argument requires cloning.
        pub requires_clone: bool,
    }

    impl<N> Graph<N>
    where
        N: Node,
    {
        /// Push evaluation steps starting from the specified outlets on the given node.
        /// 
        /// 1. For each outlet on the given node, evaluate the output and use it to evaluate the
        ///    input for each child inlet connected to this outlet.
        /// 2. Repeat this for each child in BFS order.
        pub fn push_eval_steps(&self, node: NodeIndex) -> Vec<EvalStep> {
            // let mut bfs = petgraph::visit::Bfs::new(&self.graph, node);
            // while let Some(parent) = bfs.next(&self.graph) {
            //     let n_outlets = self.graph[parent].n_outlets();
            //     for outlet in (0..n_outlets).map(node::Outlet) {
            //         // For every outgoing neighbour of the parent, if it has an input that is
            //         // connected to this outlet, process it.
            //         self.graph[parent].state.proc_outlet_at_index(outlet);
            //         let mut children = self.graph.neighbors(parent).detach();
            //         while let Some((e, child)) = children.next(&self.graph) {
            //             let edge = self.graph[e];
            //             if edge.outlet == outlet {
            //                 let (parent, child) = self.graph.index_twice_mut(parent, child);
            //                 child.proc_inlet_at_index(edge.inlet, parent.outlet_ref(outlet));
            //             }
            //         }
            //     }
            // }
            unimplemented!();
        }

        /// Pull evaluation steps starting from the specified outlets on the given node.
        ///
        /// This causes the graph to performa DFS through each of the inlets to find the deepest
        /// nodes. Evaluation occurs as though a "push" was sent simultaneously to each of the
        /// deepest nodes. Outlets are only calculated once per node. If an outlet value is
        /// required more than once, it will be borrowed accordingly.
        pub fn pull_eval_steps(&self, node: NodeIndex) -> Vec<EvalStep> {
            unimplemented!();
        }
    }
}
