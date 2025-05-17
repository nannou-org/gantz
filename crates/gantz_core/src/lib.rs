pub use node::Node;
pub use steel;
use serde::{Deserialize, Serialize};

/// The ident used to represent the root state.
/// This is the state of the top-level graph.
pub const ROOT_STATE: &str = "__root_state";
/// The ident used to represent the state of a graph.
/// Note that this can be either nested or top-level.
pub const GRAPH_STATE: &str = "__graph_state";

pub mod codegen;
pub mod graph;
pub mod node;

/// Describes a connection between two nodes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Edge {
    /// The output of the node at the source of this edge.
    pub output: node::Output,
    /// The input of the node at the destination of this edge.
    pub input: node::Input,
}

impl Edge {
    /// Create an edge representing a connection from the given node `Output` to
    /// the given node `Input`.
    pub fn new(output: node::Output, input: node::Input) -> Self {
        Edge { output, input }
    }
}

impl<A, B> From<(A, B)> for Edge
where
    A: Into<node::Output>,
    B: Into<node::Input>,
{
    fn from((a, b): (A, B)) -> Self {
        let output = a.into();
        let input = b.into();
        Edge { output, input }
    }
}
