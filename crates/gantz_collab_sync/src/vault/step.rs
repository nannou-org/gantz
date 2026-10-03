//! The rules that bring each name into agreement with a vault.

use gantz_ca::{self as ca, CommitAddr, SyncStep, registry::Commits};

/// How a device brings one name into agreement with a vault.
///
/// Produced by [`plan`]. The vault accepts a push for a name only while its
/// head for that name is still the head the pushing device last agreed with.
/// So a device never overwrites a vault change it has not seen, and it does
/// all merging itself. Every push is made against the vault's current head
/// as the base.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Step {
    /// The local head equals the vault's. Record it as the agreed base.
    InSync,
    /// Only the local head changed, and it moved to an ancestor of the
    /// vault's head. Backwards moves do not sync. Nothing to do.
    Keep,
    /// Set the local head to the vault's head exactly. `None` removes the
    /// name locally.
    Adopt(Option<CommitAddr>),
    /// Push the local head to the vault. `None` removes the name there.
    Push(Option<CommitAddr>),
    /// The local and vault heads diverged. Merge `(first, second)` in
    /// canonical orientation, then push the merge. `remote` is the vault's
    /// head, one of the two.
    Merge {
        first: CommitAddr,
        second: CommitAddr,
        remote: CommitAddr,
    },
    /// The local and vault heads both changed and share no history. Move the
    /// local graph aside to a new name, adopt the vault's head, and push the
    /// aside name as a new name.
    Aside {
        local: CommitAddr,
        remote: CommitAddr,
    },
}

/// Classify how a device brings one name into agreement with a vault.
///
/// It is a three-way decision over the name's `local` head, the `base` head
/// the device last agreed with the vault, and the vault's current `remote`
/// head. `None` means the name is absent on that side.
///
/// - Only the vault changed: adopt its head exactly, deletes included.
/// - Only the local head changed: push it. A move to an ancestor of the base
///   stays local, and a divergence from the base merges first.
/// - Both changed: an edit beats a delete on either side. Otherwise follow
///   [`ca::plan_sync_step`], except that the vault is the incumbent. Twins
///   adopt the vault's head, and unrelated histories move the local graph
///   aside.
///
/// When the heads differ, `commits` must hold `remote`'s history. That is,
/// the caller fetches the vault's head before planning.
pub(crate) fn plan(
    commits: &Commits,
    local: Option<CommitAddr>,
    base: Option<CommitAddr>,
    remote: Option<CommitAddr>,
) -> Step {
    if local == remote {
        return Step::InSync;
    }
    if local == base {
        return Step::Adopt(remote);
    }
    let only_local = remote == base;
    let (Some(l), Some(r)) = (local, remote) else {
        return match local {
            None if only_local => Step::Push(None),
            None => Step::Adopt(remote),
            Some(l) => Step::Push(Some(l)),
        };
    };
    match ca::plan_sync_step(commits, l, r) {
        SyncStep::UpToDate => Step::Push(Some(l)),
        SyncStep::FastForward(_) if only_local => Step::Keep,
        SyncStep::FastForward(_) | SyncStep::Adopt(_) => Step::Adopt(Some(r)),
        SyncStep::Merge { first, second } => Step::Merge {
            first,
            second,
            remote: r,
        },
        SyncStep::Unrelated if only_local => Step::Push(Some(l)),
        SyncStep::Unrelated => Step::Aside {
            local: l,
            remote: r,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_ca::{Commit, ContentAddr, GraphAddr};
    use std::time::Duration;

    /// Add a commit to the map, returning its address.
    fn add(commits: &mut Commits, secs: u64, parent: Option<CommitAddr>, g: u8) -> CommitAddr {
        let ga = GraphAddr::from(ContentAddr::from([g; 32]));
        let commit = Commit::new(Duration::from_secs(secs), parent, ga);
        let ca = ca::commit_addr(&commit);
        commits.insert(ca, commit);
        ca
    }

    #[test]
    fn plan_cases() {
        let mut commits = Commits::default();
        let root = add(&mut commits, 1, None, 1);
        let a = add(&mut commits, 2, Some(root), 2);
        let b = add(&mut commits, 3, Some(a), 3);
        let c = add(&mut commits, 4, Some(root), 4);
        let other = add(&mut commits, 5, None, 5);
        // Twins reach the same graph. The local one is newer.
        let twin_local = add(&mut commits, 7, Some(root), 6);
        let twin_remote = add(&mut commits, 6, Some(root), 6);
        let merge = Step::Merge {
            first: a,
            second: c,
            remote: a,
        };
        let aside = Step::Aside {
            local: other,
            remote: a,
        };
        for (label, local, base, remote, expected) in [
            ("in sync", Some(b), Some(a), Some(b), Step::InSync),
            ("removed on both", None, Some(a), None, Step::InSync),
            ("absent on all", None, None, None, Step::InSync),
            // Only the vault changed.
            (
                "vault moved on",
                Some(a),
                Some(a),
                Some(b),
                Step::Adopt(Some(b)),
            ),
            ("vault removed", Some(a), Some(a), None, Step::Adopt(None)),
            (
                "new on the vault",
                None,
                None,
                Some(a),
                Step::Adopt(Some(a)),
            ),
            (
                "vault recreated",
                Some(a),
                Some(a),
                Some(other),
                Step::Adopt(Some(other)),
            ),
            // Only the local head changed.
            (
                "local moved on",
                Some(b),
                Some(a),
                Some(a),
                Step::Push(Some(b)),
            ),
            ("local removed", None, Some(a), Some(a), Step::Push(None)),
            ("new locally", Some(a), None, None, Step::Push(Some(a))),
            (
                "local recreated",
                Some(other),
                Some(a),
                Some(a),
                Step::Push(Some(other)),
            ),
            ("local moved back", Some(root), Some(a), Some(a), Step::Keep),
            ("local diverged from base", Some(c), Some(a), Some(a), merge),
            // Both changed. An edit beats a delete, and the vault is the
            // incumbent.
            (
                "delete against an edit",
                None,
                Some(a),
                Some(b),
                Step::Adopt(Some(b)),
            ),
            (
                "edit against a delete",
                Some(b),
                Some(a),
                None,
                Step::Push(Some(b)),
            ),
            (
                "vault behind local",
                Some(b),
                Some(root),
                Some(a),
                Step::Push(Some(b)),
            ),
            (
                "local behind vault",
                Some(a),
                Some(root),
                Some(b),
                Step::Adopt(Some(b)),
            ),
            (
                "twins",
                Some(twin_local),
                Some(root),
                Some(twin_remote),
                Step::Adopt(Some(twin_remote)),
            ),
            ("diverged", Some(c), Some(root), Some(a), merge),
            ("unrelated", Some(other), Some(root), Some(a), aside),
            (
                "unrelated on first pairing",
                Some(other),
                None,
                Some(a),
                aside,
            ),
        ] {
            assert_eq!(plan(&commits, local, base, remote), expected, "{label}");
        }
    }
}
