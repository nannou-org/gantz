//! Commit operations over a [`Registry`]. Minting commits and advancing
//! heads.
//!
//! See the [`registry`](crate::registry) module docs for the `graph_ca` and
//! `graph` closure contract shared by the commit fns.

use crate::{
    Commit, CommitAddr, DataGraph, GraphAddr, Head, Name, Registry, Timestamp, commit_addr,
};

/// Commit the graph at the given address.
pub fn commit_graph(
    reg: &mut Registry,
    timestamp: Timestamp,
    parent_ca: Option<CommitAddr>,
    graph_ca: GraphAddr,
    graph: impl FnOnce() -> DataGraph,
) -> CommitAddr {
    insert_graph_lazily(reg, graph_ca, graph);
    let commit = Commit::new(timestamp, parent_ca, graph_ca);
    let commit_ca = commit_addr(&commit);
    reg.insert_commit_at(commit_ca, commit);
    commit_ca
}

/// Commit the given graph to the given name.
pub fn commit_graph_to_name(
    reg: &mut Registry,
    timestamp: Timestamp,
    graph_ca: GraphAddr,
    graph: impl FnOnce() -> DataGraph,
    name: &Name,
) -> CommitAddr {
    let parent_ca = reg.head(name);
    let commit_ca = commit_graph(reg, timestamp, parent_ca, graph_ca, graph);
    reg.set_head(name.clone(), commit_ca);
    commit_ca
}

/// Commit the given graph to the given head.
pub fn commit_graph_to_head(
    reg: &mut Registry,
    timestamp: Timestamp,
    graph_ca: GraphAddr,
    graph: impl FnOnce() -> DataGraph,
    head: &mut Head,
) -> CommitAddr {
    let parent_ca = reg.head_commit_ca(head).unwrap();
    let commit_ca = commit_graph(reg, timestamp, Some(parent_ca), graph_ca, graph);
    point_head_at(reg, head, commit_ca);
    commit_ca
}

/// Commit the given graph to the given head as a merge of `theirs` into the
/// head's current commit.
pub fn commit_merge_to_head(
    reg: &mut Registry,
    timestamp: Timestamp,
    graph_ca: GraphAddr,
    graph: impl FnOnce() -> DataGraph,
    theirs: CommitAddr,
    head: &mut Head,
) -> CommitAddr {
    let ours = reg.head_commit_ca(head).unwrap();
    insert_graph_lazily(reg, graph_ca, graph);
    let commit = Commit::new_merge(timestamp, ours, theirs, graph_ca);
    let commit_ca = commit_addr(&commit);
    reg.insert_commit_at(commit_ca, commit);
    point_head_at(reg, head, commit_ca);
    commit_ca
}

/// Commit the given graph to the given head as a canonical merge of the
/// diverged tips `a` and `b`. See [`Registry::commit_merge_canonical`].
pub fn commit_merge_canonical(
    reg: &mut Registry,
    a: CommitAddr,
    b: CommitAddr,
    graph_ca: GraphAddr,
    graph: impl FnOnce() -> DataGraph,
    head: &mut Head,
) -> CommitAddr {
    let (first, second) = crate::sync::canonical_tips(reg.commits(), a, b);
    let timestamp = crate::sync::merge_timestamp(reg.commits(), first, second);
    insert_graph_lazily(reg, graph_ca, graph);
    let commit = Commit::new_merge(timestamp, first, second, graph_ca);
    let commit_ca = commit_addr(&commit);
    reg.insert_commit_at(commit_ca, commit);
    point_head_at(reg, head, commit_ca);
    commit_ca
}

/// Point the head at the given commit. A branch head updates the heads
/// section. A detached head is reassigned directly.
pub fn point_head_at(reg: &mut Registry, head: &mut Head, commit_ca: CommitAddr) {
    match *head {
        Head::Commit(ref mut ca) => *ca = commit_ca,
        Head::Branch(ref name) => {
            reg.set_head(name.clone(), commit_ca);
        }
    }
}

/// Insert the graph at the given address, calling `graph()` only when the
/// address is absent.
fn insert_graph_lazily(reg: &mut Registry, graph_ca: GraphAddr, graph: impl FnOnce() -> DataGraph) {
    if reg.graph(&graph_ca).is_none() {
        reg.insert_graph_at(graph_ca, graph());
    }
}
