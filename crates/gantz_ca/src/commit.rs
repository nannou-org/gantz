use super::{CaHash, ContentAddr, GraphAddr, Hasher, Name};
use serde::{Deserialize, Serialize};
use std::{fmt, ops, time::Duration};

/// A commit captures a snapshot of a `Graph`.
#[derive(Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Commit {
    /// The timestamp of the commit, represented as the duration since
    /// `UNIX_EPOCH`.
    pub timestamp: Timestamp,
    /// The first parent of this commit. This is the commit the head was on
    /// when the commit was made.
    pub parent: Option<CommitAddr>,
    /// The address of the graph pointed to by this commit.
    pub graph: GraphAddr,
    /// Extra parents, the merged-in tips. Present only on merge commits.
    ///
    /// Empty on ordinary commits. An empty list is omitted from both the
    /// content hash and the serialized form.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merge_parents: Vec<CommitAddr>,
}

/// The timestamp of a commit, represented as the duration since `UNIX_EPOCH`.
pub type Timestamp = std::time::Duration;

/// The content address of a commit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct CommitAddr(ContentAddr);

/// Represents a name used to track a working series of commits.
///
/// Users can think of this as their graph or project name.
pub type Branch = Name;

/// Acts as a pointer to the current working graph, whether directly to a commit
/// or to a name mapping.
#[derive(Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum Head {
    /// The head is pointing directly to a commit.
    Commit(CommitAddr),
    /// The head is pointing to a name which maps to a commit.
    Branch(Branch),
}

impl std::fmt::Display for Head {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Head::Branch(name) => write!(f, "{name}"),
            Head::Commit(ca) => write!(f, "{}", ca.display_short()),
        }
    }
}

impl Commit {
    /// Create a new commit with the given timestamp, parent and graph.
    pub fn new(timestamp: Duration, parent: Option<CommitAddr>, graph: GraphAddr) -> Self {
        Self {
            timestamp,
            parent,
            graph,
            merge_parents: vec![],
        }
    }

    /// Create a merge commit joining `theirs` into `ours`.
    ///
    /// `ours` becomes the first parent and `theirs` the merge parent.
    pub fn new_merge(
        timestamp: Duration,
        ours: CommitAddr,
        theirs: CommitAddr,
        graph: GraphAddr,
    ) -> Self {
        Self {
            timestamp,
            parent: Some(ours),
            graph,
            merge_parents: vec![theirs],
        }
    }

    /// All parents of this commit. The first parent, if any, then the merge
    /// parents.
    pub fn parents(&self) -> impl Iterator<Item = CommitAddr> + '_ {
        self.parent
            .into_iter()
            .chain(self.merge_parents.iter().copied())
    }
}

/// Commits are addressed by timestamp, parents and graph CA.
///
/// The `gantz.commit` prefix domain-separates commit addresses from every
/// other kind. Graphs carry their own prefix. Blob addresses are the raw
/// blake3 of their bytes. No two kinds can collide on one address.
impl CaHash for Commit {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(b"gantz.commit");
        self.timestamp.as_secs().hash(hasher);
        self.timestamp.subsec_nanos().hash(hasher);
        self.parent.hash(hasher);
        self.graph.hash(hasher);
        // Folded in only when non-empty, so the field does not change the
        // address of an ordinary commit.
        if !self.merge_parents.is_empty() {
            hasher.update(b"merge-parents");
            self.merge_parents.hash(hasher);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_addr(n: u8) -> GraphAddr {
        GraphAddr::from(ContentAddr::from([n; 32]))
    }

    fn commit_addr_raw(n: u8) -> CommitAddr {
        CommitAddr::from(ContentAddr::from([n; 32]))
    }

    #[test]
    fn commit_addr_is_domain_separated() {
        // The kind prefix keeps a commit's address distinct from a raw blake3
        // hash of the same field bytes, which is how blobs are addressed.
        let root = Commit::new(Duration::from_secs(1), None, graph_addr(1));
        let mut unprefixed = Hasher::new();
        root.timestamp.as_secs().hash(&mut unprefixed);
        root.timestamp.subsec_nanos().hash(&mut unprefixed);
        root.parent.hash(&mut unprefixed);
        root.graph.hash(&mut unprefixed);
        let bytes: [u8; 32] = unprefixed.finalize().into();
        assert_ne!(addr(&root), CommitAddr::from(ContentAddr::from(bytes)));
    }

    #[test]
    fn merge_commit_addr_differs_from_ordinary() {
        let ours = commit_addr_raw(10);
        let theirs = commit_addr_raw(20);
        let merge = Commit::new_merge(Duration::from_secs(3), ours, theirs, graph_addr(1));
        let ordinary = Commit::new(Duration::from_secs(3), Some(ours), graph_addr(1));
        assert_ne!(addr(&merge), addr(&ordinary));
    }

    #[test]
    fn parents_yields_first_parent_then_merge_parents() {
        let ours = commit_addr_raw(10);
        let theirs = commit_addr_raw(20);
        let merge = Commit::new_merge(Duration::from_secs(3), ours, theirs, graph_addr(1));
        assert_eq!(merge.parents().collect::<Vec<_>>(), vec![ours, theirs]);
        let root = Commit::new(Duration::from_secs(1), None, graph_addr(1));
        assert_eq!(root.parents().count(), 0);
    }

    #[test]
    fn legacy_commit_ron_deserializes() {
        // A commit serialized before `merge_parents` existed.
        let commit = Commit::new(Duration::from_secs(1), None, graph_addr(1));
        let legacy = {
            #[derive(serde::Serialize)]
            struct OldCommit {
                timestamp: Timestamp,
                parent: Option<CommitAddr>,
                graph: GraphAddr,
            }
            ron::to_string(&OldCommit {
                timestamp: commit.timestamp,
                parent: commit.parent,
                graph: commit.graph,
            })
            .unwrap()
        };
        let de: Commit = ron::de::from_str(&legacy).unwrap();
        assert_eq!(de, commit);
    }

    #[test]
    fn ordinary_commit_ron_omits_merge_parents() {
        let commit = Commit::new(Duration::from_secs(1), None, graph_addr(1));
        let s = ron::to_string(&commit).unwrap();
        assert!(!s.contains("merge_parents"));
        // A merge commit round-trips its merge parents.
        let merge = Commit::new_merge(
            Duration::from_secs(2),
            commit_addr_raw(10),
            commit_addr_raw(20),
            graph_addr(1),
        );
        let s = ron::to_string(&merge).unwrap();
        assert!(s.contains("merge_parents"));
        let de: Commit = ron::de::from_str(&s).unwrap();
        assert_eq!(de, merge);
    }
}
