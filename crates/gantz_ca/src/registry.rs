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
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
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

/// The result of merging an incoming registry into an existing one.
#[derive(Clone, Debug, Default)]
pub struct MergeResult {
    /// Names that were newly added.
    pub names_added: Vec<String>,
    /// Names that were replaced (pointed to a different commit): (name, old, new).
    pub names_replaced: Vec<(String, CommitAddr, CommitAddr)>,
}

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
    ) -> CommitAddr {
        commit_graph_to_head(self, timestamp, graph_ca, graph, head)
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

    /// Merge an incoming registry into this one.
    ///
    /// Graphs and commits are inserted idempotently (content-addressing means
    /// duplicates are identical). Names are merged: absent names are added,
    /// same-commit names are kept, different-commit names are replaced.
    pub fn merge(&mut self, incoming: Registry<G>) -> MergeResult {
        self.graphs.extend(incoming.graphs);
        self.commits.extend(incoming.commits);
        let mut result = MergeResult::default();
        for (name, new_ca) in incoming.names {
            match self.names.get(&name) {
                Some(&existing_ca) if existing_ca == new_ca => {}
                Some(&existing_ca) => {
                    result
                        .names_replaced
                        .push((name.clone(), existing_ca, new_ca));
                    self.names.insert(name, new_ca);
                }
                None => {
                    result.names_added.push(name.clone());
                    self.names.insert(name, new_ca);
                }
            }
        }
        result
    }
}

impl<G> Registry<G>
where
    G: Clone,
{
    /// Export a subset of the registry containing only the given commits and
    /// their referenced graphs and names.
    pub fn export(&self, required_commits: &HashSet<CommitAddr>) -> Registry<G> {
        let commits: HashMap<CommitAddr, Commit> = self
            .commits
            .iter()
            .filter(|(ca, _)| required_commits.contains(ca))
            .map(|(&ca, commit)| (ca, commit.clone()))
            .collect();
        let used_graphs: HashSet<GraphAddr> = commits.values().map(|c| c.graph).collect();
        let graphs: HashMap<GraphAddr, G> = self
            .graphs
            .iter()
            .filter(|(ca, _)| used_graphs.contains(ca))
            .map(|(&ca, g)| (ca, g.clone()))
            .collect();
        let names: BTreeMap<String, CommitAddr> = self
            .names
            .iter()
            .filter(|(_, ca)| required_commits.contains(ca))
            .map(|(name, &ca)| (name.clone(), ca))
            .collect();
        Registry::new(graphs, commits, names)
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
) -> CommitAddr {
    let parent_ca = *head_commit_ca(&reg.names, head).unwrap();
    let commit_ca = commit_graph(reg, timestamp, Some(parent_ca), graph_ca, graph);
    match *head {
        Head::Commit(ref mut ca) => *ca = commit_ca,
        Head::Branch(ref name) => {
            reg.names.insert(name.to_string(), commit_ca);
        }
    }
    commit_ca
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContentAddr;
    use std::time::Duration;

    fn graph_addr(n: u8) -> GraphAddr {
        GraphAddr::from(ContentAddr::from([n; 32]))
    }

    fn commit_addr_raw(n: u8) -> CommitAddr {
        CommitAddr::from(ContentAddr::from([n; 32]))
    }

    /// Build a simple registry with two independent commits (each with its own
    /// graph) and a name pointing to one of them.
    fn test_registry() -> (Registry<String>, CommitAddr, CommitAddr) {
        let ga = graph_addr(1);
        let gb = graph_addr(2);
        let ca = commit_addr_raw(10);
        let cb = commit_addr_raw(20);
        let commit_a = Commit::new(Duration::from_secs(1), None, ga);
        let commit_b = Commit::new(Duration::from_secs(2), None, gb);
        let graphs = HashMap::from([(ga, "graph_a".to_string()), (gb, "graph_b".to_string())]);
        let commits = HashMap::from([(ca, commit_a), (cb, commit_b)]);
        let names = BTreeMap::from([("alpha".to_string(), ca)]);
        (Registry::new(graphs, commits, names), ca, cb)
    }

    #[test]
    fn export_includes_required_commits_only() {
        let (reg, ca, cb) = test_registry();
        let required: HashSet<_> = [ca].into_iter().collect();
        let exported = reg.export(&required);
        assert!(exported.commits().contains_key(&ca));
        assert!(!exported.commits().contains_key(&cb));
        // Graph referenced by commit_a should be present.
        let ga = reg.commits()[&ca].graph;
        assert!(exported.graphs().contains_key(&ga));
        // Graph referenced by commit_b should not.
        let gb = reg.commits()[&cb].graph;
        assert!(!exported.graphs().contains_key(&gb));
    }

    #[test]
    fn export_filters_names() {
        let (reg, ca, _cb) = test_registry();
        let required: HashSet<_> = [ca].into_iter().collect();
        let exported = reg.export(&required);
        assert_eq!(exported.names().get("alpha"), Some(&ca));
        assert_eq!(exported.names().len(), 1);
    }

    #[test]
    fn export_excludes_names_for_unrequired_commits() {
        let (reg, _ca, cb) = test_registry();
        // Only require commit_b which has no name.
        let required: HashSet<_> = [cb].into_iter().collect();
        let exported = reg.export(&required);
        assert!(exported.names().is_empty());
    }

    #[test]
    fn merge_adds_new_graphs_commits_names() {
        let (mut base, _ca, _cb) = test_registry();
        let gc = graph_addr(3);
        let cc = commit_addr_raw(30);
        let commit_c = Commit::new(Duration::from_secs(3), None, gc);
        let incoming = Registry::new(
            HashMap::from([(gc, "graph_c".to_string())]),
            HashMap::from([(cc, commit_c)]),
            BTreeMap::from([("beta".to_string(), cc)]),
        );
        let result = base.merge(incoming);
        assert!(base.commits().contains_key(&cc));
        assert!(base.graphs().contains_key(&gc));
        assert_eq!(base.names().get("beta"), Some(&cc));
        assert_eq!(result.names_added, vec!["beta".to_string()]);
        assert!(result.names_replaced.is_empty());
    }

    #[test]
    fn merge_same_name_same_commit_is_noop() {
        let (mut base, ca, _cb) = test_registry();
        let ga = base.commits()[&ca].graph;
        let commit_a = base.commits()[&ca].clone();
        let incoming = Registry::new(
            HashMap::from([(ga, "graph_a".to_string())]),
            HashMap::from([(ca, commit_a)]),
            BTreeMap::from([("alpha".to_string(), ca)]),
        );
        let result = base.merge(incoming);
        assert!(result.names_added.is_empty());
        assert!(result.names_replaced.is_empty());
    }

    #[test]
    fn merge_name_conflict_replaces() {
        let (mut base, ca, cb) = test_registry();
        // Incoming maps "alpha" to a different commit.
        let gb = base.commits()[&cb].graph;
        let commit_b = base.commits()[&cb].clone();
        let incoming = Registry::new(
            HashMap::from([(gb, "graph_b".to_string())]),
            HashMap::from([(cb, commit_b)]),
            BTreeMap::from([("alpha".to_string(), cb)]),
        );
        let result = base.merge(incoming);
        assert!(result.names_added.is_empty());
        assert_eq!(result.names_replaced.len(), 1);
        let (name, old, new) = &result.names_replaced[0];
        assert_eq!(name, "alpha");
        assert_eq!(*old, ca);
        assert_eq!(*new, cb);
        assert_eq!(base.names().get("alpha"), Some(&cb));
    }

    #[test]
    fn export_empty_required_set_produces_empty_registry() {
        let (reg, _ca, _cb) = test_registry();
        let exported = reg.export(&HashSet::new());
        assert!(exported.commits().is_empty());
        assert!(exported.graphs().is_empty());
        assert!(exported.names().is_empty());
    }
}
