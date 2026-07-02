//! Convergence primitives for synchronising commit DAGs between peers.
//!
//! Peers collaborating on a shared graph exchange commits and independently
//! decide how to bring their local tip up to date with a remote one. For all
//! peers to *converge* - identical tip [`CommitAddr`]s with no further
//! exchange - every decision here is a pure, side-independent function of
//! commit content:
//!
//! - [`plan_sync_step`] classifies a `(local, remote)` tip pair. Diverged
//!   tips whose commits point at the *same* graph are resolved by
//!   deterministic adoption ([`SyncStep::Adopt`]): no merge commit is minted
//!   for "twin" commits differing only in timestamp. Truly diverged graphs
//!   are merged in *canonical orientation* ([`SyncStep::Merge`]): every peer
//!   merges the same `(first, second)` pair, so the merged graph (whose node
//!   order depends on orientation, see [`MergeOutcome::graph`]) and the
//!   resulting merge commit are identical on every peer - see
//!   [`Registry::commit_merge_canonical`].
//! - [`Staged`] validates fetched commits and graphs against their claimed
//!   addresses before they may touch a registry. Graph content is recomputed
//!   and rejected on mismatch (see [`verify_graph`]) - the security-critical
//!   check, as graphs compile to executed code. Commit chains apply
//!   oldest-first under their claimed keys, preserving addresses that
//!   [`Registry::add_commit`]'s parent-clearing would rewrite.
//! - [`monotonic_timestamp`] guards locally *minted* commit timestamps so an
//!   edit made after observing another commit always outranks it under
//!   [`BothModified::KeepNewest`], without ever rewriting received content
//!   (timestamps are part of the commit hash).
//!
//! [`MergeOutcome::graph`]: crate::merge::MergeOutcome::graph
//! [`BothModified::KeepNewest`]: crate::merge::BothModified::KeepNewest

use crate::{
    BlobLiveness, Bytes, Commit, CommitAddr, ContentAddr, DataGraph, GraphAddr, SectionId,
    Timestamp, blob_addr, commit_addr, graph_addr,
    history::{self, MergeAnalysis},
    registry::{Commits, Registry},
};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fmt,
    time::Duration,
};

/// How a local tip is brought up to date with a remote one.
///
/// Produced by [`plan_sync_step`]. Applying the step is the caller's job;
/// every variant that moves the tip does so identically on all peers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncStep {
    /// The remote tip is already in the local tip's ancestry (or is the local
    /// tip itself): nothing to do.
    UpToDate,
    /// The local tip is an ancestor of the remote: move to the remote tip, no
    /// commit required.
    FastForward(CommitAddr),
    /// The tips diverged but their commits point at the same graph:
    /// deterministically adopt the winner (max by `(timestamp, addr)`), no
    /// commit required. When the winner is the local tip there is nothing to
    /// do - the remote peer adopts ours.
    Adopt(CommitAddr),
    /// The tips truly diverged: merge `(first, second)` in canonical
    /// orientation (see `canonical_tips`) and commit the outcome via
    /// [`Registry::commit_merge_canonical`].
    Merge {
        first: CommitAddr,
        second: CommitAddr,
    },
    /// The tips share no common ancestor. How to proceed is an application
    /// decision (e.g. rename the local graph aside), never automatic.
    Unrelated,
}

/// A fetched object whose recomputed content address does not match the
/// address it was claimed under, or whose content is not in canonical form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyError {
    /// The commit's recomputed address differs from the claimed one.
    Commit {
        claimed: CommitAddr,
        actual: CommitAddr,
    },
    /// The graph's recomputed address differs from the claimed one.
    Graph {
        claimed: GraphAddr,
        actual: GraphAddr,
    },
    /// The blob's recomputed address differs from the claimed one.
    Blob {
        claimed: ContentAddr,
        actual: ContentAddr,
    },
    /// A data graph carries a non-canonical node, which would alias the same
    /// logical node under a second address.
    NonCanonicalNode { graph: GraphAddr, node_ix: usize },
}

/// An error preventing a [`Staged`] set from being applied to a registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyError {
    /// A strictly staged commit references a parent that is neither in the
    /// registry nor staged - a protocol violation for live deltas (snapshot
    /// commits detach instead, see [`Applied::truncated`]).
    MissingParent {
        commit: CommitAddr,
        parent: CommitAddr,
    },
    /// A staged commit's graph is neither in the registry nor staged.
    MissingGraph {
        commit: CommitAddr,
        graph: GraphAddr,
    },
}

/// Objects still required before a tip's closure is complete.
///
/// See [`Staged::missing`]. Both vecs are in discovery (breadth-first) order
/// and free of duplicates.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Missing {
    pub commits: Vec<CommitAddr>,
    pub graphs: Vec<GraphAddr>,
}

/// The result of applying a [`Staged`] set to a registry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Applied {
    /// The commits applied, oldest-first (parents before children).
    pub commits: Vec<CommitAddr>,
    /// The graphs applied.
    pub graphs: Vec<GraphAddr>,
    /// The blobs applied, as (section, address) pairs.
    pub blobs: Vec<(SectionId, ContentAddr)>,
    /// The number of snapshot commits whose absent parents were detached on
    /// insert because the sender truncated history below them (the same
    /// semantic a local [`prune`](crate::reach::prune) leaves behind).
    pub truncated: usize,
}

/// A staging area validating fetched objects before they touch a registry.
///
/// Live-delta commits are staged with [`insert_commit`](Self::insert_commit)
/// (strict: content must hash to the claimed address); join-snapshot commits
/// with [`insert_commit_grandfathered`](Self::insert_commit_grandfathered)
/// (a mismatch is tolerated and recorded, as senders legitimately hold
/// hash-inconsistent history after pruning). Graphs are always strict.
///
/// [`missing`](Self::missing) drives a fetch loop until
/// [`is_complete`](Self::is_complete), after which
/// [`apply`](Self::apply) inserts everything oldest-first under claimed keys.
#[derive(Clone, Debug, Default)]
pub struct Staged {
    commits: BTreeMap<CommitAddr, StagedCommit>,
    graphs: BTreeMap<GraphAddr, DataGraph>,
    blobs: BTreeMap<(SectionId, ContentAddr), (BlobLiveness, Bytes)>,
    grandfathered: Vec<CommitAddr>,
}

/// A staged commit alongside whether it arrived via a snapshot (tolerant
/// validation) or a live delta (strict).
#[derive(Clone, Debug)]
struct StagedCommit {
    commit: Commit,
    snapshot: bool,
}

/// The smallest timestamp increment: used to derive strictly-newer
/// deterministic timestamps.
const NANO: Duration = Duration::from_nanos(1);

impl Missing {
    /// `true` when no commits or graphs are missing.
    pub fn is_empty(&self) -> bool {
        self.commits.is_empty() && self.graphs.is_empty()
    }
}

impl Staged {
    /// An empty staging area.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage a live-delta commit, verifying that its content hashes to the
    /// claimed address.
    pub fn insert_commit(
        &mut self,
        claimed: CommitAddr,
        commit: Commit,
    ) -> Result<(), VerifyError> {
        let actual = commit_addr(&commit);
        if actual != claimed {
            return Err(VerifyError::Commit { claimed, actual });
        }
        let staged = StagedCommit {
            commit,
            snapshot: false,
        };
        self.commits.insert(claimed, staged);
        Ok(())
    }

    /// Stage a join-snapshot commit under its claimed address.
    ///
    /// An address mismatch is tolerated and recorded (see
    /// [`grandfathered`](Self::grandfathered)): a sender that has pruned
    /// history legitimately holds commits whose parents were detached in
    /// place under their original keys, so their content no longer re-hashes
    /// to the key. This defends DAG bookkeeping only - content honesty is the
    /// graph check, which is always strict.
    pub fn insert_commit_grandfathered(&mut self, claimed: CommitAddr, commit: Commit) {
        if commit_addr(&commit) != claimed {
            self.grandfathered.push(claimed);
        }
        let staged = StagedCommit {
            commit,
            snapshot: true,
        };
        self.commits.insert(claimed, staged);
    }

    /// The staged commits whose content did not re-hash to their claimed
    /// address (accepted via
    /// [`insert_commit_grandfathered`](Self::insert_commit_grandfathered)).
    pub fn grandfathered(&self) -> &[CommitAddr] {
        &self.grandfathered
    }

    /// The staged graphs, e.g. for computing wants beyond commit ancestry
    /// (node-referenced commits).
    pub fn graphs(&self) -> impl Iterator<Item = (&GraphAddr, &DataGraph)> {
        self.graphs.iter()
    }

    /// The staged commits under their claimed addresses.
    pub fn commits(&self) -> impl Iterator<Item = (&CommitAddr, &Commit)> {
        self.commits.iter().map(|(ca, s)| (ca, &s.commit))
    }

    /// The staged blobs under their section and claimed address.
    pub fn blobs(&self) -> impl Iterator<Item = &(SectionId, ContentAddr)> {
        self.blobs.keys()
    }

    /// Stage a graph, verifying it against the claimed address (see
    /// [`verify_graph`]).
    ///
    /// Always strict: graph content is compiled to executed code, so a graph
    /// that does not hash to the address it was requested under - or that
    /// carries a non-canonical node, aliasing a logical node under a second
    /// address - is rejected.
    pub fn insert_graph(
        &mut self,
        claimed: GraphAddr,
        graph: DataGraph,
    ) -> Result<(), VerifyError> {
        verify_graph(claimed, &graph)?;
        self.graphs.insert(claimed, graph);
        Ok(())
    }

    /// Stage a blob for the given blob section, verifying that its raw
    /// bytes hash to the claimed address. Always strict (blake3 of the
    /// bytes is cheap).
    ///
    /// `liveness` stamps the section if the receiving registry does not
    /// hold it yet (an existing section's stored liveness wins on apply).
    pub fn insert_blob(
        &mut self,
        section: SectionId,
        liveness: BlobLiveness,
        claimed: ContentAddr,
        bytes: impl Into<Bytes>,
    ) -> Result<(), VerifyError> {
        let bytes = bytes.into();
        let actual = blob_addr(&bytes);
        if actual != claimed {
            return Err(VerifyError::Blob { claimed, actual });
        }
        self.blobs.insert((section, claimed), (liveness, bytes));
        Ok(())
    }

    /// The commits and graphs still required to complete `tip`'s closure,
    /// walking ancestry through the staged set and the registry.
    ///
    /// The walk stops at commits already in the registry, relying on the
    /// registry invariant that its commits are parent-closed (absent parents
    /// are detached on insert and prune) and graph-complete.
    pub fn missing(&self, reg: &Registry, tip: CommitAddr) -> Missing {
        let mut missing = Missing::default();
        let mut missing_graphs: HashSet<GraphAddr> = HashSet::new();
        let mut queue: VecDeque<CommitAddr> = VecDeque::from([tip]);
        let mut visited: HashSet<CommitAddr> = HashSet::from([tip]);
        while let Some(ca) = queue.pop_front() {
            let Some(staged) = self.commits.get(&ca) else {
                if !reg.commits().contains_key(&ca) {
                    missing.commits.push(ca);
                }
                continue;
            };
            let graph = staged.commit.graph;
            if !reg.graphs().contains_key(&graph)
                && !self.graphs.contains_key(&graph)
                && missing_graphs.insert(graph)
            {
                missing.graphs.push(graph);
            }
            for parent in staged.commit.parents() {
                if visited.insert(parent) {
                    queue.push_back(parent);
                }
            }
        }
        missing
    }

    /// `true` when `tip`'s closure is complete and the staged set is ready to
    /// [`apply`](Self::apply).
    pub fn is_complete(&self, reg: &Registry, tip: CommitAddr) -> bool {
        self.missing(reg, tip).is_empty()
    }

    /// Apply the staged set to the registry: graphs first, then commits
    /// oldest-first (parents before children), each under its claimed key.
    ///
    /// All validation happens before the registry is touched, so an `Err`
    /// leaves it unchanged. Strict commits must have every parent present
    /// (registry or staged); snapshot commits with absent parents are
    /// detached on insert and counted in [`Applied::truncated`], mirroring
    /// the local post-prune semantic.
    pub fn apply(self, reg: &mut Registry) -> Result<Applied, ApplyError> {
        for (&ca, staged) in &self.commits {
            let graph = staged.commit.graph;
            if !reg.graphs().contains_key(&graph) && !self.graphs.contains_key(&graph) {
                return Err(ApplyError::MissingGraph { commit: ca, graph });
            }
            if !staged.snapshot {
                for parent in staged.commit.parents() {
                    if !reg.commits().contains_key(&parent) && !self.commits.contains_key(&parent) {
                        return Err(ApplyError::MissingParent { commit: ca, parent });
                    }
                }
            }
        }
        let order = topo_order(&self.commits);
        let mut applied = Applied::default();
        for (ga, graph) in self.graphs {
            reg.insert_graph_at(ga, graph);
            applied.graphs.push(ga);
        }
        // Blobs are leaves with no referential invariants: apply alongside
        // graphs, before any commit.
        for ((section, addr), (liveness, bytes)) in self.blobs {
            reg.insert_blob_at(section.clone(), liveness, addr, bytes);
            applied.blobs.push((section, addr));
        }
        let mut commits = self.commits;
        for ca in order {
            let Some(staged) = commits.remove(&ca) else {
                continue;
            };
            let mut commit = staged.commit;
            if staged.snapshot {
                let mut detached = false;
                if commit
                    .parent
                    .is_some_and(|p| !reg.commits().contains_key(&p))
                {
                    commit.parent = None;
                    detached = true;
                }
                let n = commit.merge_parents.len();
                commit
                    .merge_parents
                    .retain(|p| reg.commits().contains_key(p));
                detached |= commit.merge_parents.len() != n;
                if detached {
                    applied.truncated += 1;
                }
            }
            reg.insert_commit_at(ca, commit);
            applied.commits.push(ca);
        }
        Ok(applied)
    }
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Commit { claimed, actual } => {
                write!(f, "commit claimed as {claimed} hashes to {actual}")
            }
            Self::Graph { claimed, actual } => {
                write!(f, "graph claimed as {claimed} hashes to {actual}")
            }
            Self::Blob { claimed, actual } => {
                write!(f, "blob claimed as {claimed} hashes to {actual}")
            }
            Self::NonCanonicalNode { graph, node_ix } => {
                write!(
                    f,
                    "graph claimed as {graph} has non-canonical node {node_ix}"
                )
            }
        }
    }
}

impl std::error::Error for VerifyError {}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingParent { commit, parent } => {
                write!(f, "commit {commit} references missing parent {parent}")
            }
            Self::MissingGraph { commit, graph } => {
                write!(f, "commit {commit} references missing graph {graph}")
            }
        }
    }
}

impl std::error::Error for ApplyError {}

/// Verify a fetched graph against the address it was claimed under: canonical
/// form of every node plus a strict content re-hash.
///
/// A non-canonical node hashes consistently with its own (non-canonical)
/// form, so the address check alone would admit it - as an alias of the same
/// logical node under a second network-wide address. Rejecting it keeps one
/// logical node to exactly one address.
pub fn verify_graph(claimed: GraphAddr, graph: &DataGraph) -> Result<(), VerifyError> {
    if let Some(node_ix) = graph.node_weights().position(|n| !n.is_canonical()) {
        return Err(VerifyError::NonCanonicalNode {
            graph: claimed,
            node_ix,
        });
    }
    let actual = graph_addr(graph);
    if actual != claimed {
        return Err(VerifyError::Graph { claimed, actual });
    }
    Ok(())
}

/// Order two diverged tips canonically: ascending by `(timestamp, addr)`.
///
/// The order is a pure function of commit content, so every peer derives the
/// same orientation for the same pair - the prerequisite for identical merge
/// outcomes, as the merged graph's node order depends on which side plays
/// "ours" (see [`MergeOutcome::graph`](crate::merge::MergeOutcome::graph)).
pub(crate) fn canonical_tips(
    commits: &Commits,
    a: CommitAddr,
    b: CommitAddr,
) -> (CommitAddr, CommitAddr) {
    let key = |ca: CommitAddr| (commits.get(&ca).map(|c| c.timestamp), ca);
    if key(a) <= key(b) { (a, b) } else { (b, a) }
}

/// The timestamp for a canonical merge commit: strictly newer than both tips
/// (`max + 1ns`), and a pure function of the two tips so every peer mints the
/// identical merge commit.
///
/// Being strictly newer also means chain-tracked edit times through the merge
/// never tie against pre-merge edits.
pub(crate) fn merge_timestamp(
    commits: &Commits,
    first: CommitAddr,
    second: CommitAddr,
) -> Timestamp {
    let ts = |ca: CommitAddr| commits.get(&ca).map(|c| c.timestamp).unwrap_or_default();
    ts(first).max(ts(second)).saturating_add(NANO)
}

/// A locally minted commit's timestamp, guarded for session causality: at
/// least one nanosecond newer than the newest commit observed in the session.
///
/// This keeps "last edit wins" honest under clock skew - an edit made *after*
/// seeing a remote commit always outranks it - without touching received
/// content. Truly concurrent blind edits still race wall clocks, which is
/// exactly the case where last-edit-wins is arbitrary anyway.
pub fn monotonic_timestamp(now: Timestamp, newest_seen: Timestamp) -> Timestamp {
    now.max(newest_seen.saturating_add(NANO))
}

/// Classify how the local tip should be brought up to date with a remote tip.
///
/// Both tips (and the history connecting them to their base) are expected to
/// be present in `commits` - i.e. the remote tip's closure has been fetched
/// and applied. Every branch of the decision is a pure, side-independent
/// function of commit content, so two peers planning opposite directions of
/// the same pair reach complementary steps that converge on the same tip.
pub fn plan_sync_step(commits: &Commits, local: CommitAddr, remote: CommitAddr) -> SyncStep {
    if local == remote {
        return SyncStep::UpToDate;
    }
    match history::analyze(commits, local, remote) {
        MergeAnalysis::Unrelated => SyncStep::Unrelated,
        MergeAnalysis::AlreadyUpToDate => SyncStep::UpToDate,
        MergeAnalysis::FastForward => SyncStep::FastForward(remote),
        MergeAnalysis::Diverged(_) => {
            let graph = |ca: CommitAddr| commits.get(&ca).map(|c| c.graph);
            match (graph(local), graph(remote)) {
                // Twin commits: same graph reached independently (concurrent
                // identical edits, concurrent resyncs). Adopt the winner
                // rather than minting a pointless merge commit.
                (Some(gl), Some(gr)) if gl == gr => {
                    let key = |ca: CommitAddr| (commits.get(&ca).map(|c| c.timestamp), ca);
                    let winner = if key(local) >= key(remote) {
                        local
                    } else {
                        remote
                    };
                    SyncStep::Adopt(winner)
                }
                _ => {
                    let (first, second) = canonical_tips(commits, local, remote);
                    SyncStep::Merge { first, second }
                }
            }
        }
    }
}

/// Topologically order the staged commits oldest-first (parents before
/// children), following only parent edges within the staged set.
///
/// Iteration over the `BTreeMap` keeps the order deterministic. A parent
/// cycle (only forgeable via grandfathered claimed keys) cannot occur in
/// honestly hashed content and degrades gracefully: the visited set breaks
/// the cycle and the out-of-order parent is later detached on apply.
fn topo_order(staged: &BTreeMap<CommitAddr, StagedCommit>) -> Vec<CommitAddr> {
    let mut order = Vec::with_capacity(staged.len());
    let mut visited: HashSet<CommitAddr> = HashSet::new();
    for &start in staged.keys() {
        if visited.contains(&start) {
            continue;
        }
        let mut stack = vec![(start, false)];
        while let Some((ca, expanded)) = stack.pop() {
            if expanded {
                order.push(ca);
                continue;
            }
            if !visited.insert(ca) {
                continue;
            }
            stack.push((ca, true));
            if let Some(staged_commit) = staged.get(&ca) {
                for parent in staged_commit.commit.parents() {
                    if staged.contains_key(&parent) && !visited.contains(&parent) {
                        stack.push((parent, false));
                    }
                }
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentAddr, Datum, Head, NodeData};

    fn graph(nodes: &[&str]) -> DataGraph {
        let mut g = DataGraph::default();
        for n in nodes {
            g.add_node(NodeData::new(*n, Datum::Map(vec![])));
        }
        g
    }

    fn graph_addr_raw(n: u8) -> GraphAddr {
        GraphAddr::from(ContentAddr::from([n; 32]))
    }

    /// Add a commit to the map, returning its address.
    fn add(commits: &mut Commits, secs: u64, parent: Option<CommitAddr>, g: u8) -> CommitAddr {
        let commit = Commit::new(Duration::from_secs(secs), parent, graph_addr_raw(g));
        let ca = commit_addr(&commit);
        commits.insert(ca, commit);
        ca
    }

    #[test]
    fn canonical_tips_is_order_independent() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let a = add(&mut commits, 2, Some(root), 2);
        let b = add(&mut commits, 3, Some(root), 3);
        assert_eq!(canonical_tips(&commits, a, b), (a, b));
        assert_eq!(canonical_tips(&commits, b, a), (a, b));
    }

    #[test]
    fn canonical_tips_tie_breaks_on_addr() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        // Same timestamp, different graphs: only the addr differentiates.
        let a = add(&mut commits, 2, Some(root), 2);
        let b = add(&mut commits, 2, Some(root), 3);
        let expected = if a < b { (a, b) } else { (b, a) };
        assert_eq!(canonical_tips(&commits, a, b), expected);
        assert_eq!(canonical_tips(&commits, b, a), expected);
    }

    #[test]
    fn merge_timestamp_is_strictly_newer_than_both_tips() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let a = add(&mut commits, 2, Some(root), 2);
        let b = add(&mut commits, 5, Some(root), 3);
        let ts = merge_timestamp(&commits, a, b);
        assert_eq!(ts, Duration::from_secs(5) + NANO);
        assert_eq!(ts, merge_timestamp(&commits, b, a));
    }

    #[test]
    fn monotonic_timestamp_guards_causality() {
        let now = Duration::from_secs(10);
        let behind = Duration::from_secs(5);
        let ahead = Duration::from_secs(20);
        // A clock ahead of everything observed passes through.
        assert_eq!(monotonic_timestamp(now, behind), now);
        // A clock behind an observed commit is bumped strictly past it.
        assert_eq!(monotonic_timestamp(now, ahead), ahead + NANO);
    }

    #[test]
    fn plan_up_to_date_and_fast_forward() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let tip = add(&mut commits, 2, Some(root), 2);
        assert_eq!(plan_sync_step(&commits, tip, tip), SyncStep::UpToDate);
        assert_eq!(plan_sync_step(&commits, tip, root), SyncStep::UpToDate);
        assert_eq!(
            plan_sync_step(&commits, root, tip),
            SyncStep::FastForward(tip)
        );
    }

    #[test]
    fn plan_unrelated() {
        let mut commits = Commits::default();
        let a = add(&mut commits, 1, None, 1);
        let b = add(&mut commits, 2, None, 2);
        assert_eq!(plan_sync_step(&commits, a, b), SyncStep::Unrelated);
    }

    #[test]
    fn plan_adopts_twin_commits_without_merging() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        // Two peers independently commit the same graph at different times.
        let a = add(&mut commits, 2, Some(root), 2);
        let b = add(&mut commits, 3, Some(root), 2);
        // Both directions adopt the same winner: the newer commit.
        assert_eq!(plan_sync_step(&commits, a, b), SyncStep::Adopt(b));
        assert_eq!(plan_sync_step(&commits, b, a), SyncStep::Adopt(b));
    }

    #[test]
    fn plan_merges_diverged_graphs_in_canonical_orientation() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let a = add(&mut commits, 2, Some(root), 2);
        let b = add(&mut commits, 3, Some(root), 3);
        let expected = SyncStep::Merge {
            first: a,
            second: b,
        };
        // The orientation is the same regardless of which side plans.
        assert_eq!(plan_sync_step(&commits, a, b), expected);
        assert_eq!(plan_sync_step(&commits, b, a), expected);
    }

    #[test]
    fn commit_merge_canonical_mints_identical_commit_on_both_peers() {
        // Two registries with identical content merge the same diverged pair
        // from opposite orientations.
        let build = || {
            let mut reg = Registry::default();
            let base = graph(&["base"]);
            let base_ca = crate::graph_addr(&base);
            let root = reg.commit_graph(Duration::from_secs(1), None, base_ca, || base);
            let ga = graph(&["base", "a"]);
            let ga_ca = crate::graph_addr(&ga);
            let a = reg.commit_graph(Duration::from_secs(2), Some(root), ga_ca, || ga);
            let gb = graph(&["base", "b"]);
            let gb_ca = crate::graph_addr(&gb);
            let b = reg.commit_graph(Duration::from_secs(3), Some(root), gb_ca, || gb);
            (reg, a, b)
        };
        let (mut reg_1, a, b) = build();
        let (mut reg_2, a_2, b_2) = build();
        assert_eq!((a, b), (a_2, b_2));
        let merged = graph(&["base", "a", "b"]);
        let merged_ca = crate::graph_addr(&merged);
        // Peer 1's head is on `a` and merges in `b`; peer 2 vice versa.
        let mut head_1 = Head::Commit(a);
        let mut head_2 = Head::Commit(b);
        let m_1 = reg_1.commit_merge_canonical(a, b, merged_ca, || merged.clone(), &mut head_1);
        let m_2 = reg_2.commit_merge_canonical(b, a, merged_ca, || merged.clone(), &mut head_2);
        assert_eq!(m_1, m_2);
        assert_eq!(head_1, Head::Commit(m_1));
        assert_eq!(head_2, Head::Commit(m_1));
        let commit = &reg_1.commits()[&m_1];
        // Canonical orientation: `a` (older) is the first parent on both.
        assert_eq!(commit.parent, Some(a));
        assert_eq!(commit.merge_parents, vec![b]);
        assert_eq!(commit, &reg_2.commits()[&m_2]);
    }

    #[test]
    fn staged_rejects_forged_commit_addr() {
        let mut staged = Staged::new();
        let commit = Commit::new(Duration::from_secs(1), None, graph_addr_raw(1));
        let forged = CommitAddr::from(ContentAddr::from([9; 32]));
        let err = staged.insert_commit(forged, commit.clone()).unwrap_err();
        assert_eq!(
            err,
            VerifyError::Commit {
                claimed: forged,
                actual: commit_addr(&commit),
            }
        );
        // The honest address is accepted.
        staged.insert_commit(commit_addr(&commit), commit).unwrap();
    }

    #[test]
    fn staged_rejects_forged_graph_addr() {
        let mut staged = Staged::new();
        let g = graph(&["a"]);
        let forged = graph_addr_raw(9);
        let err = staged.insert_graph(forged, g.clone()).unwrap_err();
        assert_eq!(
            err,
            VerifyError::Graph {
                claimed: forged,
                actual: crate::graph_addr(&g),
            }
        );
        staged.insert_graph(crate::graph_addr(&g), g).unwrap();
    }

    /// A non-canonical node hashes consistently with its own form, so an
    /// address check alone would admit it as an alias of the same logical
    /// node under a second address. The graph check must reject it.
    #[test]
    fn staged_rejects_non_canonical_node() {
        let node = |entries: Vec<(&str, Datum)>| {
            NodeData::new(
                "test",
                Datum::Map(
                    entries
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), v))
                        .collect(),
                ),
            )
        };
        // Map keys deliberately out of order.
        let bad = node(vec![("b", Datum::Null), ("a", Datum::Bool(true))]);
        assert!(!bad.is_canonical());
        let mut g = DataGraph::default();
        g.add_node(bad);
        // The graph hashes consistently with its non-canonical form: the
        // claimed address matches, yet the graph must still be rejected.
        let claimed = crate::graph_addr(&g);

        let mut staged = Staged::new();
        let err = staged.insert_graph(claimed, g.clone()).unwrap_err();
        assert_eq!(
            err,
            VerifyError::NonCanonicalNode {
                graph: claimed,
                node_ix: 0,
            }
        );

        g.node_weights_mut().for_each(NodeData::canonicalize);
        staged.insert_graph(crate::graph_addr(&g), g).unwrap();
    }

    #[test]
    fn staged_grandfathered_accepts_and_records_mismatch() {
        let mut staged = Staged::new();
        // A post-prune commit: parent detached in place under its original
        // key, so the content no longer hashes to the key.
        let parent = CommitAddr::from(ContentAddr::from([7; 32]));
        let original = Commit::new(Duration::from_secs(2), Some(parent), graph_addr_raw(1));
        let original_ca = commit_addr(&original);
        let detached = Commit::new(Duration::from_secs(2), None, graph_addr_raw(1));
        staged.insert_commit_grandfathered(original_ca, detached);
        assert_eq!(staged.grandfathered(), &[original_ca]);
        // A consistent snapshot commit records nothing.
        let ok = Commit::new(Duration::from_secs(3), None, graph_addr_raw(1));
        staged.insert_commit_grandfathered(commit_addr(&ok), ok);
        assert_eq!(staged.grandfathered().len(), 1);
    }

    #[test]
    fn missing_reports_unfetched_parents_and_graphs() {
        let reg = Registry::default();
        let mut staged = Staged::new();
        let g = graph(&["a"]);
        let g_ca = crate::graph_addr(&g);
        let root = Commit::new(Duration::from_secs(1), None, g_ca);
        let root_ca = commit_addr(&root);
        let tip = Commit::new(Duration::from_secs(2), Some(root_ca), g_ca);
        let tip_ca = commit_addr(&tip);
        // Nothing staged: the tip itself is missing.
        assert_eq!(staged.missing(&reg, tip_ca).commits, vec![tip_ca]);
        staged.insert_commit(tip_ca, tip).unwrap();
        // The tip is staged: its parent and graph are missing.
        let missing = staged.missing(&reg, tip_ca);
        assert_eq!(missing.commits, vec![root_ca]);
        assert_eq!(missing.graphs, vec![g_ca]);
        staged.insert_commit(root_ca, root).unwrap();
        staged.insert_graph(g_ca, g).unwrap();
        assert!(staged.is_complete(&reg, tip_ca));
    }

    #[test]
    fn missing_stops_at_registry_commits() {
        let mut reg = Registry::default();
        let base = graph(&["base"]);
        let base_ca = crate::graph_addr(&base);
        let root = reg.commit_graph(Duration::from_secs(1), None, base_ca, || base);
        let mut staged = Staged::new();
        let g = graph(&["base", "a"]);
        let g_ca = crate::graph_addr(&g);
        let tip = Commit::new(Duration::from_secs(2), Some(root), g_ca);
        let tip_ca = commit_addr(&tip);
        staged.insert_commit(tip_ca, tip).unwrap();
        staged.insert_graph(g_ca, g).unwrap();
        // The parent is already in the registry: closure is complete.
        assert!(staged.is_complete(&reg, tip_ca));
    }

    #[test]
    fn apply_preserves_addresses_add_commit_would_rewrite() {
        // A chain staged out of order applies parents-first under claimed
        // keys, unlike `Registry::add_commit` which re-parents-then-hashes.
        let mut reg = Registry::default();
        let mut staged = Staged::new();
        let g = graph(&["a"]);
        let g_ca = crate::graph_addr(&g);
        let root = Commit::new(Duration::from_secs(1), None, g_ca);
        let root_ca = commit_addr(&root);
        let mid = Commit::new(Duration::from_secs(2), Some(root_ca), g_ca);
        let mid_ca = commit_addr(&mid);
        let tip = Commit::new(Duration::from_secs(3), Some(mid_ca), g_ca);
        let tip_ca = commit_addr(&tip);
        // Stage newest-first, as a fetch loop discovers them.
        staged.insert_commit(tip_ca, tip).unwrap();
        staged.insert_commit(mid_ca, mid).unwrap();
        staged.insert_commit(root_ca, root).unwrap();
        staged.insert_graph(g_ca, g).unwrap();
        let applied = staged.apply(&mut reg).unwrap();
        assert_eq!(applied.commits.len(), 3);
        assert_eq!(applied.truncated, 0);
        // Parents precede children in the applied order.
        let pos = |ca| applied.commits.iter().position(|&c| c == ca).unwrap();
        assert!(pos(root_ca) < pos(mid_ca));
        assert!(pos(mid_ca) < pos(tip_ca));
        // Every commit re-hashes to its key: no address was rewritten.
        for (&ca, commit) in reg.commits() {
            assert_eq!(ca, commit_addr(commit));
        }
        assert_eq!(reg.commits()[&tip_ca].parent, Some(mid_ca));
    }

    #[test]
    fn apply_errors_on_strict_missing_parent() {
        let mut reg = Registry::default();
        let mut staged = Staged::new();
        let g = graph(&["a"]);
        let g_ca = crate::graph_addr(&g);
        let absent = CommitAddr::from(ContentAddr::from([7; 32]));
        let tip = Commit::new(Duration::from_secs(2), Some(absent), g_ca);
        let tip_ca = commit_addr(&tip);
        staged.insert_commit(tip_ca, tip).unwrap();
        staged.insert_graph(g_ca, g).unwrap();
        let err = staged.apply(&mut reg).unwrap_err();
        assert_eq!(
            err,
            ApplyError::MissingParent {
                commit: tip_ca,
                parent: absent,
            }
        );
        // The registry was left untouched.
        assert!(reg.commits().is_empty());
        assert!(reg.graphs().is_empty());
    }

    #[test]
    fn apply_errors_on_missing_graph() {
        let mut reg = Registry::default();
        let mut staged = Staged::new();
        let tip = Commit::new(Duration::from_secs(1), None, graph_addr_raw(1));
        let tip_ca = commit_addr(&tip);
        staged.insert_commit(tip_ca, tip).unwrap();
        let err = staged.apply(&mut reg).unwrap_err();
        assert_eq!(
            err,
            ApplyError::MissingGraph {
                commit: tip_ca,
                graph: graph_addr_raw(1),
            }
        );
        assert!(reg.commits().is_empty());
    }

    #[test]
    fn apply_detaches_truncated_snapshot_parents() {
        // The oldest snapshot commit references a parent below the history
        // depth cutoff: it applies detached, mirroring post-prune semantics.
        let mut reg = Registry::default();
        let mut staged = Staged::new();
        let g = graph(&["a"]);
        let g_ca = crate::graph_addr(&g);
        let below_cutoff = CommitAddr::from(ContentAddr::from([7; 32]));
        let oldest = Commit::new(Duration::from_secs(1), Some(below_cutoff), g_ca);
        let oldest_ca = commit_addr(&oldest);
        let tip = Commit::new(Duration::from_secs(2), Some(oldest_ca), g_ca);
        let tip_ca = commit_addr(&tip);
        staged.insert_commit_grandfathered(oldest_ca, oldest);
        staged.insert_commit_grandfathered(tip_ca, tip);
        staged.insert_graph(g_ca, g).unwrap();
        let applied = staged.apply(&mut reg).unwrap();
        assert_eq!(applied.truncated, 1);
        assert_eq!(reg.commits()[&oldest_ca].parent, None);
        assert_eq!(reg.commits()[&tip_ca].parent, Some(oldest_ca));
    }
}
