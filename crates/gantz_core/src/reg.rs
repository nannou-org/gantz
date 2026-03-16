//! Utilities for working with [`gantz_ca::Registry`].

use crate::{Edge, Node, graph, node};
use petgraph::visit::{Data, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, Visitable};
use std::collections::{BTreeSet, HashSet};

/// Export a registry subset containing the transitive dependencies of the given heads.
///
/// Collects all required commits via [`required_commits`], then calls
/// [`gantz_ca::Registry::export`] to produce the subset.
pub fn export<'a, G>(
    get_node: node::GetNode<'a>,
    reg: &gantz_ca::Registry<G>,
    heads: impl IntoIterator<Item = impl std::borrow::Borrow<gantz_ca::Head>>,
) -> gantz_ca::Registry<G>
where
    G: Clone,
    for<'g> &'g G: Data<EdgeWeight = Edge>
        + IntoEdgesDirected
        + IntoNodeReferences
        + NodeIndexable
        + Visitable,
    for<'g> <&'g G as Data>::NodeWeight: Node,
{
    let required = required_commits(get_node, reg, heads);
    reg.export(&required)
}

/// Export a registry subset containing only the transitive dependencies of the
/// given heads, without including all named commits.
///
/// Unlike [`export`], which seeds from all names (suitable for pruning), this
/// produces the minimal registry for a specific set of heads — only the names
/// whose commits are transitively reachable are included.
pub fn export_heads<'a, G>(
    get_node: node::GetNode<'a>,
    reg: &gantz_ca::Registry<G>,
    heads: impl IntoIterator<Item = impl std::borrow::Borrow<gantz_ca::Head>>,
) -> gantz_ca::Registry<G>
where
    G: Clone,
    for<'g> &'g G: Data<EdgeWeight = Edge>
        + IntoEdgesDirected
        + IntoNodeReferences
        + NodeIndexable
        + Visitable,
    for<'g> <&'g G as Data>::NodeWeight: Node,
{
    let required = required_commits_for_heads(get_node, reg, heads);
    reg.export(&required)
}

/// Collect all commit addresses transitively required by named graphs and heads.
///
/// Starts from all named commits and head commits, then transitively follows
/// references within graphs to build the complete set of required commits.
pub fn required_commits<'a, G>(
    get_node: node::GetNode<'a>,
    reg: &gantz_ca::Registry<G>,
    heads: impl IntoIterator<Item = impl std::borrow::Borrow<gantz_ca::Head>>,
) -> HashSet<gantz_ca::CommitAddr>
where
    for<'g> &'g G: Data<EdgeWeight = Edge>
        + IntoEdgesDirected
        + IntoNodeReferences
        + NodeIndexable
        + Visitable,
    for<'g> <&'g G as Data>::NodeWeight: Node,
{
    let mut seeds: Vec<_> = reg
        .names()
        .values()
        .map(|&ca| gantz_ca::ContentAddr::from(ca))
        .collect();
    for head in heads {
        if let Some(&ca) = reg.head_commit_ca(head.borrow()) {
            seeds.push(gantz_ca::ContentAddr::from(ca));
        }
    }
    transitive_commits(get_node, reg, seeds)
}

/// Collect commit addresses transitively required by the given heads only.
///
/// Unlike [`required_commits`], this does *not* seed the traversal from all
/// named commits — only from the provided heads. Use this when you need the
/// minimal set of commits reachable from a specific set of heads (e.g. for
/// single-head export).
pub fn required_commits_for_heads<'a, G>(
    get_node: node::GetNode<'a>,
    reg: &gantz_ca::Registry<G>,
    heads: impl IntoIterator<Item = impl std::borrow::Borrow<gantz_ca::Head>>,
) -> HashSet<gantz_ca::CommitAddr>
where
    for<'g> &'g G: Data<EdgeWeight = Edge>
        + IntoEdgesDirected
        + IntoNodeReferences
        + NodeIndexable
        + Visitable,
    for<'g> <&'g G as Data>::NodeWeight: Node,
{
    let mut seeds = Vec::new();
    for head in heads {
        if let Some(&ca) = reg.head_commit_ca(head.borrow()) {
            seeds.push(gantz_ca::ContentAddr::from(ca));
        }
    }
    transitive_commits(get_node, reg, seeds)
}

/// Find named graphs not referenced by any other graph in the registry.
///
/// A name is "root" if no graph in the registry contains a node whose
/// `required_addrs` points to that name's commit. Returns names in
/// alphabetical order.
pub fn root_names<'a, G>(
    get_node: node::GetNode<'a>,
    reg: &gantz_ca::Registry<G>,
) -> Vec<String>
where
    for<'g> &'g G: Data<EdgeWeight = Edge>
        + IntoEdgesDirected
        + IntoNodeReferences
        + NodeIndexable
        + Visitable,
    for<'g> <&'g G as Data>::NodeWeight: Node,
{
    // Collect all CommitAddrs referenced by any graph in the registry.
    let mut referenced = HashSet::new();
    for (commit_ca, _) in reg.commits() {
        if let Some(graph) = reg.commit_graph_ref(commit_ca) {
            for ca in graph::required_addrs(get_node, graph) {
                referenced.insert(gantz_ca::CommitAddr::from(ca));
            }
        }
    }

    // Filter names to those whose commit is NOT referenced.
    reg.names()
        .iter()
        .filter(|&(_, &commit_ca)| !referenced.contains(&commit_ca))
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Traverse commit references starting from the given seed addresses.
///
/// Follows graph node references transitively to collect the complete set of
/// reachable commits.
fn transitive_commits<'a, G>(
    get_node: node::GetNode<'a>,
    reg: &gantz_ca::Registry<G>,
    mut to_visit: Vec<gantz_ca::ContentAddr>,
) -> HashSet<gantz_ca::CommitAddr>
where
    for<'g> &'g G: Data<EdgeWeight = Edge>
        + IntoEdgesDirected
        + IntoNodeReferences
        + NodeIndexable
        + Visitable,
    for<'g> <&'g G as Data>::NodeWeight: Node,
{
    let mut required = HashSet::new();
    while let Some(addr) = to_visit.pop() {
        let commit_ca = gantz_ca::CommitAddr::from(addr);
        if reg.commits().contains_key(&commit_ca) && required.insert(commit_ca) {
            if let Some(graph) = reg.commit_graph_ref(&commit_ca) {
                to_visit.extend(graph::required_addrs(get_node, graph));
            }
        }
    }
    required
}
