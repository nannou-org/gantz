//! The content-addressing abstractions for `gantz`.

// The `CaHash` derive emits `gantz_ca::` paths. This alias lets the crate use
// the derive on its own types.
extern crate self as gantz_ca;

#[doc(inline)]
pub use ca::{ContentAddr, ContentAddrShort, content_addr};
#[doc(inline)]
pub use commit::{Branch, Commit, CommitAddr, Head, Timestamp, addr as commit_addr};
#[doc(inline)]
pub use datum::{Datum, DatumError, from_datum, to_datum};
#[doc(inline)]
pub use diff::{Diff, DiffSummary, Matching};
#[doc(inline)]
pub use edge::{Edge, Input, Output};
/// Re-export the derive macro.
pub use gantz_ca_derive::CaHash;
#[doc(inline)]
pub use graph::{GraphAddr, addr as graph_addr};
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
pub use name::{Name, SEP as NAME_SEP};
#[doc(inline)]
pub use node_data::{DataGraph, NodeData};
#[doc(inline)]
pub use reach::{LiveSet, OutRefs, closure, closure_from, data_graph_out, export, prune};
#[doc(inline)]
pub use registry::{
    HEADS_ID, Heads, MergeReport, Registry, section_get, section_insert, section_insert_datum,
    section_iter, section_remove,
};
#[doc(inline)]
pub use section::{
    BlobDecl, BlobLiveness, BlobStore, Bytes, Key, Liveness, MergePolicy, Section, SectionDecl,
    SectionId, Value, blob_addr,
};
#[doc(inline)]
pub use sync::{SyncStep, monotonic_timestamp, plan_sync_step, verify_graph};

mod ca;
mod commit;
pub mod datum;
pub mod diff;
pub mod edge;
mod graph;
mod hash;
pub mod history;
pub mod merge;
pub mod name;
pub mod node_data;
pub mod ops;
pub mod reach;
pub mod registry;
pub mod section;
pub mod serde_sorted;
pub mod sync;
