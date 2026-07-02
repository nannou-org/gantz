//! Three-way merging of diverged graph heads.
//!
//! [`merge_commits`] resolves the relationship between two commit tips (see
//! [`history::analyze`]) and, when they have truly diverged, performs a
//! three-way merge of their graphs against the merge base via
//! [`merge_graphs`].
//!
//! The merge is *total*: it always produces a merged graph. Situations with
//! no single obvious resolution are recorded as [`Conflict`]s, each carrying
//! the default resolution that was applied, so callers can refuse the result,
//! surface the conflicts, or accept the defaults.

use crate::{
    CaHash, CommitAddr, Diff, GraphAddr, Matching, MergeAnalysis, Registry, content_addr, diff,
    history,
};
use petgraph::{
    Directed,
    graph::{IndexType, NodeIndex},
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

type Graph<N, E, Ix> = petgraph::graph::Graph<N, E, Directed, Ix>;

/// One side of a merge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Ours,
    Theirs,
}

/// A conflict encountered during a three-way merge.
///
/// Conflicts are flagged, not fatal: each records the default resolution the
/// merge applied so that the result remains usable and callers can decide
/// whether to accept it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Conflict<E> {
    /// Both sides modified the same base node with different results.
    ///
    /// Applied resolution: ours' content is kept. `ours`/`theirs` are the
    /// node's indices in the respective graphs.
    BothModified {
        base: usize,
        ours: usize,
        theirs: usize,
    },
    /// One side deleted the node, the other modified it.
    ///
    /// Applied resolution: the modified node is kept (an edit wins over a
    /// delete).
    DeleteModify { base: usize, modified: Side },
    /// `side` added an edge to a node the other side deleted (and which
    /// stayed deleted).
    ///
    /// Applied resolution: the edge is dropped. `src`/`dst` are indices in
    /// `side`'s graph.
    EdgeToDeleted {
        side: Side,
        src: usize,
        dst: usize,
        edge: E,
    },
}

/// The provenance of one merged node: its index in each of the three input
/// graphs it appears in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeSrc {
    pub base: Option<usize>,
    pub ours: Option<usize>,
    pub theirs: Option<usize>,
}

/// The result of a three-way graph merge.
#[derive(Clone, Debug)]
pub struct MergeOutcome<N, E, Ix: IndexType> {
    /// The merged graph: ours' surviving nodes in ours' order, followed by
    /// theirs-only nodes in ascending theirs order. When theirs removed no
    /// nodes, ours' indices are preserved exactly.
    pub graph: Graph<N, E, Ix>,
    /// The provenance of each merged node, indexed by merged node index.
    pub node_srcs: Vec<NodeSrc>,
    /// The conflicts encountered, each already resolved by its documented
    /// default.
    pub conflicts: Vec<Conflict<E>>,
}

/// The resolution of merging one commit tip into another.
#[derive(Clone, Debug)]
pub enum MergeResolution<N, E, Ix: IndexType> {
    /// Theirs is an ancestor of ours: there is nothing to merge.
    AlreadyUpToDate,
    /// Ours is an ancestor of theirs: the head can simply move to theirs'
    /// tip; no merge commit is required.
    FastForward,
    /// The tips have diverged and a three-way merge was performed.
    Diverged {
        /// The merge base the diffs are relative to.
        base: CommitAddr,
        /// Ours' changes relative to the base.
        ours_diff: Diff<E>,
        /// Theirs' changes relative to the base.
        theirs_diff: Diff<E>,
        /// The merged graph, provenance and conflicts.
        outcome: MergeOutcome<N, E, Ix>,
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

/// Merge two tips of a registry's commit DAG.
///
/// Pure: the registry is not mutated, so this doubles as a dry run for
/// previews. On [`MergeResolution::Diverged`], committing the result is the
/// caller's job (see [`Registry::commit_merge_to_head`]).
pub fn merge_commits<N, E, Ix>(
    reg: &Registry<Graph<N, E, Ix>>,
    ours: CommitAddr,
    theirs: CommitAddr,
) -> Result<MergeResolution<N, E, Ix>, MergeError>
where
    N: Clone + CaHash,
    E: Clone + Ord,
    Ix: IndexType,
{
    let commits = reg.commits();
    let base = match history::analyze(commits, ours, theirs) {
        MergeAnalysis::Unrelated => return Err(MergeError::Unrelated),
        MergeAnalysis::AlreadyUpToDate => return Ok(MergeResolution::AlreadyUpToDate),
        MergeAnalysis::FastForward => return Ok(MergeResolution::FastForward),
        MergeAnalysis::Diverged(base) => base,
    };
    let graph_of = |ca: CommitAddr| {
        let commit = commits.get(&ca).ok_or(MergeError::MissingCommit(ca))?;
        reg.graphs()
            .get(&commit.graph)
            .ok_or(MergeError::MissingGraph(commit.graph))
    };
    let base_g = graph_of(base)?;
    let ours_g = graph_of(ours)?;
    let theirs_g = graph_of(theirs)?;
    // Endpoints are verified above, so `matching` cannot fail; degrade to
    // direct matching rather than panicking should that ever change.
    let mo = diff::matching(reg, base, ours).unwrap_or_else(|| diff::match_nodes(base_g, ours_g));
    let mt =
        diff::matching(reg, base, theirs).unwrap_or_else(|| diff::match_nodes(base_g, theirs_g));
    let ours_diff = diff::diff(base_g, ours_g, &mo);
    let theirs_diff = diff::diff(base_g, theirs_g, &mt);
    let outcome = merge_graphs(base_g, ours_g, theirs_g, &ours_diff, &theirs_diff);
    Ok(MergeResolution::Diverged {
        base,
        ours_diff,
        theirs_diff,
        outcome,
    })
}

/// Three-way merge of `ours` and `theirs` against their common `base`, under
/// the diffs produced by [`diff::diff`] (which carry the node matchings).
///
/// Node rules, per base node:
///
/// - present on both sides, modified by at most one: the modified side's
///   content is kept (change beats no-change).
/// - modified by both to the same content: kept, no conflict.
/// - modified by both to different content: ours' content is kept and
///   [`Conflict::BothModified`] is flagged.
/// - deleted by one side, untouched by the other: deleted.
/// - deleted by one side, modified by the other: the modified node is kept
///   and [`Conflict::DeleteModify`] is flagged.
///
/// Nodes added by a side are always included. Ours' surviving nodes come
/// first in ours' order, then theirs-only nodes in ascending theirs order, so
/// ours' indices are preserved exactly whenever theirs removed nothing.
///
/// Edge rules, on `(source, target, weight)` sets:
///
/// - a base edge survives unless a side removed it or an endpoint is absent
///   from the merged graph.
/// - added edges from both sides are unioned; identical additions collapse.
/// - an edge added to a node that is absent from the merged graph is dropped
///   and [`Conflict::EdgeToDeleted`] is flagged.
///
/// Construction order is deterministic, so merging the same inputs always
/// yields the same graph address.
pub fn merge_graphs<N, E, Ix>(
    base: &Graph<N, E, Ix>,
    ours: &Graph<N, E, Ix>,
    theirs: &Graph<N, E, Ix>,
    ours_diff: &Diff<E>,
    theirs_diff: &Diff<E>,
) -> MergeOutcome<N, E, Ix>
where
    N: Clone + CaHash,
    E: Clone + Ord,
    Ix: IndexType,
{
    let node_ix = |i: usize| NodeIndex::<Ix>::new(i);
    let mut conflicts = Vec::new();

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
                    if content_addr(&ours[node_ix(o)]) != content_addr(&theirs[node_ix(t)]) {
                        conflicts.push(Conflict::BothModified {
                            base: b,
                            ours: o,
                            theirs: t,
                        });
                    }
                    Fate::KeepOurs
                }
            },
            (Some(_), None) if mod_o => {
                conflicts.push(Conflict::DeleteModify {
                    base: b,
                    modified: Side::Ours,
                });
                Fate::KeepOurs
            }
            (None, Some(_)) if mod_t => {
                conflicts.push(Conflict::DeleteModify {
                    base: b,
                    modified: Side::Theirs,
                });
                Fate::KeepTheirs
            }
            _ => Fate::Delete,
        };
        fates.insert(b, fate);
    }

    // Ours' surviving nodes, in ours' order.
    let inv_ours: Matching = ours_diff.matched.iter().map(|(&b, &o)| (o, b)).collect();
    let inv_theirs: Matching = theirs_diff.matched.iter().map(|(&b, &t)| (t, b)).collect();
    let mut graph = Graph::default();
    let mut node_srcs: Vec<NodeSrc> = Vec::new();
    let mut merged_of_ours: BTreeMap<usize, usize> = BTreeMap::new();
    let mut merged_of_theirs: BTreeMap<usize, usize> = BTreeMap::new();
    for o in 0..ours.node_count() {
        let (weight, src) = match inv_ours.get(&o) {
            // A node matched from base: its fate decides.
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
    // Theirs-only survivors, in theirs' order: nodes added by theirs, and
    // nodes theirs modified but ours deleted (kept by `DeleteModify`).
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

    // The merged index of a base node, if it survived (via either side).
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
    let edge_triples = |g: &Graph<N, E, Ix>| -> BTreeSet<(usize, usize, E)> {
        g.edge_indices()
            .map(|e| {
                let (s, d) = g.edge_endpoints(e).expect("edge must have endpoints");
                (s.index(), d.index(), g[e].clone())
            })
            .collect()
    };
    let mut merged_edges: BTreeSet<(usize, usize, E)> = BTreeSet::new();
    for (s, d, w) in edge_triples(base) {
        let removed = ours_diff.removed_edges.contains(&(s, d, w.clone()))
            || theirs_diff.removed_edges.contains(&(s, d, w.clone()));
        if removed {
            continue;
        }
        // An absent endpoint means the edge is implied-removed with its node.
        let (Some(ms), Some(md)) = (base_merged(s), base_merged(d)) else {
            continue;
        };
        merged_edges.insert((ms, md, w));
    }
    // Union in each side's added edges; identical additions collapse.
    let mut add_edges =
        |added: &BTreeSet<(usize, usize, E)>, merged_of: &BTreeMap<usize, usize>, side: Side| {
            for (s, d, w) in added {
                match (merged_of.get(s), merged_of.get(d)) {
                    (Some(&ms), Some(&md)) => {
                        merged_edges.insert((ms, md, w.clone()));
                    }
                    _ => conflicts.push(Conflict::EdgeToDeleted {
                        side,
                        src: *s,
                        dst: *d,
                        edge: w.clone(),
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
    use crate::{Head, graph_addr};
    use std::time::Duration;

    type G = petgraph::graph::Graph<String, u32, Directed, usize>;

    fn graph(nodes: &[&str], edges: &[(usize, usize, u32)]) -> G {
        let mut g = G::default();
        for n in nodes {
            g.add_node(n.to_string());
        }
        for &(s, d, w) in edges {
            g.add_edge(s.into(), d.into(), w);
        }
        g
    }

    fn commit(reg: &mut Registry<G>, secs: u64, parent: Option<CommitAddr>, g: &G) -> CommitAddr {
        reg.commit_graph(Duration::from_secs(secs), parent, graph_addr(g), || {
            g.clone()
        })
    }

    fn nodes(g: &G) -> Vec<&str> {
        g.node_weights().map(|s| s.as_str()).collect()
    }

    fn edges(g: &G) -> BTreeSet<(usize, usize, u32)> {
        g.edge_indices()
            .map(|e| {
                let (s, d) = g.edge_endpoints(e).unwrap();
                (s.index(), d.index(), g[e])
            })
            .collect()
    }

    /// Merge two graphs that diverged from `base` by one commit each.
    fn merge_two(
        base: &G,
        ours: &G,
        theirs: &G,
    ) -> (
        Registry<G>,
        CommitAddr,
        CommitAddr,
        MergeResolution<String, u32, usize>,
    ) {
        let mut reg = Registry::<G>::default();
        let b = commit(&mut reg, 1, None, base);
        let o = commit(&mut reg, 2, Some(b), ours);
        let t = commit(&mut reg, 3, Some(b), theirs);
        let res = merge_commits(&reg, o, t).unwrap();
        (reg, o, t, res)
    }

    fn diverged(res: MergeResolution<String, u32, usize>) -> MergeOutcome<String, u32, usize> {
        match res {
            MergeResolution::Diverged { outcome, .. } => outcome,
            other => panic!("expected Diverged, got {other:?}"),
        }
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

    /// The collaborative-editing driver scenario: one side edits a node's
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
                theirs: 1
            }],
        );
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
        // Ours deletes ix 1; theirs modifies it.
        let ours = graph(&["a"], &[]);
        let theirs = graph(&["a", "b2"], &[]);
        let (_, _, _, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert_eq!(nodes(&out.graph), vec!["a", "b2"]);
        assert_eq!(
            out.conflicts,
            vec![Conflict::DeleteModify {
                base: 1,
                modified: Side::Theirs
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
        // Ours deletes ix 1 (untouched by theirs); theirs wires into it.
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
                edge: 0
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
        let res2 = merge_commits(&reg2, o, t).unwrap();
        let out2 = diverged(res2);
        assert_eq!(graph_addr(&out.graph), graph_addr(&out2.graph));
    }

    #[test]
    fn fast_forward_and_up_to_date_and_unrelated() {
        let mut reg = Registry::<G>::default();
        let g0 = graph(&["a"], &[]);
        let g1 = graph(&["a", "b"], &[]);
        let root = commit(&mut reg, 1, None, &g0);
        let tip = commit(&mut reg, 2, Some(root), &g1);
        assert!(matches!(
            merge_commits(&reg, root, tip),
            Ok(MergeResolution::FastForward)
        ));
        assert!(matches!(
            merge_commits(&reg, tip, root),
            Ok(MergeResolution::AlreadyUpToDate)
        ));
        let stray = commit(&mut reg, 3, None, &g1);
        assert!(matches!(
            merge_commits(&reg, tip, stray),
            Err(MergeError::Unrelated)
        ));
    }

    /// End-to-end: merge two diverged branches and commit the result; the
    /// merge commit's ancestry spans both sides while undo's first-parent
    /// walk lands on ours' pre-merge tip.
    #[test]
    fn merge_commit_end_to_end() {
        let base = graph(&["a"], &[]);
        let ours = graph(&["a", "x"], &[]);
        let theirs = graph(&["a", "y"], &[]);
        let (mut reg, o, t, res) = merge_two(&base, &ours, &theirs);
        let out = diverged(res);
        assert!(out.conflicts.is_empty());
        reg.insert_name("alpha".to_string(), o);
        let mut head = Head::Branch("alpha".to_string());
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
