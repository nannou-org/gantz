//! A registry tracking graphs, commits and names (branches).

use crate::{CaHash, Commit, CommitAddr, GraphAddr, Head, Timestamp, commit_addr, graph_addr};
use petgraph::visit::{Data, GraphBase, IntoEdgeReferences, IntoNodeReferences, NodeIndexable};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::Hash,
};

/// A registry of content-addressed graphs, commits of those graphs, and
/// optional names for those commits.
#[derive(Default, Deserialize, Serialize)]
pub struct Registry<G> {
    /// A mapping from graph addresses to graphs.
    graphs: HashMap<GraphAddr, G>,
    /// A mapping from commit addresses to commits.
    commits: HashMap<CommitAddr, Commit>,
    /// A mapping from names to graph content addresses.
    names: BTreeMap<String, CommitAddr>,
}

pub type Graphs<G> = HashMap<GraphAddr, G>;
pub type Commits = HashMap<CommitAddr, Commit>;
pub type Names = BTreeMap<String, CommitAddr>;

impl<G> Registry<G> {
    /// Construct the registry from its parts.
    pub fn new(
        graphs: HashMap<GraphAddr, G>,
        commits: HashMap<CommitAddr, Commit>,
        names: BTreeMap<String, CommitAddr>,
    ) -> Self {
        Self {
            graphs,
            commits,
            names,
        }
    }

    /// A mapping from graph addresses to graphs.
    pub fn graphs(&self) -> &Graphs<G> {
        &self.graphs
    }

    /// A mapping from commit addresses to commits.
    pub fn commits(&self) -> &Commits {
        &self.commits
    }

    /// A mapping from names to graph content addresses.
    pub fn names(&self) -> &Names {
        &self.names
    }

    /// Lookup the commit for the given name.
    pub fn named_commit(&self, name: &str) -> Option<&Commit> {
        self.names.get(name).and_then(|ca| self.commits.get(ca))
    }

    /// Look-up the commit address pointed to by the given head.
    pub fn head_commit_ca<'a>(&'a self, head: &'a Head) -> Option<&'a CommitAddr> {
        head_commit_ca(&self.names, head)
    }

    /// Look-up the commit pointed to by the given head.
    pub fn head_commit<'a>(&'a self, head: &'a Head) -> Option<&'a Commit> {
        self.head_commit_ca(head)
            .and_then(|ca| self.commits.get(&ca))
    }

    /// Look-up the graph pointed to by the head.
    pub fn head_graph<'a>(&'a self, head: &'a Head) -> Option<&'a G> {
        self.head_commit(head)
            .and_then(|commit| self.graphs.get(&commit.graph))
    }

    /// Look-up the graph pointed to by the given commit address.
    pub fn commit_graph_ref(&self, ca: &CommitAddr) -> Option<&G> {
        self.commits
            .get(ca)
            .and_then(|commit| self.graphs.get(&commit.graph))
    }

    /// Commit the graph at the given address.
    ///
    /// NOTE: Assumes `graph_ca` is a correct address for the graph resulting
    /// from `graph()`.
    pub fn commit_graph(
        &mut self,
        timestamp: Timestamp,
        parent_ca: Option<CommitAddr>,
        graph_ca: GraphAddr,
        graph: impl FnOnce() -> G,
    ) -> CommitAddr {
        commit_graph(self, timestamp, parent_ca, graph_ca, graph)
    }

    /// Commit the graph to the given name.
    ///
    /// NOTE: Assumes `graph_ca` is a correct address for the graph resulting
    /// from `graph()`.
    pub fn commit_graph_to_name(
        &mut self,
        timestamp: Timestamp,
        graph_ca: GraphAddr,
        graph: impl FnOnce() -> G,
        name: &str,
    ) -> CommitAddr {
        commit_graph_to_name(self, timestamp, graph_ca, graph, name)
    }

    /// Commit the graph at the given address and update `head` to a new commit
    /// pointing to the graph.
    ///
    /// Only calls `graph` if no graph exists within the registry for the given
    /// address.
    ///
    /// NOTE: Assumes `graph_ca` is a correct address for the graph resulting
    /// from `graph()`.
    pub fn commit_graph_to_head(
        &mut self,
        timestamp: Timestamp,
        graph_ca: GraphAddr,
        graph: impl FnOnce() -> G,
        head: &mut Head,
    ) {
        commit_graph_to_head(self, timestamp, graph_ca, graph, head);
    }

    /// Insert the given name mapping into the registry.
    ///
    /// Returns the previous mapping if one exists.
    pub fn insert_name(&mut self, name: String, ca: CommitAddr) -> Option<CommitAddr> {
        self.names.insert(name, ca)
    }

    /// Remove the given name from the registry.
    ///
    /// This does not remove the underlying commit, just the name mapping.
    pub fn remove_name(&mut self, name: &str) -> Option<CommitAddr> {
        self.names.remove(name)
    }

    /// Prune commits and graphs not in the required set.
    ///
    /// 1. Removes commits not in `required_commits`
    /// 2. Removes graphs not referenced by any remaining commit
    /// 3. Detaches invalid parent references
    pub fn prune_unreachable(&mut self, required_commits: &HashSet<CommitAddr>) {
        self.commits.retain(|ca, _| required_commits.contains(ca));
        let used_graphs: HashSet<_> = self.commits.values().map(|c| c.graph).collect();
        self.graphs.retain(|ca, _| used_graphs.contains(ca));
        detach_invalid_parents(&mut self.commits);
    }
}

impl<G> Registry<G>
where
    G: Default,
{
    /// Initialise head to a new initial commit pointing to an empty graph.
    pub fn init_head(&mut self, timestamp: Timestamp) -> Head
    where
        G: Data + NodeIndexable,
        G::EdgeWeight: CaHash + Ord,
        G::NodeWeight: CaHash,
        G::NodeId: Eq + Hash + Ord,
        for<'a> &'a G: Data<EdgeWeight = G::EdgeWeight, NodeWeight = G::NodeWeight>
            + GraphBase<NodeId = G::NodeId, EdgeId = G::EdgeId>
            + IntoNodeReferences
            + IntoEdgeReferences,
    {
        let graph = G::default();
        let graph_ca = graph_addr(&graph);
        let commit_ca = self.commit_graph(timestamp, None, graph_ca, || graph);
        Head::Commit(commit_ca)
    }
}

/// Look-up the commit address pointed to by the given head.
fn head_commit_ca<'a>(names: &'a Names, head: &'a Head) -> Option<&'a CommitAddr> {
    match head {
        Head::Branch(name) => names.get(name),
        Head::Commit(ca) => Some(ca),
    }
}

/// Commit the given graph to the given head.
///
/// If the graph doesn't exist, calls `graph()` to retrieve the graph for the
/// registry.
fn commit_graph<G>(
    reg: &mut Registry<G>,
    timestamp: Timestamp,
    parent_ca: Option<CommitAddr>,
    graph_ca: GraphAddr,
    graph: impl FnOnce() -> G,
) -> CommitAddr {
    reg.graphs.entry(graph_ca).or_insert_with(graph);
    let commit = Commit::new(timestamp, parent_ca, graph_ca);
    let commit_ca = commit_addr(&commit);
    reg.commits.insert(commit_ca, commit);
    commit_ca
}

/// Commit the given graph to the given name (branch).
///
/// If the graph doesn't exist, calls `graph()` to retrieve the graph for the
/// registry.
fn commit_graph_to_name<G>(
    reg: &mut Registry<G>,
    timestamp: Timestamp,
    graph_ca: GraphAddr,
    graph: impl FnOnce() -> G,
    name: &str,
) -> CommitAddr {
    let parent_ca = reg.names.get(name).copied();
    let commit_ca = commit_graph(reg, timestamp, parent_ca, graph_ca, graph);
    reg.names.insert(name.to_string(), commit_ca);
    commit_ca
}

/// Commit the given graph to the given head.
///
/// If the graph doesn't exist, calls `graph()` to retrieve the graph for the
/// registry.
fn commit_graph_to_head<G>(
    reg: &mut Registry<G>,
    timestamp: Timestamp,
    graph_ca: GraphAddr,
    graph: impl FnOnce() -> G,
    head: &mut Head,
) {
    let parent_ca = *head_commit_ca(&reg.names, head).unwrap();
    let commit_ca = commit_graph(reg, timestamp, Some(parent_ca), graph_ca, graph);
    match *head {
        Head::Commit(ref mut ca) => *ca = commit_ca,
        Head::Branch(ref name) => {
            reg.names.insert(name.to_string(), commit_ca);
        }
    }
}

/// For all `parent` commits that are invalid (i.e. don't point to an existing
/// commit), set them to `None`.
fn detach_invalid_parents(commits: &mut Commits) {
    let mut has_invalid_parent = HashSet::new();
    for (&ca, commit) in commits.iter() {
        if let Some(parent_ca) = commit.parent {
            if !commits.contains_key(&parent_ca) {
                has_invalid_parent.insert(ca);
            }
        }
    }
    for ca in has_invalid_parent {
        commits.get_mut(&ca).unwrap().parent = None;
    }
}
