//! The content-addressing abstractions for `gantz`.

#[doc(inline)]
pub use ca::{ContentAddr, ContentAddrShort, content_addr};
#[doc(inline)]
pub use commit::{Branch, Commit, CommitAddr, Head, Timestamp, addr as commit_addr};
#[doc(inline)]
pub use graph::{
    GraphAddr, addr as graph_addr, addr_with_nodes as graph_addr_with_nodes, hash_graph,
    hash_graph_with_nodes, node_addrs,
};
#[doc(inline)]
pub use hash::{CaHash, Hasher};
#[doc(inline)]
pub use registry::Registry;

mod ca;
mod commit;
mod graph;
mod hash;
pub mod registry;
