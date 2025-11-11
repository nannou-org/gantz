//! Extends [`gantz_core::ca`] with more content-addressing abstractions.
//!
//! Provides abstractions like [`Branch`], [`Commit`] and [`Head`], required for
//! storing and navigating graphs and their history.

#[doc(inline)]
pub use commit::Commit;
/// Re-export the content addressing module from `gantz_core`.
pub use gantz_core::ca::*;
use gantz_core::compile::Edges;
use petgraph::visit::{Data, IntoEdgeReferences, IntoNodeReferences, NodeIndexable};
use serde::{Deserialize, Serialize};
use std::{fmt, hash::Hash, ops};

pub mod commit;

/// The content address of a commit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct CommitAddr(ContentAddr);

/// The content address of a graph.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct GraphAddr(ContentAddr);

/// Represents a name used to track a working series of commits.
///
/// Users can think of this as their graph or project name.
pub type Branch = String;

/// Acts as a pointer to the current working graph, whether directly to a commit
/// or to a name mapping.
#[derive(Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum Head {
    /// The head is pointing directly to a commit.
    Commit(CommitAddr),
    /// The head is pointing to a name which maps to a commit.
    Branch(Branch),
}

impl ops::Deref for CommitAddr {
    type Target = ContentAddr;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::Deref for GraphAddr {
    type Target = ContentAddr;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<ContentAddr> for CommitAddr {
    fn from(ca: ContentAddr) -> Self {
        Self(ca)
    }
}

impl From<ContentAddr> for GraphAddr {
    fn from(ca: ContentAddr) -> Self {
        Self(ca)
    }
}

impl From<CommitAddr> for ContentAddr {
    fn from(addr: CommitAddr) -> Self {
        addr.0
    }
}

impl From<GraphAddr> for ContentAddr {
    fn from(addr: GraphAddr) -> Self {
        addr.0
    }
}

impl CaHash for CommitAddr {
    fn hash(&self, hasher: &mut Hasher) {
        CaHash::hash(&self.0, hasher);
    }
}

impl CaHash for GraphAddr {
    fn hash(&self, hasher: &mut Hasher) {
        CaHash::hash(&self.0, hasher);
    }
}

impl fmt::Display for CommitAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for GraphAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Shorthand for producing a commit's [`CommitAddr`].
pub fn commit_addr(commit: &Commit) -> CommitAddr {
    CommitAddr(content_addr(commit))
}

/// Shorthand wrapper around [`gantz_core::ca::graph`] that produces the
/// type-safe `GraphAddr`.
pub fn graph_addr<G>(g: G) -> GraphAddr
where
    G: Data + IntoEdgeReferences + IntoNodeReferences + NodeIndexable,
    G::NodeId: Eq + Hash + Ord,
    G::EdgeWeight: Edges,
    G::NodeWeight: CaHash,
{
    GraphAddr(gantz_core::ca::graph(g))
}
