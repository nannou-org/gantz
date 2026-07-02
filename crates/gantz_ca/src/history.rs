//! Utilities for walking commit ancestry.
//!
//! Commits form a DAG: every commit has an optional first parent and, for
//! merge commits, one or more merge parents (see [`Commit::parents`]). These
//! free fns provide the ancestry queries needed for merging diverged heads.

use crate::{Commit, CommitAddr, registry::Commits};
use std::collections::{HashSet, VecDeque};

/// The relationship between two commit tips, from the perspective of merging
/// `theirs` into `ours`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeAnalysis {
    /// The tips share no common ancestor.
    Unrelated,
    /// `theirs` is an ancestor of `ours`: there is nothing to merge.
    AlreadyUpToDate,
    /// `ours` is an ancestor of `theirs`: the head can simply move to
    /// `theirs` without a merge commit.
    FastForward,
    /// The tips have diverged; a true merge is required. Carries the merge
    /// base (best common ancestor).
    Diverged(CommitAddr),
}

/// All ancestors of `tip` (inclusive of `tip` itself), in breadth-first order
/// over all parents (first parent and merge parents).
///
/// Commits absent from `commits` terminate their branch of the walk.
pub fn ancestors(commits: &Commits, tip: CommitAddr) -> impl Iterator<Item = CommitAddr> + '_ {
    let mut queue: VecDeque<CommitAddr> = VecDeque::from([tip]);
    let mut visited: HashSet<CommitAddr> = HashSet::from([tip]);
    std::iter::from_fn(move || {
        let ca = queue.pop_front()?;
        if let Some(commit) = commits.get(&ca) {
            for parent in commit.parents() {
                if visited.insert(parent) {
                    queue.push_back(parent);
                }
            }
        }
        Some(ca)
    })
}

/// The first-parent chain from `tip` back to the root (inclusive of `tip`).
///
/// This is the "main line" of a head's history: merge parents are not
/// followed, matching how undo walks back through commits.
pub fn first_parent_chain(
    commits: &Commits,
    tip: CommitAddr,
) -> impl Iterator<Item = CommitAddr> + '_ {
    let mut next = Some(tip);
    std::iter::from_fn(move || {
        let ca = next?;
        next = commits.get(&ca).and_then(|commit| commit.parent);
        Some(ca)
    })
}

/// The best common ancestor of `a` and `b`, or `None` when their histories
/// are unrelated.
///
/// A common ancestor is "best" when it is not itself an ancestor of another
/// common ancestor. When several such candidates exist (criss-cross
/// histories), one is chosen deterministically by max `(timestamp, addr)`;
/// recursively merging the candidates is out of scope.
pub fn merge_base(commits: &Commits, a: CommitAddr, b: CommitAddr) -> Option<CommitAddr> {
    let a_ancestors: HashSet<CommitAddr> = ancestors(commits, a).collect();
    let mut common: HashSet<CommitAddr> = ancestors(commits, b)
        .filter(|ca| a_ancestors.contains(ca))
        .collect();
    // Drop candidates that are proper ancestors of another candidate.
    for ca in common.clone() {
        if !common.contains(&ca) {
            continue;
        }
        for ancestor in ancestors(commits, ca).skip(1) {
            common.remove(&ancestor);
        }
    }
    common
        .into_iter()
        .max_by_key(|&ca| (commits.get(&ca).map(|c| c.timestamp), ca))
}

/// Analyze the relationship between two tips, from the perspective of merging
/// `theirs` into `ours`.
pub fn analyze(commits: &Commits, ours: CommitAddr, theirs: CommitAddr) -> MergeAnalysis {
    match merge_base(commits, ours, theirs) {
        None => MergeAnalysis::Unrelated,
        Some(base) if base == theirs => MergeAnalysis::AlreadyUpToDate,
        Some(base) if base == ours => MergeAnalysis::FastForward,
        Some(base) => MergeAnalysis::Diverged(base),
    }
}

/// The graph addresses along `tip`'s first-parent chain from `tip` back to
/// `base` (inclusive of both), or `None` if `base` is not on the chain.
pub fn first_parent_chain_to(
    commits: &Commits,
    tip: CommitAddr,
    base: CommitAddr,
) -> Option<Vec<&Commit>> {
    let mut chain = Vec::new();
    for ca in first_parent_chain(commits, tip) {
        chain.push(commits.get(&ca)?);
        if ca == base {
            return Some(chain);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Commit, ContentAddr, GraphAddr, commit_addr};
    use std::time::Duration;

    fn graph_addr(n: u8) -> GraphAddr {
        GraphAddr::from(ContentAddr::from([n; 32]))
    }

    /// Add a commit to the map, returning its address.
    fn add(commits: &mut Commits, secs: u64, parent: Option<CommitAddr>, g: u8) -> CommitAddr {
        let commit = Commit::new(Duration::from_secs(secs), parent, graph_addr(g));
        let ca = commit_addr(&commit);
        commits.insert(ca, commit);
        ca
    }

    /// Add a merge commit to the map, returning its address.
    fn add_merge(
        commits: &mut Commits,
        secs: u64,
        ours: CommitAddr,
        theirs: CommitAddr,
        g: u8,
    ) -> CommitAddr {
        let commit = Commit::new_merge(Duration::from_secs(secs), ours, theirs, graph_addr(g));
        let ca = commit_addr(&commit);
        commits.insert(ca, commit);
        ca
    }

    #[test]
    fn ancestors_of_linear_chain() {
        let mut commits = Commits::default();
        let a = add(&mut commits, 1, None, 1);
        let b = add(&mut commits, 2, Some(a), 2);
        let c = add(&mut commits, 3, Some(b), 3);
        assert_eq!(ancestors(&commits, c).collect::<Vec<_>>(), vec![c, b, a]);
    }

    #[test]
    fn ancestors_follow_merge_parents() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let ours = add(&mut commits, 2, Some(root), 2);
        let theirs = add(&mut commits, 3, Some(root), 3);
        let merge = add_merge(&mut commits, 4, ours, theirs, 4);
        let all: HashSet<_> = ancestors(&commits, merge).collect();
        assert_eq!(all, [merge, ours, theirs, root].into_iter().collect());
    }

    #[test]
    fn first_parent_chain_skips_merge_parents() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let ours = add(&mut commits, 2, Some(root), 2);
        let theirs = add(&mut commits, 3, Some(root), 3);
        let merge = add_merge(&mut commits, 4, ours, theirs, 4);
        assert_eq!(
            first_parent_chain(&commits, merge).collect::<Vec<_>>(),
            vec![merge, ours, root],
        );
    }

    #[test]
    fn merge_base_of_diverged_tips() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let base = add(&mut commits, 2, Some(root), 2);
        let ours = add(&mut commits, 3, Some(base), 3);
        let theirs = add(&mut commits, 4, Some(base), 4);
        assert_eq!(merge_base(&commits, ours, theirs), Some(base));
        assert_eq!(
            analyze(&commits, ours, theirs),
            MergeAnalysis::Diverged(base)
        );
    }

    #[test]
    fn merge_base_of_unrelated_roots() {
        let mut commits = Commits::default();
        let a = add(&mut commits, 1, None, 1);
        let b = add(&mut commits, 2, None, 2);
        assert_eq!(merge_base(&commits, a, b), None);
        assert_eq!(analyze(&commits, a, b), MergeAnalysis::Unrelated);
    }

    #[test]
    fn analyze_fast_forward_and_up_to_date() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let tip = add(&mut commits, 2, Some(root), 2);
        assert_eq!(analyze(&commits, root, tip), MergeAnalysis::FastForward);
        assert_eq!(analyze(&commits, tip, root), MergeAnalysis::AlreadyUpToDate);
        assert_eq!(analyze(&commits, tip, tip), MergeAnalysis::AlreadyUpToDate);
    }

    #[test]
    fn merge_base_after_a_merge_commit() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let ours = add(&mut commits, 2, Some(root), 2);
        let theirs = add(&mut commits, 3, Some(root), 3);
        let merge = add_merge(&mut commits, 4, ours, theirs, 4);
        let after = add(&mut commits, 5, Some(merge), 5);
        // Theirs' tip is now an ancestor of the merged line.
        assert_eq!(
            analyze(&commits, after, theirs),
            MergeAnalysis::AlreadyUpToDate
        );
        // A new commit on theirs' line diverges at theirs.
        let theirs2 = add(&mut commits, 6, Some(theirs), 6);
        assert_eq!(
            analyze(&commits, after, theirs2),
            MergeAnalysis::Diverged(theirs)
        );
    }

    #[test]
    fn criss_cross_tie_break_is_deterministic() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let a = add(&mut commits, 2, Some(root), 2);
        let b = add(&mut commits, 3, Some(root), 3);
        // Criss-cross: each side merges the other's tip.
        let ma = add_merge(&mut commits, 4, a, b, 4);
        let mb = add_merge(&mut commits, 5, b, a, 5);
        // Both a and b are best common ancestors; the later timestamp wins.
        assert_eq!(merge_base(&commits, ma, mb), Some(b));
    }

    #[test]
    fn first_parent_chain_to_reaches_base_or_none() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let base = add(&mut commits, 2, Some(root), 2);
        let tip = add(&mut commits, 3, Some(base), 3);
        let chain = first_parent_chain_to(&commits, tip, base).unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(crate::commit_addr(chain[0]), tip);
        assert_eq!(crate::commit_addr(chain[1]), base);
        // A base not on the chain yields None.
        let other = add(&mut commits, 4, None, 4);
        assert!(first_parent_chain_to(&commits, tip, other).is_none());
    }
}
