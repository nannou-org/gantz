use super::{CaHash, ContentAddr, GraphAddr, Hasher};
use serde::{Deserialize, Serialize};
use std::{fmt, ops, time::Duration};

/// A commit captures a snapshot of a `Graph`.
#[derive(Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Commit {
    /// The timestamp of the commit, represented as the duration since
    /// `UNIX_EPOCH`.
    pub timestamp: Timestamp,
    /// The parent of this commit.
    pub parent: Option<CommitAddr>,
    /// The address of the graph pointed to by this commit.
    pub graph: GraphAddr,
}

/// The timestamp of a commit, represented as the duration since `UNIX_EPOCH`.
pub type Timestamp = std::time::Duration;

/// The content address of a commit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct CommitAddr(ContentAddr);

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

impl Commit {
    /// Create a new commit with the given timestamp, parent and graph.
    pub fn new(timestamp: Duration, parent: Option<CommitAddr>, graph: GraphAddr) -> Self {
        Self {
            timestamp,
            parent,
            graph,
        }
    }
}

/// Commits addressed by timestamp, parent and graph CA.
impl CaHash for Commit {
    fn hash(&self, hasher: &mut Hasher) {
        self.timestamp.as_secs().hash(hasher);
        self.timestamp.subsec_nanos().hash(hasher);
        self.parent.hash(hasher);
        self.graph.hash(hasher);
    }
}

impl ops::Deref for CommitAddr {
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

impl From<CommitAddr> for ContentAddr {
    fn from(addr: CommitAddr) -> Self {
        addr.0
    }
}

impl CaHash for CommitAddr {
    fn hash(&self, hasher: &mut Hasher) {
        CaHash::hash(&self.0, hasher);
    }
}

impl fmt::Display for CommitAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Shorthand for producing the address of a [`Commit`].
pub fn addr(commit: &Commit) -> CommitAddr {
    CommitAddr(crate::content_addr(commit))
}
