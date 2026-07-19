pub use builtin::{Builtin, Builtins};
pub use diagnostic::Diagnostic;
pub use edge::Edge;
pub use gantz_ca::datum;
pub use node::Node;
pub use steel;

pub mod args;
pub mod builtin;
pub mod compile;
pub mod data;
pub mod diagnostic;
pub mod edge;
pub mod graph;
pub mod node;
pub mod visit;
pub mod vm;

/// The ident used to represent the root state.
/// This is the state of the top-level graph.
pub const ROOT_STATE: &str = "%root-state";
/// The ident used to represent the entrypoint [`args`] map - a read-only set of
/// per-evaluation inputs (e.g. the firing `time`) the caller sets before invoking
/// an entry fn, readable by any node's `expr`.
pub const ARGS: &str = "%args";
/// The ident used to represent the state of a graph.
/// Note that this can be either nested or top-level.
const GRAPH_STATE: &str = "graph-state";
