//! The content-addressing abstractions for `gantz`.

#[doc(inline)]
pub use ca::{ContentAddr, ContentAddrShort, content_addr};
#[doc(inline)]
pub use commit::{Branch, Commit, CommitAddr, Head, Timestamp, addr as commit_addr};
#[doc(inline)]
pub use diff::{Diff, DiffSummary, Matching};
/// Re-export the derive macro.
pub use gantz_ca_derive::CaHash;
#[doc(inline)]
pub use graph::{
    GraphAddr, addr as graph_addr, addr_with_nodes as graph_addr_with_nodes, hash_graph,
    hash_graph_with_nodes, node_addrs,
};
#[doc(inline)]
pub use hash::{CaHash, Hasher};
#[doc(inline)]
pub use history::{MergeAnalysis, analyze, ancestors, first_parent_chain, merge_base};
#[doc(inline)]
pub use merge::{
    BothModified, Conflict, EditOrDelete, MergeError, MergeOutcome, MergeResolution, NodeSrc,
    Resolutions, Side, merge_commits,
};
#[doc(inline)]
pub use registry::Registry;

mod ca;
mod commit;
pub mod diff;
mod graph;
mod hash;
pub mod history;
pub mod merge;
pub mod registry;
pub mod serde_sorted;
