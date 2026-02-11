//! Utilities for working with [`gantz_ca::Registry`].

use crate::{Edge, Node, graph, node};
use petgraph::visit::{Data, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, Visitable};
use std::collections::HashSet;

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
    let mut required = HashSet::new();
    let mut to_visit: Vec<gantz_ca::ContentAddr> = reg
        .names()
        .values()
        .map(|&ca| gantz_ca::ContentAddr::from(ca))
        .collect();
    for head in heads {
        if let Some(&ca) = reg.head_commit_ca(head.borrow()) {
            to_visit.push(gantz_ca::ContentAddr::from(ca));
        }
    }
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
