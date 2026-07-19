//! Merge-candidate detection and dry-run previews for the graph config pane.
//!
//! The core three-way merge lives in [`gantz_ca::merge`]; this module wraps it
//! with the GUI-facing queries: which named graphs *can* be merged into the
//! current head ([`merge_candidates`]), and what a given merge would do
//! ([`merge_preview`]). Both are pure reads, so the config pane can call them
//! while rendering; the mutating op is [`crate::ops::merge_head`].

use crate::cycle::named_ref_of;
use gantz_ca as ca;
use gantz_ca::DataGraph;

/// A named graph that can be merged into the current head.
#[derive(Clone, Debug)]
pub struct MergeCandidate {
    /// The source branch name.
    pub name: String,
    /// The source branch's tip commit.
    pub theirs: ca::CommitAddr,
    /// The merge base shared with the current head's tip.
    pub base: ca::CommitAddr,
    /// The current head has no changes since the base: merging just moves the
    /// head to `theirs`, with no merge commit.
    pub fast_forward: bool,
}

/// A dry-run summary of merging one candidate, for hover previews.
///
/// Cached by the config pane in egui temp memory keyed by the two tips, which
/// are content addresses - a pair's preview can never go stale.
#[derive(Clone, Debug)]
pub struct MergePreview {
    /// The changes the merge would bring in (the source branch's changes
    /// relative to the merge base; for a fast-forward, relative to ours' tip).
    pub summary: ca::DiffSummary,
    /// Conflicts, rendered for display. Merging despite these applies the
    /// default resolutions (see [`gantz_ca::merge::Conflict`]).
    pub conflicts: Vec<String>,
    /// Hard blockers, rendered for display: problems (e.g. reference cycles)
    /// that prevent the merge entirely.
    pub blockers: Vec<String>,
}

impl MergePreview {
    /// Whether the merge can proceed as-is (no conflicts, no blockers).
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty() && self.blockers.is_empty()
    }
}

/// The named graphs that can be merged into `ours`: those whose tip shares an
/// ancestor with ours' tip and has changes ours lacks.
///
/// Skips ours' own name and nested (`parent:child`) graphs. Candidates are
/// ordered by name (the registry's name order).
pub fn merge_candidates(reg: &ca::Registry, ours: &ca::Head) -> Vec<MergeCandidate> {
    let Some(ours_tip) = reg.head_commit_ca(ours) else {
        return vec![];
    };
    let our_name = match ours {
        ca::Head::Branch(name) => Some(name),
        ca::Head::Commit(_) => None,
    };
    let commits = reg.commits();
    reg.heads()
        .filter(|(name, _)| Some(*name) != our_name)
        .filter(|(name, _)| !name.is_nested())
        .filter_map(|(name, theirs)| {
            let (base, fast_forward) = match ca::analyze(commits, ours_tip, theirs) {
                ca::MergeAnalysis::Diverged(base) => (base, false),
                ca::MergeAnalysis::FastForward => (ours_tip, true),
                ca::MergeAnalysis::AlreadyUpToDate | ca::MergeAnalysis::Unrelated => return None,
            };
            Some(MergeCandidate {
                name: name.to_string(),
                theirs,
                base,
                fast_forward,
            })
        })
        .collect()
}

/// Dry-run the merge of the branch named `source` into `ours` (see
/// [`gantz_ca::merge_commits`]).
///
/// Returns `None` when there is nothing to merge (unknown source, unrelated or
/// already-up-to-date histories, or missing registry data).
pub fn merge_preview(
    reg: &ca::Registry,
    ours: &ca::Head,
    source: &str,
    resolutions: ca::Resolutions,
) -> Option<MergePreview> {
    let ours_tip = reg.head_commit_ca(ours)?;
    let theirs_tip = reg.head(&source.parse().expect("infallible"))?;
    match ca::merge_commits(reg, ours_tip, theirs_tip, resolutions).ok()? {
        ca::MergeResolution::Diverged {
            theirs_diff,
            outcome,
            ..
        } => Some(MergePreview {
            summary: theirs_diff.summary(),
            conflicts: conflict_strings(&outcome.conflicts),
            blockers: merge_blockers(reg, ours, &outcome.graph),
        }),
        // A fast-forward brings in exactly theirs' changes since ours' tip.
        ca::MergeResolution::FastForward => {
            let matching = ca::diff::matching(reg, ours_tip, theirs_tip)?;
            let ours_g = reg.commit_graph_ref(&ours_tip)?;
            let theirs_g = reg.commit_graph_ref(&theirs_tip)?;
            let diff = ca::diff::diff(ours_g, theirs_g, &matching);
            Some(MergePreview {
                summary: diff.summary(),
                conflicts: vec![],
                blockers: merge_blockers(reg, ours, theirs_g),
            })
        }
        ca::MergeResolution::AlreadyUpToDate => None,
    }
}

/// Render a [`ca::DiffSummary`] as a compact one-line change summary, e.g.
/// `"+2 nodes  -1 node  ~1 modified  +3/-1 edges"`.
pub fn summary_text(s: &ca::DiffSummary) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut parts = Vec::new();
    if s.nodes_added > 0 {
        parts.push(format!("+{} node{}", s.nodes_added, plural(s.nodes_added)));
    }
    if s.nodes_removed > 0 {
        parts.push(format!(
            "-{} node{}",
            s.nodes_removed,
            plural(s.nodes_removed)
        ));
    }
    if s.nodes_modified > 0 {
        parts.push(format!("~{} modified", s.nodes_modified));
    }
    match (s.edges_added, s.edges_removed) {
        (0, 0) => (),
        (a, 0) => parts.push(format!("+{a} edge{}", plural(a))),
        (0, r) => parts.push(format!("-{r} edge{}", plural(r))),
        (a, r) => parts.push(format!("+{a}/-{r} edges")),
    }
    if parts.is_empty() {
        "no structural changes".to_string()
    } else {
        parts.join("  ")
    }
}

/// Render merge conflicts for display, phrased from the current head's
/// perspective ("here" = ours, "the branch" = theirs). Each line names the
/// resolution the merge applied (per the selected [`ca::Resolutions`]).
pub fn conflict_strings(conflicts: &[ca::Conflict]) -> Vec<String> {
    conflicts
        .iter()
        .map(|conflict| match conflict {
            ca::Conflict::BothModified { ours, kept, .. } => {
                let kept = match kept {
                    ca::Side::Ours => "keeps this graph's version",
                    ca::Side::Theirs => "keeps the branch's version",
                };
                format!("node {ours}: modified on both sides ({kept})")
            }
            &ca::Conflict::DeleteModify { modified, kept, .. } => {
                let what = match modified {
                    ca::Side::Ours => "a node modified here was deleted in the branch",
                    ca::Side::Theirs => "a node deleted here was modified in the branch",
                };
                let kept = if kept { "kept" } else { "stays deleted" };
                format!("{what} ({kept})")
            }
            ca::Conflict::EdgeToDeleted {
                side: ca::Side::Ours,
                src,
                dst,
                ..
            } => format!("edge {src}\u{2192}{dst} targets a node deleted in the branch (dropped)"),
            ca::Conflict::EdgeToDeleted {
                side: ca::Side::Theirs,
                src,
                dst,
                ..
            } => format!("branch edge {src}\u{2192}{dst} targets a node deleted here (dropped)"),
        })
        .collect()
}

/// Hard blockers preventing a merge of `merged` into `ours` entirely,
/// regardless of conflict resolution.
///
/// Currently one class: a merged-in [`crate::node::NamedRef`] that would form
/// a reference cycle back to the edited graph (mirroring the guard in
/// [`crate::ops::paste`]; with sync enabled such a cycle recommits endlessly).
pub fn merge_blockers(reg: &ca::Registry, ours: &ca::Head, merged: &DataGraph) -> Vec<String> {
    let ca::Head::Branch(editing) = ours else {
        // A nameless (detached commit) head can't be a name-based cycle target.
        return vec![];
    };
    merged
        .node_weights()
        .filter_map(named_ref_of)
        .filter(|nr| crate::cycle::would_cycle(reg, nr.name(), editing))
        .map(|nr| format!("'{}' would create a reference cycle", nr.name()))
        .collect()
}
