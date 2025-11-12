use super::{CaHash, CommitAddr, GraphAddr, Hasher};
use serde::{Deserialize, Serialize};

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

impl Commit {
    /// Create a new commit, generating the timestamp using the system clock.
    pub fn timestamped(parent: Option<CommitAddr>, graph: GraphAddr) -> Self {
        let now = web_time::SystemTime::now();
        let timestamp = now
            .duration_since(web_time::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::ZERO);
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
