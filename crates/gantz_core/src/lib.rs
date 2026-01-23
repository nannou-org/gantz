pub use edge::Edge;
pub use node::Node;
pub use steel;

pub mod compile;
pub mod edge;
pub mod graph;
pub mod node;
pub mod reg;
pub mod visit;

/// The ident used to represent the root state.
/// This is the state of the top-level graph.
pub const ROOT_STATE: &str = "%root-state";
/// The ident used to represent the state of a graph.
/// Note that this can be either nested or top-level.
const GRAPH_STATE: &str = "graph-state";
