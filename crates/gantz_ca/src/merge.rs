//! Three-way merging of diverged graph heads.
//!
//! [`merge_commits`] resolves the relationship between two commit tips. See
//! [`history::analyze`]. When the tips have diverged, it performs a
//! three-way merge of their graphs against the merge base via
//! [`merge_graphs`].
//!
//! The merge is total. It always produces a merged graph. Situations with no
//! single obvious resolution are recorded as [`Conflict`]s. Each carries the
//! default resolution that was applied. Callers can refuse the result,
//! surface the conflicts, or accept the defaults.

use crate::{
    CommitAddr, DataGraph, Diff, Edge, GraphAddr, Matching, Registry, Timestamp, content_addr,
    diff, history,
};
use petgraph::graph::NodeIndex;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// One side of a merge.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize,
)]
pub enum Side {
    #[default]
    Ours,
    Theirs,
}

/// How a merge resolves each class of conflict. See [`Conflict`].
///
/// An edge added to a node absent from the merged graph is always dropped.
/// That is a consequence of the node's absence, not a choice.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize,
)]
pub struct Resolutions {
    /// Which content is kept when both sides modified the same base node with
    /// different results.
    pub both_modified: BothModified,
    /// Whether the edit or the delete wins when one side deleted a node the
    /// other modified.
    pub delete_modify: EditOrDelete,
}

/// Which content wins a [`Conflict::BothModified`].
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize,
)]
pub enum BothModified {
    /// Ours' content is kept.
    #[default]
    #[serde(alias = "Ours")]
    KeepOurs,
    /// Theirs' content is kept.
    #[serde(alias = "Theirs")]
    KeepTheirs,
    /// Last edit wins, per node. The side whose last content-changing commit
    /// for that node is newer keeps its version. See [`EditTimes`].
    ///
    /// Unlike the sided options this resolution is symmetric. Merging A into
    /// B picks the same content as merging B into A, so two peers resolving
    /// the same conflict independently converge. This is the basis for a
    /// shared-session "last edit wins" mode. Exact time ties break to the
    /// greater content address. That is arbitrary but side-independent.
    KeepNewest,
}

/// Whether an edit or a delete wins a [`Conflict::DeleteModify`].
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize,
)]
pub enum EditOrDelete {
    /// The modified node is kept, so no work is lost.
    #[default]
    KeepEdit,
    /// The node stays deleted.
    KeepDelete,
}

/// Per-node last-edit timestamps consulted by [`BothModified::KeepNewest`].
///
/// [`merge_commits`] fills this from each side's commit chain. See
/// [`diff::matching_with_times`]. A node without an entry falls back to its
/// side's tip timestamp. The `Default` is empty with zero tips. It makes
/// every comparison a tie, so `KeepNewest` degrades to the content-address
/// tie-break.
#[derive(Clone, Debug, Default)]
pub struct EditTimes {
    /// Maps a base node index to the last time ours' chain changed the node.
    pub ours: BTreeMap<usize, Timestamp>,
    /// Maps a base node index to the last time theirs' chain changed the
    /// node.
    pub theirs: BTreeMap<usize, Timestamp>,
    /// Ours' tip commit timestamp. This is the fallback edit time.
    pub ours_tip: Timestamp,
    /// Theirs' tip commit timestamp. This is the fallback edit time.
    pub theirs_tip: Timestamp,
}

/// A conflict encountered during a three-way merge.
///
/// Conflicts are flagged, not fatal. Each records the resolution the merge
/// applied per the caller's [`Resolutions`]. The result remains usable and
/// callers can decide whether to accept it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Conflict {
    /// Both sides modified the same base node with different results.
    ///
    /// The content of `kept` is kept. `ours` and `theirs` are the node's
    /// indices in the respective graphs.
    BothModified {
        base: usize,
        ours: usize,
        theirs: usize,
        kept: Side,
    },
    /// One side deleted the node and the other, `modified`, modified it.
    ///
    /// The modified node is kept when `kept` is true. Otherwise it stays
    /// deleted.
    DeleteModify {
        base: usize,
        modified: Side,
        kept: bool,
    },
    /// `side` added an edge to a node the other side deleted. The node
    /// stayed deleted.
    ///
    /// The edge is dropped. `src` and `dst` are indices in `side`'s graph.
    EdgeToDeleted {
        side: Side,
        src: usize,
        dst: usize,
        edge: Edge,
    },
}

/// The provenance of one merged node. Its index in each of the three input
/// graphs it appears in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeSrc {
    pub base: Option<usize>,
    pub ours: Option<usize>,
    pub theirs: Option<usize>,
}

/// The result of a three-way graph merge.
#[derive(Clone, Debug)]
pub struct MergeOutcome {
    /// The merged graph. Ours' surviving nodes come first in ours' order,
    /// followed by theirs-only nodes in ascending theirs order. When theirs
    /// removed no nodes, ours' indices are preserved exactly.
    pub graph: DataGraph,
    /// The provenance of each merged node, indexed by merged node index.
    pub node_srcs: Vec<NodeSrc>,
    /// The conflicts encountered, each already resolved by its documented
    /// default.
    pub conflicts: Vec<Conflict>,
}

/// The resolution of merging one commit tip into another.
#[derive(Clone, Debug)]
pub enum MergeResolution {
    /// Theirs is an ancestor of ours. There is nothing to merge.
    AlreadyUpToDate,
    /// Ours is an ancestor of theirs. The head can move to theirs' tip
    /// without a merge commit.
    FastForward,
    /// The tips have diverged and a three-way merge was performed.
    Diverged {
        /// The merge base the diffs are relative to.
        ///
        /// For criss-cross histories this is the nominal tie-break candidate.
        /// The diffs and outcome are computed against a virtual base merged
        /// from all candidates. See [`merge_commits`].
        base: CommitAddr,
        /// Ours' changes relative to the base.
        ours_diff: Diff,
        /// Theirs' changes relative to the base.
        theirs_diff: Diff,
        /// The merged graph, provenance and conflicts.
        outcome: MergeOutcome,
    },
}

/// An error preventing a merge from being attempted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeError {
    /// The tips share no common ancestor.
    Unrelated,
    /// A required commit is missing from the registry.
    MissingCommit(CommitAddr),
    /// A required graph is missing from the registry.
    MissingGraph(GraphAddr),
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unrelated => write!(f, "the commits share no common ancestor"),
            Self::MissingCommit(ca) => write!(f, "missing commit {ca}"),
            Self::MissingGraph(ca) => write!(f, "missing graph {ca}"),
        }
    }
}

impl std::error::Error for MergeError {}

/// The fate of a base node in the merged graph.
#[derive(Clone, Copy)]
enum Fate {
    /// The node is absent from the merged graph.
    Delete,
    /// The node survives with ours' content.
    KeepOurs,
    /// The node survives with theirs' content.
    KeepTheirs,
}

/// Maximum recursion depth when constructing virtual merge bases for
/// criss-cross histories. Beyond it the deterministic tie-break candidate is
/// used directly as the base.
const MAX_BASE_RECURSION: usize = 5;

/// Merge two tips of a registry's commit DAG.
///
/// Pure. The registry is not mutated, so this doubles as a dry run for
/// previews. On [`MergeResolution::Diverged`], committing the result is the
/// caller's job. See [`Registry::commit_merge_to_head`].
///
/// Criss-cross histories have multiple best common ancestors. For example,
/// session peers repeatedly merging one another. They are handled git-style.
/// The candidates are recursively merged into a virtual base. Changes both
/// tips already contain via different merge paths are then part of the base
/// rather than duplicated as parallel additions.
pub fn merge_commits(
    reg: &Registry,
    ours: CommitAddr,
    theirs: CommitAddr,
    resolutions: Resolutions,
) -> Result<MergeResolution, MergeError> {
    merge_commits_recursive(reg, ours, theirs, resolutions, 0)
}

/// The implementation of [`merge_commits`]. `depth` guards the virtual-base
/// recursion for criss-cross histories.
fn merge_commits_recursive(
    reg: &Registry,
    ours: CommitAddr,
    theirs: CommitAddr,
    resolutions: Resolutions,
    depth: usize,
) -> Result<MergeResolution, MergeError> {
    let commits = reg.commits();
    // A tip that is itself a common ancestor is always the sole candidate,
    // so the singleton checks cover the analysis. See `history::analyze`.
    let bases = history::merge_bases(commits, ours, theirs);
    match bases.as_slice() {
        [] => return Err(MergeError::Unrelated),
        [base] if *base == theirs => return Ok(MergeResolution::AlreadyUpToDate),
        [base] if *base == ours => return Ok(MergeResolution::FastForward),
        _ => (),
    }
    let ours_g = commit_graph_of(reg, ours)?;
    let theirs_g = commit_graph_of(reg, theirs)?;
    let tip_time = |ca: CommitAddr| commits.get(&ca).map(|c| c.timestamp).unwrap_or_default();
    // The nominal base reported in the resolution is the tie-break candidate.
    let base = *bases.last().expect("diverged tips have a merge base");
    let (ours_diff, theirs_diff, outcome) = if bases.len() > 1 && depth < MAX_BASE_RECURSION {
        // Criss-cross. Merge the candidates into a virtual base. It has no
        // commit chain, so node identity and edit times degrade to direct
        // content matching and tip timestamps. This is coarser but still
        // deterministic, so peers still converge.
        let virt = base_graph(reg, &bases, resolutions, depth)?;
        let edit_times = EditTimes {
            ours: BTreeMap::new(),
            theirs: BTreeMap::new(),
            ours_tip: tip_time(ours),
            theirs_tip: tip_time(theirs),
        };
        merge_graphs_direct(&virt, ours_g, theirs_g, resolutions, &edit_times)
    } else {
        let base_g = commit_graph_of(reg, base)?;
        // Endpoints are verified above, so `matching_with_times` cannot
        // fail. Degrade to direct matching rather than panic should that
        // ever change.
        let (mo, ours_times) = diff::matching_with_times(reg, base, ours)
            .unwrap_or_else(|| (diff::match_nodes(base_g, ours_g), Default::default()));
        let (mt, theirs_times) = diff::matching_with_times(reg, base, theirs)
            .unwrap_or_else(|| (diff::match_nodes(base_g, theirs_g), Default::default()));
        let ours_diff = diff::diff(base_g, ours_g, &mo);
        let theirs_diff = diff::diff(base_g, theirs_g, &mt);
        let edit_times = EditTimes {
            ours: ours_times,
            theirs: theirs_times,
            ours_tip: tip_time(ours),
            theirs_tip: tip_time(theirs),
        };
        let outcome = merge_graphs(
            base_g,
            ours_g,
            theirs_g,
            &ours_diff,
            &theirs_diff,
            resolutions,
            &edit_times,
        );
        (ours_diff, theirs_diff, outcome)
    };
    Ok(MergeResolution::Diverged {
        base,
        ours_diff,
        theirs_diff,
        outcome,
    })
}

/// The graph pointed to by `ca`'s commit.
fn commit_graph_of(reg: &Registry, ca: CommitAddr) -> Result<&DataGraph, MergeError> {
    let commit = reg
        .commits()
        .get(&ca)
        .ok_or(MergeError::MissingCommit(ca))?;
    reg.graphs()
        .get(&commit.graph)
        .ok_or(MergeError::MissingGraph(commit.graph))
}

/// [`merge_graphs`] under diffs computed by direct content matching against
/// `base`. This is the path for virtual bases, which have no commit chain to
/// track node identity along.
fn merge_graphs_direct(
    base: &DataGraph,
    ours: &DataGraph,
    theirs: &DataGraph,
    resolutions: Resolutions,
    edit_times: &EditTimes,
) -> (Diff, Diff, MergeOutcome) {
    let mo = diff::match_nodes(base, ours);
    let mt = diff::match_nodes(base, theirs);
    let ours_diff = diff::diff(base, ours, &mo);
    let theirs_diff = diff::diff(base, theirs, &mt);
    let outcome = merge_graphs(
        base,
        ours,
        theirs,
        &ours_diff,
        &theirs_diff,
        resolutions,
        edit_times,
    );
    (ours_diff, theirs_diff, outcome)
}

/// The merged graph of two commit tips, minting nothing. This is the
/// building block for virtual bases.
fn merged_tip_graph(
    reg: &Registry,
    a: CommitAddr,
    b: CommitAddr,
    resolutions: Resolutions,
    depth: usize,
) -> Result<DataGraph, MergeError> {
    match merge_commits_recursive(reg, a, b, resolutions, depth) {
        Ok(MergeResolution::AlreadyUpToDate) => Ok(commit_graph_of(reg, a)?.clone()),
        Ok(MergeResolution::FastForward) => Ok(commit_graph_of(reg, b)?.clone()),
        Ok(MergeResolution::Diverged { outcome, .. }) => Ok(outcome.graph),
        // Unrelated candidates, such as merged-in foreign roots, merge against
        // an empty base. Everything unions, deterministically.
        Err(MergeError::Unrelated) => {
            let a_g = commit_graph_of(reg, a)?;
            let b_g = commit_graph_of(reg, b)?;
            let empty = DataGraph::default();
            let (_, _, outcome) =
                merge_graphs_direct(&empty, a_g, b_g, resolutions, &EditTimes::default());
            Ok(outcome.graph)
        }
        Err(e) => Err(e),
    }
}

/// The base graph for canonically-sorted merge-base `candidates` at the
/// given recursion `depth`.
///
/// - No candidates. The tips are unrelated. The base is an empty graph, so
///   everything unions.
/// - One candidate. Its graph is the base.
/// - Several candidates within [`MAX_BASE_RECURSION`]. This is a criss-cross.
///   The candidates are merged left to right into a virtual base.
/// - Several candidates at the recursion cap. The deterministic tie-break
///   candidate's graph is the base.
fn base_graph(
    reg: &Registry,
    candidates: &[CommitAddr],
    resolutions: Resolutions,
    depth: usize,
) -> Result<DataGraph, MergeError> {
    match candidates {
        [] => return Ok(DataGraph::default()),
        [only] => return Ok(commit_graph_of(reg, *only)?.clone()),
        _ if depth >= MAX_BASE_RECURSION => {
            let last = *candidates.last().expect("non-empty");
            return Ok(commit_graph_of(reg, last)?.clone());
        }
        _ => (),
    }
    let mut virt = merged_tip_graph(reg, candidates[0], candidates[1], resolutions, depth + 1)?;
    for &c in &candidates[2..] {
        let c_g = commit_graph_of(reg, c)?;
        // The fold step's base is the base of the first candidate and `c`.
        // That base may itself be criss-cross. It is empty when unrelated.
        let pair_bases = history::merge_bases(reg.commits(), candidates[0], c);
        let step_base = base_graph(reg, &pair_bases, resolutions, depth + 1)?;
        let (_, _, outcome) =
            merge_graphs_direct(&step_base, &virt, c_g, resolutions, &EditTimes::default());
        virt = outcome.graph;
    }
    Ok(virt)
}

/// Three-way merge of `ours` and `theirs` against their common `base`, under
/// the diffs produced by [`diff::diff`]. The diffs carry the node matchings.
///
/// Node rules, per base node:
///
/// - Present on both sides and modified by at most one. The modified side's
///   content is kept. Change beats no-change.
/// - Modified by both to the same content. Kept, no conflict.
/// - Modified by both to different content. The content chosen by
///   [`Resolutions::both_modified`] is kept and [`Conflict::BothModified`]
///   is flagged.
/// - Deleted by one side, untouched by the other. Deleted.
/// - Deleted by one side, modified by the other. Resolved per
///   [`Resolutions::delete_modify`] and [`Conflict::DeleteModify`] is
///   flagged.
///
/// Nodes added by a side are always included. Ours' surviving nodes come
/// first in ours' order, then theirs-only nodes in ascending theirs order.
/// Ours' indices are thus preserved exactly whenever theirs removed nothing.
///
/// Edge rules, on `(source, target, weight)` sets:
///
/// - A base edge survives unless a side removed it or an endpoint is absent
///   from the merged graph.
/// - Added edges from both sides are unioned. Identical additions collapse.
/// - An edge added to a node that is absent from the merged graph is dropped
///   and [`Conflict::EdgeToDeleted`] is flagged.
///
/// Construction order is deterministic, so merging the same inputs always
/// yields the same graph address.
pub fn merge_graphs(
    base: &DataGraph,
    ours: &DataGraph,
    theirs: &DataGraph,
    ours_diff: &Diff,
    theirs_diff: &Diff,
    resolutions: Resolutions,
    edit_times: &EditTimes,
) -> MergeOutcome {
    let node_ix = NodeIndex::<usize>::new;
    let mut conflicts = Vec::new();
    let keep_edit = resolutions.delete_modify == EditOrDelete::KeepEdit;

    // Decide each base node's fate.
    let mut fates: BTreeMap<usize, Fate> = BTreeMap::new();
    for b in 0..base.node_count() {
        let o = ours_diff.matched.get(&b).copied();
        let t = theirs_diff.matched.get(&b).copied();
        let mod_o = ours_diff.modified.contains(&b);
        let mod_t = theirs_diff.modified.contains(&b);
        let fate = match (o, t) {
            (Some(o), Some(t)) => match (mod_o, mod_t) {
                (_, false) => Fate::KeepOurs,
                (false, true) => Fate::KeepTheirs,
                (true, true) => {
                    let o_ca = content_addr(&ours[node_ix(o)]);
                    let t_ca = content_addr(&theirs[node_ix(t)]);
                    let kept = match resolutions.both_modified {
                        BothModified::KeepOurs => Side::Ours,
                        BothModified::KeepTheirs => Side::Theirs,
                        // Per-node last edit wins. Both orderings of the same
                        // merge compare identical values. The time maps swap
                        // sides but keep their entries, and the tie-break is
                        // on content. Independent peers therefore converge.
                        BothModified::KeepNewest => {
                            let ot = edit_times
                                .ours
                                .get(&b)
                                .copied()
                                .unwrap_or(edit_times.ours_tip);
                            let tt = edit_times
                                .theirs
                                .get(&b)
                                .copied()
                                .unwrap_or(edit_times.theirs_tip);
                            match (tt.cmp(&ot), t_ca > o_ca) {
                                (std::cmp::Ordering::Greater, _) => Side::Theirs,
                                (std::cmp::Ordering::Less, _) => Side::Ours,
                                (std::cmp::Ordering::Equal, true) => Side::Theirs,
                                (std::cmp::Ordering::Equal, false) => Side::Ours,
                            }
                        }
                    };
                    if o_ca != t_ca {
                        conflicts.push(Conflict::BothModified {
                            base: b,
                            ours: o,
                            theirs: t,
                            kept,
                        });
                    }
                    match kept {
                        Side::Ours => Fate::KeepOurs,
                        Side::Theirs => Fate::KeepTheirs,
                    }
                }
            },
            (Some(_), None) if mod_o => {
                conflicts.push(Conflict::DeleteModify {
                    base: b,
                    modified: Side::Ours,
                    kept: keep_edit,
                });
                if keep_edit {
                    Fate::KeepOurs
                } else {
                    Fate::Delete
                }
            }
            (None, Some(_)) if mod_t => {
                conflicts.push(Conflict::DeleteModify {
                    base: b,
                    modified: Side::Theirs,
                    kept: keep_edit,
                });
                if keep_edit {
                    Fate::KeepTheirs
                } else {
                    Fate::Delete
                }
            }
            _ => Fate::Delete,
        };
        fates.insert(b, fate);
    }

    // Ours' surviving nodes, in ours' order.
    let inv_ours: Matching = ours_diff.matched.iter().map(|(&b, &o)| (o, b)).collect();
    let inv_theirs: Matching = theirs_diff.matched.iter().map(|(&b, &t)| (t, b)).collect();
    let mut graph = DataGraph::default();
    let mut node_srcs: Vec<NodeSrc> = Vec::new();
    let mut merged_of_ours: BTreeMap<usize, usize> = BTreeMap::new();
    let mut merged_of_theirs: BTreeMap<usize, usize> = BTreeMap::new();
    for o in 0..ours.node_count() {
        let (weight, src) = match inv_ours.get(&o) {
            // A node matched from base. Its fate decides.
            Some(&b) => {
                let t = theirs_diff.matched.get(&b).copied();
                let src = NodeSrc {
                    base: Some(b),
                    ours: Some(o),
                    theirs: t,
                };
                match fates[&b] {
                    Fate::Delete => continue,
                    Fate::KeepOurs => (ours[node_ix(o)].clone(), src),
                    Fate::KeepTheirs => {
                        let t = t.expect("`KeepTheirs` fate requires a theirs match");
                        (theirs[node_ix(t)].clone(), src)
                    }
                }
            }
            // A node added by ours.
            None => {
                let src = NodeSrc {
                    base: None,
                    ours: Some(o),
                    theirs: None,
                };
                (ours[node_ix(o)].clone(), src)
            }
        };
        let m = graph.add_node(weight).index();
        node_srcs.push(src);
        merged_of_ours.insert(o, m);
        if let Some(t) = src.theirs {
            merged_of_theirs.insert(t, m);
        }
    }
    // Theirs-only survivors, in theirs' order. These are nodes added by
    // theirs, and nodes theirs modified but ours deleted and `DeleteModify`
    // kept.
    for t in 0..theirs.node_count() {
        if merged_of_theirs.contains_key(&t) {
            continue;
        }
        let src = match inv_theirs.get(&t) {
            Some(&b) => match fates[&b] {
                Fate::KeepTheirs => NodeSrc {
                    base: Some(b),
                    ours: None,
                    theirs: Some(t),
                },
                // Deleted, or already added via ours above.
                _ => continue,
            },
            None => NodeSrc {
                base: None,
                ours: None,
                theirs: Some(t),
            },
        };
        let m = graph.add_node(theirs[node_ix(t)].clone()).index();
        node_srcs.push(src);
        merged_of_theirs.insert(t, m);
    }

    // The merged index of a base node, if it survived via either side.
    let base_merged = |b: usize| -> Option<usize> {
        let via_ours = ours_diff
            .matched
            .get(&b)
            .and_then(|o| merged_of_ours.get(o));
        let via_theirs = theirs_diff
            .matched
            .get(&b)
            .and_then(|t| merged_of_theirs.get(t));
        via_ours.or(via_theirs).copied()
    };

    // Base edges survive unless a side removed them or an endpoint is gone.
    let mut merged_edges: BTreeSet<(usize, usize, Edge)> = BTreeSet::new();
    for (s, d, w) in diff::edge_set(base) {
        let removed = ours_diff.removed_edges.contains(&(s, d, w))
            || theirs_diff.removed_edges.contains(&(s, d, w));
        if removed {
            continue;
        }
        // An absent endpoint means the edge is implied-removed with its node.
        let (Some(ms), Some(md)) = (base_merged(s), base_merged(d)) else {
            continue;
        };
        merged_edges.insert((ms, md, w));
    }
    // Union in each side's added edges. Identical additions collapse.
    let mut add_edges =
        |added: &BTreeSet<(usize, usize, Edge)>, merged_of: &BTreeMap<usize, usize>, side: Side| {
            for &(s, d, w) in added {
                match (merged_of.get(&s), merged_of.get(&d)) {
                    (Some(&ms), Some(&md)) => {
                        merged_edges.insert((ms, md, w));
                    }
                    _ => conflicts.push(Conflict::EdgeToDeleted {
                        side,
                        src: s,
                        dst: d,
                        edge: w,
                    }),
                }
            }
        };
    add_edges(&ours_diff.added_edges, &merged_of_ours, Side::Ours);
    add_edges(&theirs_diff.added_edges, &merged_of_theirs, Side::Theirs);
    for (s, d, w) in merged_edges {
        graph.add_edge(node_ix(s), node_ix(d), w);
    }

    MergeOutcome {
        graph,
        node_srcs,
        conflicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Commit, Datum, Head, NodeData, graph_addr};
    use std::time::Duration;

    fn graph(nodes: &[&str], edges: &[(usize, usize, u16)]) -> DataGraph {
        let mut g = DataGraph::default();
        for n in nodes {
            g.add_node(NodeData::new(*n, Datum::Map(vec![])));
        }
        for &(s, d, w) in edges {
            g.add_edge(s.into(), d.into(), edge(w));
        }
        g
    }

    fn edge(w: u16) -> Edge {
        Edge::from((w, 0u16))
    }

    fn commit(
        reg: &mut Registry,
        secs: u64,
        parent: Option<CommitAddr>,
        g: &DataGraph,
    ) -> CommitAddr {
        reg.commit_graph(Duration::from_secs(secs), parent, graph_addr(g), || {
            g.clone()
        })
    }

    fn nodes(g: &DataGraph) -> Vec<&str> {
        g.node_weights().map(|n| n.tag.as_str()).collect()
    }

    fn edges(g: &DataGraph) -> BTreeSet<(usize, usize, u16)> {
        diff::edge_set(g)
            .into_iter()
            .map(|(s, d, e)| (s, d, e.output.0))
            .collect()
    }

    /// Merge two graphs that diverged from `base` by one commit each, using
    /// the default [`Resolutions`].
    fn merge_two(
        base: &DataGraph,
        ours: &DataGraph,
        theirs: &DataGraph,
    ) -> (Registry, CommitAddr, CommitAddr, MergeResolution) {
        merge_two_with(base, ours, theirs, Resolutions::default())
    }

    /// [`merge_two`] with explicit [`Resolutions`].
    fn merge_two_with(
        base: &DataGraph,
        ours: &DataGraph,
        theirs: &DataGraph,
        resolutions: Resolutions,
    ) -> (Registry, CommitAddr, CommitAddr, MergeResolution) {
        let mut reg = Registry::default();
        let b = commit(&mut reg, 1, None, base);
        let o = commit(&mut reg, 2, Some(b), ours);
        let t = commit(&mut reg, 3, Some(b), theirs);
        let res = merge_commits(&reg, o, t, resolutions).unwrap();
        (reg, o, t, res)
    }

    fn diverged(res: MergeResolution) -> MergeOutcome {
        match res {
            MergeResolution::Diverged { outcome, .. } => outcome,
            other => panic!("expected Diverged, got {other:?}"),
        }
    }

    #[test]
    fn criss_cross_merges_via_virtual_base_without_duplication() {
        // Each side merged the other's tip with differing merge commits, as
        // manual merges produce. Both best common ancestors {a, b} predate
        // the shared additions x and y. A single tie-break base would see x
        // or y as an addition on both sides and union it twice. The virtual
        // base already contains both.
        let mut reg = Registry::default();
        let root = commit(&mut reg, 1, None, &graph(&["n"], &[]));
        let ga = graph(&["n", "x"], &[]);
        let gb = graph(&["n", "y"], &[]);
        let a = commit(&mut reg, 2, Some(root), &ga);
        let b = commit(&mut reg, 3, Some(root), &gb);
        let gab = graph(&["n", "x", "y"], &[]);
        let gba = graph(&["n", "y", "x"], &[]);
        reg.add_graph(gab.clone());
        reg.add_graph(gba.clone());
        let mab = reg.add_commit(Commit::new_merge(
            Duration::from_secs(4),
            a,
            b,
            graph_addr(&gab),
        ));
        let mba = reg.add_commit(Commit::new_merge(
            Duration::from_secs(5),
            b,
            a,
            graph_addr(&gba),
        ));
        let res = merge_commits(&reg, mab, mba, Resolutions::default()).unwrap();
        let out = diverged(res);
        assert!(out.conflicts.is_empty());
        // Ours' order, exactly one of each. No duplicated x or y.
        assert_eq!(nodes(&out.graph), vec!["n", "x", "y"]);
    }

    #[test]
    fn disjoint_additions_union() {
        let base = graph(&["a"], &[]);
        let ours = graph(&["a", "x"], &[]);
        let theirs = graph(&["a", "y"], &[]);
        let (_, _, _, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert!(out.conflicts.is_empty());
        assert_eq!(nodes(&out.graph), vec!["a", "x", "y"]);
        assert_eq!(
            out.node_srcs,
            vec![
                NodeSrc {
                    base: Some(0),
                    ours: Some(0),
                    theirs: Some(0)
                },
                NodeSrc {
                    base: None,
                    ours: Some(1),
                    theirs: None
                },
                NodeSrc {
                    base: None,
                    ours: None,
                    theirs: Some(1)
                },
            ],
        );
    }

    /// The collaborative-editing driver scenario. One side edits a node's
    /// content while the other connects an edge to it. Chain-tracked identity
    /// makes this a clean merge.
    #[test]
    fn content_edit_and_edge_add_merge_cleanly() {
        let base = graph(&["a", "b"], &[]);
        let ours = graph(&["a", "b2"], &[]);
        let theirs = graph(&["a", "b"], &[(0, 1, 0)]);
        let (_, _, _, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert!(out.conflicts.is_empty());
        assert_eq!(nodes(&out.graph), vec!["a", "b2"]);
        assert_eq!(edges(&out.graph), BTreeSet::from([(0, 1, 0)]));
    }

    #[test]
    fn both_modified_differently_keeps_ours_and_flags() {
        let base = graph(&["a", "b"], &[]);
        let ours = graph(&["a", "b2"], &[]);
        let theirs = graph(&["a", "b3"], &[]);
        let (_, _, _, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert_eq!(nodes(&out.graph), vec!["a", "b2"]);
        assert_eq!(
            out.conflicts,
            vec![Conflict::BothModified {
                base: 1,
                ours: 1,
                theirs: 1,
                kept: Side::Ours,
            }],
        );
    }

    #[test]
    fn both_modified_resolves_to_theirs_when_asked() {
        let base = graph(&["a", "b"], &[]);
        let ours = graph(&["a", "b2"], &[]);
        let theirs = graph(&["a", "b3"], &[]);
        let resolutions = Resolutions {
            both_modified: BothModified::KeepTheirs,
            ..Default::default()
        };
        let (_, _, _, res) = merge_two_with(&base, &ours, &theirs, resolutions);
        let out = diverged(res);
        assert_eq!(nodes(&out.graph), vec!["a", "b3"]);
        assert_eq!(
            out.conflicts,
            vec![Conflict::BothModified {
                base: 1,
                ours: 1,
                theirs: 1,
                kept: Side::Theirs,
            }],
        );
    }

    /// Per-node last edit wins. Ours edited node 0 at t=2 then node 1 at
    /// t=5, so ours has the newer tip. Theirs edited node 0 at t=3. Node 0's
    /// conflict resolves to theirs, the side whose last edit to that node is
    /// newer, even though ours' tip is newer overall.
    #[test]
    fn keep_newest_resolves_per_node_not_per_tip() {
        let resolutions = Resolutions {
            both_modified: BothModified::KeepNewest,
            ..Default::default()
        };
        let mut reg = Registry::default();
        let base = commit(&mut reg, 1, None, &graph(&["x", "y"], &[]));
        let o1 = commit(&mut reg, 2, Some(base), &graph(&["x2", "y"], &[]));
        let ours = commit(&mut reg, 5, Some(o1), &graph(&["x2", "y2"], &[]));
        let theirs = commit(&mut reg, 3, Some(base), &graph(&["x3", "y"], &[]));
        let out = diverged(merge_commits(&reg, ours, theirs, resolutions).unwrap());
        assert_eq!(nodes(&out.graph), vec!["x3", "y2"]);
        assert_eq!(
            out.conflicts,
            vec![Conflict::BothModified {
                base: 0,
                ours: 0,
                theirs: 0,
                kept: Side::Theirs,
            }],
        );
    }

    /// `KeepNewest` is symmetric. Merging in either direction keeps the same
    /// content, including on exact time ties via the content-address
    /// tie-break. Independent peers converge on the same merged graph.
    #[test]
    fn keep_newest_is_symmetric() {
        let resolutions = Resolutions {
            both_modified: BothModified::KeepNewest,
            ..Default::default()
        };
        let mut reg = Registry::default();
        let base = commit(&mut reg, 1, None, &graph(&["x"], &[]));
        // Both sides edit the node at the same timestamp.
        let a = commit(&mut reg, 2, Some(base), &graph(&["x-a"], &[]));
        let b = commit(&mut reg, 2, Some(base), &graph(&["x-b"], &[]));
        let ab = diverged(merge_commits(&reg, a, b, resolutions).unwrap());
        let ba = diverged(merge_commits(&reg, b, a, resolutions).unwrap());
        assert_eq!(graph_addr(&ab.graph), graph_addr(&ba.graph));
        assert_eq!(ab.conflicts.len(), 1);
        assert_eq!(ba.conflicts.len(), 1);
    }

    #[test]
    fn both_modified_identically_is_clean() {
        let base = graph(&["a", "b"], &[]);
        let ours = graph(&["a", "b2"], &[]);
        let theirs = graph(&["a", "b2"], &[]);
        let (_, _, _, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert!(out.conflicts.is_empty());
        assert_eq!(nodes(&out.graph), vec!["a", "b2"]);
    }

    #[test]
    fn delete_vs_modify_keeps_the_modified_node() {
        let base = graph(&["a", "b"], &[]);
        // Ours deletes ix 1 and theirs modifies it.
        let ours = graph(&["a"], &[]);
        let theirs = graph(&["a", "b2"], &[]);
        let (_, _, _, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert_eq!(nodes(&out.graph), vec!["a", "b2"]);
        assert_eq!(
            out.conflicts,
            vec![Conflict::DeleteModify {
                base: 1,
                modified: Side::Theirs,
                kept: true,
            }],
        );
        assert_eq!(
            out.node_srcs[1],
            NodeSrc {
                base: Some(1),
                ours: None,
                theirs: Some(1)
            },
        );
    }

    #[test]
    fn delete_vs_modify_deletes_when_asked() {
        let base = graph(&["a", "b"], &[]);
        // Ours deletes ix 1. Theirs modifies it and wires into it.
        let ours = graph(&["a"], &[]);
        let theirs = graph(&["a", "b2"], &[(0, 1, 0)]);
        let resolutions = Resolutions {
            delete_modify: EditOrDelete::KeepDelete,
            ..Default::default()
        };
        let (_, _, _, res) = merge_two_with(&base, &ours, &theirs, resolutions);
        let out = diverged(res);
        // The delete wins. Theirs' edge into the node dangles and drops.
        assert_eq!(nodes(&out.graph), vec!["a"]);
        assert!(edges(&out.graph).is_empty());
        assert_eq!(
            out.conflicts,
            vec![
                Conflict::DeleteModify {
                    base: 1,
                    modified: Side::Theirs,
                    kept: false,
                },
                Conflict::EdgeToDeleted {
                    side: Side::Theirs,
                    src: 0,
                    dst: 1,
                    edge: edge(0),
                },
            ],
        );
    }

    #[test]
    fn delete_vs_untouched_deletes() {
        let base = graph(&["a", "b"], &[]);
        let ours = graph(&["a"], &[]);
        let theirs = graph(&["a", "b"], &[]);
        let (_, _, _, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert!(out.conflicts.is_empty());
        assert_eq!(nodes(&out.graph), vec!["a"]);
    }

    #[test]
    fn edge_to_deleted_node_is_dropped_and_flagged() {
        let base = graph(&["a", "b"], &[]);
        // Ours deletes ix 1, which theirs left untouched. Theirs wires into it.
        let ours = graph(&["a"], &[]);
        let theirs = graph(&["a", "b"], &[(0, 1, 0)]);
        let (_, _, _, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert_eq!(nodes(&out.graph), vec!["a"]);
        assert!(edges(&out.graph).is_empty());
        assert_eq!(
            out.conflicts,
            vec![Conflict::EdgeToDeleted {
                side: Side::Theirs,
                src: 0,
                dst: 1,
                edge: edge(0)
            }],
        );
    }

    #[test]
    fn identical_edge_additions_collapse() {
        let base = graph(&["a", "b"], &[]);
        let ours = graph(&["a", "b"], &[(0, 1, 0)]);
        let theirs = graph(&["a", "b"], &[(0, 1, 0)]);
        let (_, _, _, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert!(out.conflicts.is_empty());
        assert_eq!(edges(&out.graph), BTreeSet::from([(0, 1, 0)]));
        assert_eq!(out.graph.edge_count(), 1);
    }

    #[test]
    fn distinct_parallel_edge_additions_are_both_kept() {
        let base = graph(&["a", "b"], &[]);
        let ours = graph(&["a", "b"], &[(0, 1, 0)]);
        let theirs = graph(&["a", "b"], &[(0, 1, 1)]);
        let (_, _, _, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert!(out.conflicts.is_empty());
        assert_eq!(edges(&out.graph), BTreeSet::from([(0, 1, 0), (0, 1, 1)]));
    }

    #[test]
    fn edge_removed_by_one_side_stays_removed() {
        let base = graph(&["a", "b"], &[(0, 1, 0)]);
        let ours = graph(&["a", "b"], &[]);
        let theirs = graph(&["a", "b", "c"], &[(0, 1, 0)]);
        let (_, _, _, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert!(out.conflicts.is_empty());
        assert_eq!(nodes(&out.graph), vec!["a", "b", "c"]);
        assert!(edges(&out.graph).is_empty());
    }

    #[test]
    fn merge_is_deterministic() {
        let base = graph(&["a", "b"], &[(0, 1, 0)]);
        let ours = graph(&["a", "b", "x"], &[(0, 1, 0), (0, 2, 1)]);
        let theirs = graph(&["a", "b2", "y"], &[(0, 1, 0), (2, 1, 2)]);
        let (_, o, t, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        let (reg2, _, _, _) = merge_two(&base, &ours, &theirs);
        let res2 = merge_commits(&reg2, o, t, Resolutions::default()).unwrap();
        let out2 = diverged(res2);
        assert_eq!(graph_addr(&out.graph), graph_addr(&out2.graph));
    }

    #[test]
    fn fast_forward_and_up_to_date_and_unrelated() {
        let mut reg = Registry::default();
        let g0 = graph(&["a"], &[]);
        let g1 = graph(&["a", "b"], &[]);
        let root = commit(&mut reg, 1, None, &g0);
        let tip = commit(&mut reg, 2, Some(root), &g1);
        let rs = Resolutions::default();
        assert!(matches!(
            merge_commits(&reg, root, tip, rs),
            Ok(MergeResolution::FastForward)
        ));
        assert!(matches!(
            merge_commits(&reg, tip, root, rs),
            Ok(MergeResolution::AlreadyUpToDate)
        ));
        let stray = commit(&mut reg, 3, None, &g1);
        assert!(matches!(
            merge_commits(&reg, tip, stray, rs),
            Err(MergeError::Unrelated)
        ));
    }

    /// End-to-end. Merge two diverged branches and commit the result. The
    /// merge commit's ancestry spans both sides, while undo's first-parent
    /// walk lands on ours' pre-merge tip.
    #[test]
    fn merge_commit_end_to_end() {
        let base = graph(&["a"], &[]);
        let ours = graph(&["a", "x"], &[]);
        let theirs = graph(&["a", "y"], &[]);
        let (mut reg, o, t, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert!(out.conflicts.is_empty());
        reg.set_head("alpha".parse().unwrap(), o);
        let mut head = Head::Branch("alpha".parse().unwrap());
        let merge_ca = reg.commit_merge_to_head(
            Duration::from_secs(4),
            graph_addr(&out.graph),
            || out.graph.clone(),
            t,
            &mut head,
        );
        let ancestors: BTreeSet<_> = history::ancestors(reg.commits(), merge_ca).collect();
        assert!(ancestors.contains(&o) && ancestors.contains(&t));
        assert_eq!(reg.commits()[&merge_ca].parent, Some(o));
        // The merged graph is now reachable via the head.
        assert_eq!(nodes(reg.head_graph(&head).unwrap()), vec!["a", "x", "y"]);
    }
}
