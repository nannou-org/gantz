//! Node identity matching and structural diffing between graph versions.
//!
//! Nodes have no persistent identity of their own - only an index and a
//! content address - so diffing two versions of a graph first requires a
//! [`Matching`]: an injective mapping pairing "the same node" across the two
//! versions.
//!
//! Two strategies are provided:
//!
//! - [`match_nodes`]: direct content matching. Nodes are grouped by content
//!   address and paired within each group in ascending-index order. This is
//!   conservative: a node whose content was edited appears as a removal plus
//!   an addition.
//! - [`matching`]: chain-tracked matching. The registry retains every commit
//!   and graph, and gantz's commit-on-change model means consecutive commits
//!   differ by a single logical edit, during which a node's *index* is its
//!   identity (an in-place edit keeps the node's index; a removal swap-moves
//!   exactly one other node, which content-matching pairs). Matching each
//!   consecutive pair of commits along the first-parent chain and composing
//!   the results tracks a node's identity through content edits, so an edit
//!   diffs as a *modification* rather than a remove + add.

use crate::{CaHash, CommitAddr, Registry, Timestamp, content_addr, history};
use petgraph::{Directed, graph::IndexType};
use std::collections::{BTreeMap, BTreeSet, HashSet};

type Graph<N, E, Ix> = petgraph::graph::Graph<N, E, Directed, Ix>;

/// A node identity mapping between two versions of a graph: left node index
/// to right node index. Injective: no two left nodes map to the same right
/// node.
pub type Matching = BTreeMap<usize, usize>;

/// A structural diff of `other` relative to `base`, under a node [`Matching`].
///
/// Node entries are expressed in the coordinates of the graph they exist in:
/// removals in `base` indices, additions in `other` indices. Edges are
/// treated as sets of `(source, target, weight)` triples; parallel edges with
/// identical weights are not distinguished.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diff<E> {
    /// Base node index to other node index, for nodes present in both.
    pub matched: Matching,
    /// The subset of `matched` (base indices) whose node content changed.
    pub modified: BTreeSet<usize>,
    /// Base indices with no counterpart in `other`.
    pub removed_nodes: BTreeSet<usize>,
    /// Other indices with no counterpart in `base`.
    pub added_nodes: BTreeSet<usize>,
    /// Edges present in `base` but not `other`, in *base* coordinates.
    ///
    /// Only edges whose endpoints both survive into `other`: edges lost as a
    /// consequence of node removal are implied by `removed_nodes`.
    pub removed_edges: BTreeSet<(usize, usize, E)>,
    /// Edges present in `other` but not `base`, in *other* coordinates
    /// (endpoints may be added nodes).
    pub added_edges: BTreeSet<(usize, usize, E)>,
}

/// Change counts for a [`Diff`], e.g. for GUI hover summaries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiffSummary {
    pub nodes_added: usize,
    pub nodes_removed: usize,
    pub nodes_modified: usize,
    pub edges_added: usize,
    pub edges_removed: usize,
}

impl<E> Diff<E> {
    /// Change counts for this diff.
    pub fn summary(&self) -> DiffSummary {
        DiffSummary {
            nodes_added: self.added_nodes.len(),
            nodes_removed: self.removed_nodes.len(),
            nodes_modified: self.modified.len(),
            edges_added: self.added_edges.len(),
            edges_removed: self.removed_edges.len(),
        }
    }

    /// Whether the diff records no changes.
    pub fn is_empty(&self) -> bool {
        self.modified.is_empty()
            && self.removed_nodes.is_empty()
            && self.added_nodes.is_empty()
            && self.removed_edges.is_empty()
            && self.added_edges.is_empty()
    }
}

/// The content address of every node, indexed by node index (contiguous for
/// a plain `petgraph::Graph`).
fn node_cas<N, E, Ix>(g: &Graph<N, E, Ix>) -> Vec<crate::ContentAddr>
where
    N: CaHash,
    Ix: IndexType,
{
    g.node_weights().map(crate::content_addr).collect()
}

/// The [`match_nodes`] grouping over precomputed node content addresses.
fn match_cas(a: &[crate::ContentAddr], b: &[crate::ContentAddr]) -> Matching {
    let mut groups: BTreeMap<crate::ContentAddr, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for (ix, ca) in a.iter().enumerate() {
        groups.entry(*ca).or_default().0.push(ix);
    }
    for (ix, ca) in b.iter().enumerate() {
        groups.entry(*ca).or_default().1.push(ix);
    }
    // Indices are pushed in ascending order, so the per-group zips pair by
    // canonical rank without sorting.
    let mut matching = Matching::new();
    for (a_ixs, b_ixs) in groups.into_values() {
        matching.extend(a_ixs.into_iter().zip(b_ixs));
    }
    matching
}

/// Directly match nodes between two graphs by content.
///
/// Nodes are grouped by content address and paired within each group in
/// ascending-index order (canonical rank). Conservative: a node whose content
/// was edited is left unmatched on both sides.
pub fn match_nodes<N, E, Ix>(a: &Graph<N, E, Ix>, b: &Graph<N, E, Ix>) -> Matching
where
    N: CaHash,
    Ix: IndexType,
{
    match_cas(&node_cas(a), &node_cas(b))
}

/// Match nodes between two *consecutive* commits' graphs (by their node
/// content addresses).
///
/// [`match_cas`] pairs everything whose content is unchanged; the leftover
/// nodes on both sides are then paired when they share an index. A single
/// edit step justifies this: an in-place content edit preserves the node's
/// index, while a swap-removal never leaves an equal-index leftover pair.
fn match_step_cas(prev: &[crate::ContentAddr], next: &[crate::ContentAddr]) -> Matching {
    let mut matching = match_cas(prev, next);
    let matched_next: HashSet<usize> = matching.values().copied().collect();
    for ix in 0..prev.len() {
        if !matching.contains_key(&ix) && ix < next.len() && !matched_next.contains(&ix) {
            matching.insert(ix, ix);
        }
    }
    matching
}

/// Node identity between `base`'s graph and `tip`'s graph, tracked step-wise
/// along `tip`'s first-parent chain (see the module docs).
///
/// Steps whose graph address is unchanged (e.g. layout-only commits) are
/// identity. Falls back to direct [`match_nodes`] between the endpoint graphs
/// when `base` is not on the chain or an intermediate graph is unavailable.
/// Returns `None` only when an endpoint commit or graph is missing from the
/// registry.
pub fn matching<N, E, Ix>(
    reg: &Registry<Graph<N, E, Ix>>,
    base: CommitAddr,
    tip: CommitAddr,
) -> Option<Matching>
where
    N: CaHash,
    Ix: IndexType,
{
    matching_with_times(reg, base, tip).map(|(matching, _)| matching)
}

/// [`matching`], also returning each tracked node's *last-edit time*: for
/// every base node still present at the tip, the timestamp of the last commit
/// along the chain that changed its content (no entry = content untouched).
///
/// Feeds per-node "last edit wins" conflict resolution (see
/// [`crate::merge::BothModified::KeepNewest`]). The direct-matching fallback
/// has no chain to read times from, so it returns them empty; callers fall
/// back to the tips' own timestamps.
pub fn matching_with_times<N, E, Ix>(
    reg: &Registry<Graph<N, E, Ix>>,
    base: CommitAddr,
    tip: CommitAddr,
) -> Option<(Matching, BTreeMap<usize, Timestamp>)>
where
    N: CaHash,
    Ix: IndexType,
{
    let commits = reg.commits();
    let graphs = reg.graphs();
    let base_graph = graphs.get(&commits.get(&base)?.graph)?;
    let tip_graph = graphs.get(&commits.get(&tip)?.graph)?;
    let identity = |g: &Graph<N, E, Ix>| (0..g.node_count()).map(|ix| (ix, ix)).collect();
    let direct = || (match_nodes(base_graph, tip_graph), BTreeMap::new());

    let Some(chain) = history::first_parent_chain_to(commits, tip, base) else {
        return Some(direct());
    };

    // Walk the chain oldest-first, composing the per-step matchings and
    // stamping tracked nodes whose content changed at a step. Each step's
    // node addresses are computed once and carried into the next iteration.
    let mut matching: Matching = identity(base_graph);
    let mut times: BTreeMap<usize, Timestamp> = BTreeMap::new();
    let mut steps = chain.iter().rev();
    let mut prev = steps.next()?;
    let mut prev_cas: Option<Vec<crate::ContentAddr>> = None;
    for next in steps {
        // Same graph (e.g. a layout-only commit): identity step.
        if prev.graph == next.graph {
            prev = next;
            continue;
        }
        let (Some(pg), Some(ng)) = (graphs.get(&prev.graph), graphs.get(&next.graph)) else {
            // An intermediate graph is unavailable: fall back to direct.
            return Some(direct());
        };
        let pc = prev_cas.take().unwrap_or_else(|| node_cas(pg));
        let nc = node_cas(ng);
        let step = match_step_cas(&pc, &nc);
        matching = matching
            .into_iter()
            .filter_map(|(b, cur)| {
                let &n = step.get(&cur)?;
                if pc[cur] != nc[n] {
                    times.insert(b, next.timestamp);
                }
                Some((b, n))
            })
            .collect();
        prev_cas = Some(nc);
        prev = next;
    }
    Some((matching, times))
}

/// The structural diff of `other` relative to `base` under the given node
/// [`Matching`] (see [`Diff`]).
pub fn diff<N, E, Ix>(
    base: &Graph<N, E, Ix>,
    other: &Graph<N, E, Ix>,
    matching: &Matching,
) -> Diff<E>
where
    N: CaHash,
    E: Clone + Ord,
    Ix: IndexType,
{
    let matched_other: HashSet<usize> = matching.values().copied().collect();
    let modified = matching
        .iter()
        .filter(|&(&b, &o)| {
            let b_ca = content_addr(&base[petgraph::graph::NodeIndex::new(b)]);
            let o_ca = content_addr(&other[petgraph::graph::NodeIndex::new(o)]);
            b_ca != o_ca
        })
        .map(|(&b, _)| b)
        .collect();
    let removed_nodes: BTreeSet<usize> = (0..base.node_count())
        .filter(|ix| !matching.contains_key(ix))
        .collect();
    let added_nodes: BTreeSet<usize> = (0..other.node_count())
        .filter(|ix| !matched_other.contains(ix))
        .collect();

    let edge_set = |g: &Graph<N, E, Ix>| -> BTreeSet<(usize, usize, E)> {
        g.edge_indices()
            .map(|e| {
                let (s, d) = g.edge_endpoints(e).expect("edge must have endpoints");
                (s.index(), d.index(), g[e].clone())
            })
            .collect()
    };
    let base_edges = edge_set(base);
    let other_edges = edge_set(other);

    // Map base edges into other coordinates where both endpoints survive.
    let removed_edges = base_edges
        .iter()
        .filter(|(s, d, w)| {
            let (Some(&os), Some(&od)) = (matching.get(s), matching.get(d)) else {
                // An endpoint was removed: implied by `removed_nodes`.
                return false;
            };
            !other_edges.contains(&(os, od, w.clone()))
        })
        .cloned()
        .collect();
    // Map other edges back into base coordinates to detect additions.
    let inverse: BTreeMap<usize, usize> = matching.iter().map(|(&b, &o)| (o, b)).collect();
    let added_edges = other_edges
        .iter()
        .filter(|(s, d, w)| {
            let (Some(&bs), Some(&bd)) = (inverse.get(s), inverse.get(d)) else {
                // An endpoint is an added node: the edge is necessarily new.
                return true;
            };
            !base_edges.contains(&(bs, bd, w.clone()))
        })
        .cloned()
        .collect();

    Diff {
        matched: matching.clone(),
        modified,
        removed_nodes,
        added_nodes,
        removed_edges,
        added_edges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Commit, Timestamp, commit_addr, graph_addr};
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

    /// Commit `g` on top of `parent` in `reg`, returning the commit addr.
    fn commit(reg: &mut Registry<G>, secs: u64, parent: Option<CommitAddr>, g: &G) -> CommitAddr {
        reg.commit_graph(Duration::from_secs(secs), parent, graph_addr(g), || {
            g.clone()
        })
    }

    /// [`match_step_cas`] over whole graphs, for test brevity.
    fn match_step(prev: &G, next: &G) -> Matching {
        match_step_cas(&node_cas(prev), &node_cas(next))
    }

    #[test]
    fn match_nodes_pairs_duplicate_content_by_rank() {
        // [A, B, A] with B removed via swap-remove -> [A, A].
        let a = graph(&["a", "b", "a"], &[]);
        let b = graph(&["a", "a"], &[]);
        let m = match_nodes(&a, &b);
        assert_eq!(m, Matching::from([(0, 0), (2, 1)]));
    }

    #[test]
    fn match_step_pairs_edited_node_by_index() {
        let prev = graph(&["a", "b", "c"], &[]);
        let next = graph(&["a", "b2", "c"], &[]);
        let m = match_step(&prev, &next);
        assert_eq!(m, Matching::from([(0, 0), (1, 1), (2, 2)]));
    }

    #[test]
    fn match_step_swap_removal_leaves_no_false_pair() {
        // Removing ix 1 swap-moves the last node into its slot.
        let prev = graph(&["a", "b", "c"], &[]);
        let next = graph(&["a", "c"], &[]);
        let m = match_step(&prev, &next);
        assert_eq!(m, Matching::from([(0, 0), (2, 1)]));
    }

    #[test]
    fn match_step_edit_to_duplicate_content() {
        // Editing ix 1 to duplicate ix 0's content must not steal its match.
        let prev = graph(&["a", "b"], &[]);
        let next = graph(&["a", "a"], &[]);
        let m = match_step(&prev, &next);
        assert_eq!(m, Matching::from([(0, 0), (1, 1)]));
    }

    #[test]
    fn matching_composes_along_the_chain() {
        let mut reg = Registry::<G>::default();
        let g0 = graph(&["a", "b"], &[]);
        let g1 = graph(&["a", "b2"], &[]); // edit ix 1
        let g2 = graph(&["a", "b2", "c"], &[]); // add ix 2
        let g3 = graph(&["c", "b2"], &[]); // remove ix 0 (c swaps in)
        let base = commit(&mut reg, 1, None, &g0);
        let c1 = commit(&mut reg, 2, Some(base), &g1);
        // A layout-only commit: same graph, new commit.
        let c1b = commit(&mut reg, 3, Some(c1), &g1);
        let c2 = commit(&mut reg, 4, Some(c1b), &g2);
        let c3 = commit(&mut reg, 5, Some(c2), &g3);

        // Through the edit: identity is preserved.
        assert_eq!(
            matching(&reg, base, c2).unwrap(),
            Matching::from([(0, 0), (1, 1)]),
        );
        // Through the removal: base ix 0 is gone, ix 1 tracked at ix 1.
        assert_eq!(matching(&reg, base, c3).unwrap(), Matching::from([(1, 1)]));
    }

    #[test]
    fn matching_with_times_stamps_the_last_content_change() {
        let mut reg = Registry::<G>::default();
        let g0 = graph(&["a", "b"], &[]);
        let g1 = graph(&["a", "b2"], &[]); // edit ix 1 @ t=2
        let g2 = graph(&["a", "b2", "c"], &[]); // add ix 2 @ t=3
        let g3 = graph(&["a", "b3", "c"], &[]); // edit ix 1 again @ t=4
        let base = commit(&mut reg, 1, None, &g0);
        let c1 = commit(&mut reg, 2, Some(base), &g1);
        let c2 = commit(&mut reg, 3, Some(c1), &g2);
        let c3 = commit(&mut reg, 4, Some(c2), &g3);
        let (m, times) = matching_with_times(&reg, base, c3).unwrap();
        assert_eq!(m, Matching::from([(0, 0), (1, 1)]));
        // Node 1's last content change was the t=4 commit; node 0 untouched.
        assert_eq!(times, BTreeMap::from([(1, Timestamp::from_secs(4))]),);
    }

    #[test]
    fn matching_falls_back_to_direct_when_chain_unavailable() {
        // Hand-build a registry whose intermediate graph is absent.
        let g0 = graph(&["a", "b"], &[]);
        let g1 = graph(&["a", "b2"], &[]);
        let g2 = graph(&["a", "b3"], &[]);
        let c0 = Commit::new(Timestamp::from_secs(1), None, graph_addr(&g0));
        let ca0 = commit_addr(&c0);
        let c1 = Commit::new(Timestamp::from_secs(2), Some(ca0), graph_addr(&g1));
        let ca1 = commit_addr(&c1);
        let c2 = Commit::new(Timestamp::from_secs(3), Some(ca1), graph_addr(&g2));
        let ca2 = commit_addr(&c2);
        let graphs = [(graph_addr(&g0), g0.clone()), (graph_addr(&g2), g2.clone())]
            .into_iter()
            .collect();
        let commits = [(ca0, c0), (ca1, c1), (ca2, c2)].into_iter().collect();
        let reg = Registry::new(graphs, commits, Default::default());
        // Direct matching pairs only the content-identical node.
        assert_eq!(matching(&reg, ca0, ca2).unwrap(), Matching::from([(0, 0)]));
    }

    #[test]
    fn diff_reports_node_and_edge_changes() {
        let base = graph(&["a", "b"], &[(0, 1, 0)]);
        let other = graph(&["a", "b2", "c"], &[(0, 2, 1)]);
        let m = Matching::from([(0, 0), (1, 1)]);
        let d = diff(&base, &other, &m);
        assert_eq!(d.modified, BTreeSet::from([1]));
        assert!(d.removed_nodes.is_empty());
        assert_eq!(d.added_nodes, BTreeSet::from([2]));
        assert_eq!(d.removed_edges, BTreeSet::from([(0, 1, 0)]));
        assert_eq!(d.added_edges, BTreeSet::from([(0, 2, 1)]));
        let s = d.summary();
        assert_eq!(
            (s.nodes_added, s.nodes_removed, s.nodes_modified),
            (1, 0, 1)
        );
        assert_eq!((s.edges_added, s.edges_removed), (1, 1));
    }

    #[test]
    fn diff_excludes_edges_implied_by_node_removal() {
        let base = graph(&["a", "b"], &[(0, 1, 0)]);
        let other = graph(&["a"], &[]);
        let m = Matching::from([(0, 0)]);
        let d = diff(&base, &other, &m);
        assert_eq!(d.removed_nodes, BTreeSet::from([1]));
        assert!(d.removed_edges.is_empty());
        assert!(!d.is_empty());
    }

    #[test]
    fn diff_of_identical_graphs_is_empty() {
        let g = graph(&["a", "b"], &[(0, 1, 0)]);
        let m = match_nodes(&g, &g);
        assert!(diff(&g, &g, &m).is_empty());
    }
}
