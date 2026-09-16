//! The registry. Content-addressed content columns plus open metadata
//! sections.
//!
//! The registry has exactly two parts:
//!
//! - The content columns. These are [`graphs`](Registry::graphs),
//!   [`commits`](Registry::commits) and per-section
//!   [`blobs`](Registry::blobs). They are immutable, content-addressed and
//!   insert-only, except for [`prune`](crate::reach::prune).
//! - The [`sections`](Registry::sections). These hold all mutable state.
//!   Heads are the core-declared [`Heads`] section. Domains attach their own
//!   metadata by declaring a [`SectionDecl`] in their own crate. The registry
//!   core never learns domain types. Sections carry their merge policy and
//!   liveness rules as data, so unknown sections are merged, pruned,
//!   exported and round-tripped correctly.
//!
//! Content forms a pure Merkle DAG. Graphs are [`DataGraph`]s of erased
//! [`NodeData`] nodes. They reference nested graphs by [`GraphAddr`] and
//! blobs by content address, so content identity depends only on content.
//! Commits form the history DAG above it and carry the timestamps. Names
//! point at commits and name lines of history.
//!
//! Registry invariant: commits are parent-closed and graph-complete. Every
//! stored commit's parents are present or detached, never dangling, and its
//! graph is present. [`add_commit`](Registry::add_commit) clears absent
//! parents before hashing. [`sync::Staged::apply`] validates or detaches.
//! [`prune`](crate::reach::prune) detaches after removal.
//!
//! The commit methods take a `graph_ca` and a `graph` closure. The closure
//! is called only when the registry has no graph at `graph_ca`. The caller
//! must ensure `graph_ca` is the address of the graph the closure returns.
//!
//! [`sync::Staged::apply`]: crate::sync::Staged

use crate::section::{TryFromKey, value_to_datum};
use crate::{
    BlobLiveness, BlobStore, Bytes, Commit, CommitAddr, ContentAddr, DataGraph, GraphAddr, Head,
    Key, Liveness, MergePolicy, Name, NodeData, Section, SectionDecl, SectionId, Timestamp, Value,
    commit_addr, datum::DatumError, graph_addr,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A registry of content-addressed graphs, commits of those graphs, blob
/// stores, and metadata sections. See the module docs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Registry {
    /// A mapping from graph addresses to graphs.
    #[serde(serialize_with = "crate::serde_sorted::serialize_map")]
    graphs: HashMap<GraphAddr, DataGraph>,
    /// A mapping from commit addresses to commits.
    #[serde(serialize_with = "crate::serde_sorted::serialize_map")]
    commits: HashMap<CommitAddr, Commit>,
    /// Content-addressed blob stores, one per section.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    blobs: BTreeMap<SectionId, BlobStore>,
    /// Mutable metadata sections, `heads` among them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sections: BTreeMap<SectionId, Section>,
}

/// The core `heads` section. Maps names to the commits at the tips of their
/// lines of history.
///
/// Its merge policy is replace. An incoming head wins and is reported in
/// [`MergeReport`]. Its entries are the roots of reachability.
pub struct Heads;

pub type Graphs = HashMap<GraphAddr, DataGraph>;
pub type Commits = HashMap<CommitAddr, Commit>;

/// The id of the core [`Heads`] section.
pub const HEADS_ID: &str = "heads";

/// The result of merging an incoming registry into an existing one.
#[derive(Clone, Debug, Default)]
pub struct MergeReport {
    /// Head names that were newly added.
    pub heads_added: Vec<Name>,
    /// Head names that were repointed, as (name, old, new).
    pub heads_replaced: Vec<(Name, CommitAddr, CommitAddr)>,
    /// Non-head section entries that were newly added.
    pub sections_added: Vec<(SectionId, Key)>,
    /// Non-head section entries replaced under a `Replace` policy section.
    pub sections_replaced: Vec<(SectionId, Key)>,
}

impl SectionDecl for Heads {
    const ID: &'static str = HEADS_ID;
    const POLICY: MergePolicy = MergePolicy::Replace;
    const LIVENESS: Liveness = Liveness::Root;
    type Key = Name;
    type Value = CommitAddr;

    fn encode(value: &Self::Value) -> Result<Value, DatumError> {
        Ok(Value::Commit(*value))
    }

    fn decode(value: &Value) -> Option<Self::Value> {
        match value {
            Value::Commit(ca) => Some(*ca),
            _ => None,
        }
    }
}

impl Registry {
    /// Construct a registry from content columns and head mappings.
    pub fn from_parts(
        graphs: HashMap<GraphAddr, DataGraph>,
        commits: HashMap<CommitAddr, Commit>,
        heads: BTreeMap<Name, CommitAddr>,
    ) -> Self {
        let mut reg = Self {
            graphs,
            commits,
            blobs: BTreeMap::new(),
            sections: BTreeMap::new(),
        };
        for (name, ca) in heads {
            reg.set_head(name, ca);
        }
        reg
    }

    /// A mapping from graph addresses to graphs.
    pub fn graphs(&self) -> &Graphs {
        &self.graphs
    }

    /// A mapping from commit addresses to commits.
    pub fn commits(&self) -> &Commits {
        &self.commits
    }

    /// The blob stores, keyed by section.
    pub fn blobs(&self) -> &BTreeMap<SectionId, BlobStore> {
        &self.blobs
    }

    /// The metadata sections, keyed by section id.
    pub fn sections(&self) -> &BTreeMap<SectionId, Section> {
        &self.sections
    }

    /// The graph stored at the given address.
    pub fn graph(&self, ga: &GraphAddr) -> Option<&DataGraph> {
        self.graphs.get(ga)
    }

    /// The bytes stored at the given address in the given blob section.
    pub fn blob(&self, section: &str, addr: &ContentAddr) -> Option<&Bytes> {
        self.blobs.get(section).and_then(|store| store.get(addr))
    }

    /// The section with the given id, if any.
    pub fn section(&self, id: &str) -> Option<&Section> {
        self.sections.get(id)
    }

    /// The raw value stored under `key` in the section with the given id.
    pub fn section_entry(&self, id: &str, key: &Key) -> Option<&Value> {
        self.sections.get(id).and_then(|s| s.entries.get(key))
    }

    /// The commit at the tip of the named line of history.
    pub fn head(&self, name: &Name) -> Option<CommitAddr> {
        section_get::<Heads>(self, name)
    }

    /// All heads, ordered by name.
    pub fn heads(&self) -> impl Iterator<Item = (&Name, CommitAddr)> + '_ {
        self.sections
            .get(Heads::ID)
            .into_iter()
            .flat_map(|s| s.entries.iter())
            .filter_map(|(key, value)| match (key, value) {
                (Key::Name(name), Value::Commit(ca)) => Some((name, *ca)),
                _ => None,
            })
    }

    /// Point the named head at the given commit.
    ///
    /// Returns the previous commit if the head existed.
    pub fn set_head(&mut self, name: Name, ca: CommitAddr) -> Option<CommitAddr> {
        let prev = self.set_section_value(
            Heads::ID,
            Heads::POLICY,
            Heads::LIVENESS,
            Key::Name(name),
            Value::Commit(ca),
        );
        prev.as_ref().and_then(Heads::decode)
    }

    /// Remove the named head.
    ///
    /// Entries keyed by this name in `WithName`-liveness sections are
    /// dropped with it. Their subject is gone.
    pub fn remove_head(&mut self, name: &Name) -> Option<CommitAddr> {
        let key = Key::Name(name.clone());
        let prev = self
            .sections
            .get_mut(Heads::ID)
            .and_then(|s| s.entries.remove(&key));
        for section in self.sections.values_mut() {
            if section.liveness == Liveness::WithName {
                section.entries.remove(&key);
            }
        }
        prev.as_ref().and_then(Heads::decode)
    }

    /// Look-up the commit address pointed to by the given head.
    pub fn head_commit_ca(&self, head: &Head) -> Option<CommitAddr> {
        match head {
            Head::Branch(name) => self.head(name),
            Head::Commit(ca) => Some(*ca),
        }
    }

    /// Look-up the commit pointed to by the given head.
    pub fn head_commit(&self, head: &Head) -> Option<&Commit> {
        self.head_commit_ca(head)
            .and_then(|ca| self.commits.get(&ca))
    }

    /// Look-up the graph pointed to by the head.
    pub fn head_graph(&self, head: &Head) -> Option<&DataGraph> {
        self.head_commit(head)
            .and_then(|commit| self.graphs.get(&commit.graph))
    }

    /// Lookup the commit for the given name.
    pub fn named_commit(&self, name: &Name) -> Option<&Commit> {
        self.head(name).and_then(|ca| self.commits.get(&ca))
    }

    /// Look-up the graph pointed to by the given commit address.
    pub fn commit_graph_ref(&self, ca: &CommitAddr) -> Option<&DataGraph> {
        self.commits
            .get(ca)
            .and_then(|commit| self.graphs.get(&commit.graph))
    }

    /// Commit the graph at the given address.
    pub fn commit_graph(
        &mut self,
        timestamp: Timestamp,
        parent_ca: Option<CommitAddr>,
        graph_ca: GraphAddr,
        graph: impl FnOnce() -> DataGraph,
    ) -> CommitAddr {
        crate::ops::commit_graph(self, timestamp, parent_ca, graph_ca, graph)
    }

    /// Commit the graph to the given name.
    pub fn commit_graph_to_name(
        &mut self,
        timestamp: Timestamp,
        graph_ca: GraphAddr,
        graph: impl FnOnce() -> DataGraph,
        name: &Name,
    ) -> CommitAddr {
        crate::ops::commit_graph_to_name(self, timestamp, graph_ca, graph, name)
    }

    /// Commit the graph at the given address and update `head` to a new
    /// commit pointing to the graph.
    pub fn commit_graph_to_head(
        &mut self,
        timestamp: Timestamp,
        graph_ca: GraphAddr,
        graph: impl FnOnce() -> DataGraph,
        head: &mut Head,
    ) -> CommitAddr {
        crate::ops::commit_graph_to_head(self, timestamp, graph_ca, graph, head)
    }

    /// Commit the graph at the given address as a merge of `theirs` into the
    /// current head commit, and update `head` to the new merge commit.
    ///
    /// The head's current commit becomes the first parent and `theirs` the
    /// merge parent.
    pub fn commit_merge_to_head(
        &mut self,
        timestamp: Timestamp,
        graph_ca: GraphAddr,
        graph: impl FnOnce() -> DataGraph,
        theirs: CommitAddr,
        head: &mut Head,
    ) -> CommitAddr {
        crate::ops::commit_merge_to_head(self, timestamp, graph_ca, graph, theirs, head)
    }

    /// Commit the graph at the given address as a canonical merge of the
    /// diverged tips `a` and `b`, and update `head` to the new merge commit.
    ///
    /// [`commit_merge_to_head`](Self::commit_merge_to_head) keeps the head's
    /// tip as the first parent. Here both the parent order and the timestamp
    /// are pure functions of the two tips. See `sync::canonical_tips` and
    /// `sync::merge_timestamp`. Peers that merge the same pair independently
    /// mint the identical merge commit. This lets their DAGs converge rather
    /// than re-diverge.
    ///
    /// The caller must have produced the graph by merging in the same
    /// canonical orientation. See
    /// [`sync::plan_sync_step`](crate::sync::plan_sync_step). The merged
    /// graph's node order depends on which side plays "ours".
    pub fn commit_merge_canonical(
        &mut self,
        a: CommitAddr,
        b: CommitAddr,
        graph_ca: GraphAddr,
        graph: impl FnOnce() -> DataGraph,
        head: &mut Head,
    ) -> CommitAddr {
        crate::ops::commit_merge_canonical(self, a, b, graph_ca, graph, head)
    }

    /// Insert a commit, computing its address from the commit's contents.
    ///
    /// A commit must not reference a parent that is not in the registry. An
    /// absent parent is cleared to `None` and absent merge parents are
    /// dropped before the address is computed. Parents are part of the hashed
    /// content, so the returned address reflects the cleared parents. To
    /// preserve a chain's addresses, insert its commits oldest-first so each
    /// parent is already present.
    ///
    /// Returns the computed [`CommitAddr`], which always matches the stored
    /// commit.
    pub fn add_commit(&mut self, mut commit: Commit) -> CommitAddr {
        if let Some(parent) = commit.parent {
            if !self.commits.contains_key(&parent) {
                commit.parent = None;
            }
        }
        commit
            .merge_parents
            .retain(|p| self.commits.contains_key(p));
        let ca = commit_addr(&commit);
        self.commits.insert(ca, commit);
        ca
    }

    /// Insert a commit under a claimed, already validated address. The
    /// caller upholds the parent-closed invariant.
    pub(crate) fn insert_commit_at(&mut self, ca: CommitAddr, commit: Commit) {
        self.commits.insert(ca, commit);
    }

    /// Insert a graph under a claimed, already validated address.
    pub(crate) fn insert_graph_at(&mut self, ca: GraphAddr, graph: DataGraph) {
        self.graphs.entry(ca).or_insert(graph);
    }

    /// Insert verified blob bytes under a claimed address.
    pub(crate) fn insert_blob_at(
        &mut self,
        section: SectionId,
        liveness: BlobLiveness,
        addr: ContentAddr,
        bytes: Bytes,
    ) {
        self.blobs
            .entry(section)
            .or_insert_with(|| BlobStore::new(liveness))
            .entries
            .entry(addr)
            .or_insert(bytes);
    }

    /// Insert a graph, computing its address from the graph's contents.
    ///
    /// Every node is canonicalized via [`NodeData::canonicalize`] before
    /// hashing, so one logical graph always lands under exactly one address.
    ///
    /// Returns the computed [`GraphAddr`], which always matches the graph.
    /// Content-addressing makes this idempotent. An existing entry for the
    /// computed address is identical and is left in place.
    pub fn add_graph(&mut self, mut graph: DataGraph) -> GraphAddr {
        graph.node_weights_mut().for_each(NodeData::canonicalize);
        let ca = graph_addr(&graph);
        self.graphs.entry(ca).or_insert(graph);
        ca
    }

    /// Insert bytes into the named blob section, computing their address.
    ///
    /// Creates the section with the given liveness if absent. An existing
    /// section's stored liveness wins.
    pub fn add_blob(
        &mut self,
        section: impl Into<SectionId>,
        liveness: BlobLiveness,
        bytes: impl Into<Bytes>,
    ) -> ContentAddr {
        self.blobs
            .entry(section.into())
            .or_insert_with(|| BlobStore::new(liveness))
            .insert(bytes)
    }

    /// Initialise head to a new initial commit pointing to an empty graph.
    pub fn init_head(&mut self, timestamp: Timestamp) -> Head {
        let graph = DataGraph::default();
        let graph_ca = graph_addr(&graph);
        let commit_ca = self.commit_graph(timestamp, None, graph_ca, || graph);
        Head::Commit(commit_ca)
    }

    /// Insert a raw value into the section with the given id. The section is
    /// created and stamped with the given semantics on first write. An
    /// existing section's stored semantics win.
    ///
    /// Returns the previous value, if any. Typed callers use
    /// [`section_insert`].
    pub fn set_section_value(
        &mut self,
        id: impl Into<SectionId>,
        policy: MergePolicy,
        liveness: Liveness,
        key: Key,
        value: Value,
    ) -> Option<Value> {
        self.sections
            .entry(id.into())
            .or_insert_with(|| Section::new(policy, liveness))
            .entries
            .insert(key, value)
    }

    /// Remove the entry under `key` from the section with the given id.
    pub fn remove_section_entry(&mut self, id: &str, key: &Key) -> Option<Value> {
        self.sections
            .get_mut(id)
            .and_then(|s| s.entries.remove(key))
    }

    /// Retain only the live content and the section entries whose stored
    /// liveness rule holds against the surviving state. See
    /// [`reach::prune`](crate::reach::prune).
    pub(crate) fn retain_live(&mut self, live: &crate::reach::LiveSet) {
        self.commits.retain(|ca, _| live.commits.contains(ca));
        self.graphs.retain(|ga, _| live.graphs.contains(ga));
        for (id, store) in &mut self.blobs {
            store.entries.retain(|addr, _| live.blob_live(id, addr));
        }
        // Root sections first. Their entries follow their commit values, so
        // the WithName pass below sees the surviving heads.
        for section in self.sections.values_mut() {
            if section.liveness != Liveness::Root {
                continue;
            }
            section.entries.retain(|_, value| match value {
                Value::Commit(ca) => live.commits.contains(ca),
                _ => true,
            });
        }
        let head_names: HashSet<Name> = self.heads().map(|(n, _)| n.clone()).collect();
        for section in self.sections.values_mut() {
            let liveness = section.liveness;
            if liveness == Liveness::Root {
                continue;
            }
            section.entries.retain(|key, _| match liveness {
                Liveness::Root | Liveness::Pinned => true,
                Liveness::WithName => {
                    matches!(key, Key::Name(name) if head_names.contains(name))
                }
                Liveness::WithCommit => {
                    matches!(key, Key::Commit(ca) if live.commits.contains(ca))
                }
                Liveness::WithGraph => {
                    matches!(key, Key::Graph(ga) if live.graphs.contains(ga))
                }
            });
        }
        // Emptied sections and stores carry no data worth serializing. A
        // later write re-stamps semantics from the writer's declaration.
        self.sections.retain(|_, s| !s.entries.is_empty());
        self.blobs.retain(|_, s| !s.entries.is_empty());
        detach_invalid_parents(&mut self.commits);
    }

    /// Merge an incoming registry into this one.
    ///
    /// Graphs, commits and blobs are inserted idempotently.
    /// Content-addressing means duplicates are identical. Sections merge per
    /// their stored policy. `Replace` entries are overwritten by differing
    /// incoming entries and reported. `KeepExisting` entries keep the local
    /// value. An incoming section unknown locally is adopted wholesale,
    /// semantics and all.
    pub fn merge(&mut self, incoming: Registry) -> MergeReport {
        let mut report = MergeReport::default();
        self.graphs.extend(incoming.graphs);
        self.commits.extend(incoming.commits);
        for (id, store) in incoming.blobs {
            self.blobs
                .entry(id)
                .or_insert_with(|| BlobStore::new(store.liveness))
                .entries
                .extend(store.entries);
        }
        for (id, section) in incoming.sections {
            let local = self
                .sections
                .entry(id.clone())
                .or_insert_with(|| Section::new(section.policy, section.liveness));
            for (key, value) in section.entries {
                match local.entries.get(&key) {
                    Some(existing) if *existing == value => {}
                    Some(existing) => match local.policy {
                        MergePolicy::KeepExisting => {}
                        MergePolicy::Replace => {
                            report.record_replaced(&id, &key, existing, &value);
                            local.entries.insert(key, value);
                        }
                    },
                    None => {
                        report.record_added(&id, &key);
                        local.entries.insert(key, value);
                    }
                }
            }
        }
        report
    }
}

impl MergeReport {
    fn record_added(&mut self, id: &str, key: &Key) {
        if id == Heads::ID {
            if let Key::Name(name) = key {
                self.heads_added.push(name.clone());
                return;
            }
        }
        self.sections_added.push((id.to_string(), key.clone()));
    }

    fn record_replaced(&mut self, id: &str, key: &Key, old: &Value, new: &Value) {
        if id == Heads::ID {
            if let (Key::Name(name), Some(old), Some(new)) =
                (key, Heads::decode(old), Heads::decode(new))
            {
                self.heads_replaced.push((name.clone(), old, new));
                return;
            }
        }
        self.sections_replaced.push((id.to_string(), key.clone()));
    }
}

/// The typed value stored under `key` in `S`'s section, if present and of
/// the declared shape.
pub fn section_get<S: SectionDecl>(reg: &Registry, key: &S::Key) -> Option<S::Value>
where
    S::Key: Clone,
{
    let key: Key = key.clone().into();
    reg.section_entry(S::ID, &key).and_then(S::decode)
}

/// Insert a typed value under `key` in `S`'s section. The section is created
/// and stamped with `S`'s declared semantics on first write.
pub fn section_insert<S: SectionDecl>(
    reg: &mut Registry,
    key: S::Key,
    value: &S::Value,
) -> Result<(), DatumError> {
    let value = S::encode(value)?;
    reg.set_section_value(S::ID, S::POLICY, S::LIVENESS, key.into(), value);
    Ok(())
}

/// Remove the entry under `key` from `S`'s section.
pub fn section_remove<S: SectionDecl>(reg: &mut Registry, key: &S::Key) -> Option<S::Value>
where
    S::Key: Clone,
{
    let key: Key = key.clone().into();
    reg.sections
        .get_mut(S::ID)
        .and_then(|s| s.entries.remove(&key))
        .as_ref()
        .and_then(S::decode)
}

/// Iterate `S`'s section entries in key order, decoding keys and values.
/// Entries of an unexpected shape are skipped.
pub fn section_iter<'a, S: SectionDecl>(
    reg: &'a Registry,
) -> impl Iterator<Item = (S::Key, S::Value)> + 'a
where
    S::Key: 'a,
    S::Value: 'a,
{
    reg.sections
        .get(S::ID)
        .into_iter()
        .flat_map(|s| s.entries.iter())
        .filter_map(|(key, value)| {
            let key = S::Key::try_from_key(key)?;
            let value = S::decode(value)?;
            Some((key, value))
        })
}

/// Encode and insert a datum-valued entry into the section with the given
/// id, creating the section with the given semantics on first write.
///
/// A convenience for erased writers such as format import. Typed callers use
/// [`section_insert`].
pub fn section_insert_datum(
    reg: &mut Registry,
    id: impl Into<SectionId>,
    policy: MergePolicy,
    liveness: Liveness,
    key: Key,
    value: &impl Serialize,
) -> Result<(), DatumError> {
    let value = value_to_datum(value)?;
    reg.set_section_value(id, policy, liveness, key, value);
    Ok(())
}

/// Set every `parent` that does not point to an existing commit to `None`.
/// Invalid merge parents are dropped likewise.
pub(crate) fn detach_invalid_parents(commits: &mut Commits) {
    let present: HashSet<CommitAddr> = commits.keys().copied().collect();
    for commit in commits.values_mut() {
        if commit
            .parent
            .is_some_and(|parent_ca| !present.contains(&parent_ca))
        {
            commit.parent = None;
        }
        commit.merge_parents.retain(|p| present.contains(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentAddr, Datum};
    use std::time::Duration;

    fn graph_addr(n: u8) -> GraphAddr {
        GraphAddr::from(ContentAddr::from([n; 32]))
    }

    fn commit_addr_raw(n: u8) -> CommitAddr {
        CommitAddr::from(ContentAddr::from([n; 32]))
    }

    fn name(s: &str) -> Name {
        s.parse().unwrap()
    }

    /// A one-node graph whose node is tagged `tag`.
    fn data_graph(tag: &str) -> DataGraph {
        let mut g = DataGraph::default();
        g.add_node(NodeData::new(tag, Datum::Map(vec![])));
        g
    }

    /// Build a registry with two independent commits, each with its own
    /// graph, and a head pointing to one of them.
    fn test_registry() -> (Registry, CommitAddr, CommitAddr) {
        let ga = graph_addr(1);
        let gb = graph_addr(2);
        let ca = commit_addr_raw(10);
        let cb = commit_addr_raw(20);
        let commit_a = Commit::new(Duration::from_secs(1), None, ga);
        let commit_b = Commit::new(Duration::from_secs(2), None, gb);
        let graphs = HashMap::from([(ga, data_graph("graph_a")), (gb, data_graph("graph_b"))]);
        let commits = HashMap::from([(ca, commit_a), (cb, commit_b)]);
        let heads = BTreeMap::from([(name("alpha"), ca)]);
        (Registry::from_parts(graphs, commits, heads), ca, cb)
    }

    #[test]
    fn heads_read_back() {
        let (reg, ca, _cb) = test_registry();
        assert_eq!(reg.head(&name("alpha")), Some(ca));
        let heads: Vec<_> = reg.heads().map(|(n, ca)| (n.clone(), ca)).collect();
        assert_eq!(heads, vec![(name("alpha"), ca)]);
        assert_eq!(reg.head_commit_ca(&Head::Branch(name("alpha"))), Some(ca));
        assert_eq!(reg.head_commit_ca(&Head::Commit(ca)), Some(ca));
    }

    #[test]
    fn remove_head_drops_with_name_metadata() {
        let (mut reg, _ca, _cb) = test_registry();
        section_insert_datum(
            &mut reg,
            "test.description",
            MergePolicy::KeepExisting,
            Liveness::WithName,
            Key::Name(name("alpha")),
            &"doc".to_string(),
        )
        .unwrap();
        assert!(
            reg.section_entry("test.description", &Key::Name(name("alpha")))
                .is_some()
        );
        reg.remove_head(&name("alpha"));
        assert_eq!(reg.head(&name("alpha")), None);
        assert!(
            reg.section_entry("test.description", &Key::Name(name("alpha")))
                .is_none()
        );
    }

    #[test]
    fn merge_adds_new_graphs_commits_heads() {
        let (mut base, _ca, _cb) = test_registry();
        let gc = graph_addr(3);
        let cc = commit_addr_raw(30);
        let commit_c = Commit::new(Duration::from_secs(3), None, gc);
        let incoming = Registry::from_parts(
            HashMap::from([(gc, data_graph("graph_c"))]),
            HashMap::from([(cc, commit_c)]),
            BTreeMap::from([(name("beta"), cc)]),
        );
        let report = base.merge(incoming);
        assert!(base.commits().contains_key(&cc));
        assert!(base.graphs().contains_key(&gc));
        assert_eq!(base.head(&name("beta")), Some(cc));
        assert_eq!(report.heads_added, vec![name("beta")]);
        assert!(report.heads_replaced.is_empty());
    }

    #[test]
    fn merge_same_head_same_commit_is_noop() {
        let (mut base, ca, _cb) = test_registry();
        let ga = base.commits()[&ca].graph;
        let commit_a = base.commits()[&ca].clone();
        let incoming = Registry::from_parts(
            HashMap::from([(ga, data_graph("graph_a"))]),
            HashMap::from([(ca, commit_a)]),
            BTreeMap::from([(name("alpha"), ca)]),
        );
        let report = base.merge(incoming);
        assert!(report.heads_added.is_empty());
        assert!(report.heads_replaced.is_empty());
    }

    #[test]
    fn merge_head_conflict_replaces() {
        let (mut base, ca, cb) = test_registry();
        let gb = base.commits()[&cb].graph;
        let commit_b = base.commits()[&cb].clone();
        let incoming = Registry::from_parts(
            HashMap::from([(gb, data_graph("graph_b"))]),
            HashMap::from([(cb, commit_b)]),
            BTreeMap::from([(name("alpha"), cb)]),
        );
        let report = base.merge(incoming);
        assert!(report.heads_added.is_empty());
        assert_eq!(report.heads_replaced, vec![(name("alpha"), ca, cb)]);
        assert_eq!(base.head(&name("alpha")), Some(cb));
    }

    #[test]
    fn merge_keep_existing_section_keeps_local() {
        let (mut base, _ca, _cb) = test_registry();
        section_insert_datum(
            &mut base,
            "test.description",
            MergePolicy::KeepExisting,
            Liveness::WithName,
            Key::Name(name("alpha")),
            &"local".to_string(),
        )
        .unwrap();
        let mut incoming = Registry::default();
        section_insert_datum(
            &mut incoming,
            "test.description",
            MergePolicy::KeepExisting,
            Liveness::WithName,
            Key::Name(name("alpha")),
            &"imported".to_string(),
        )
        .unwrap();
        section_insert_datum(
            &mut incoming,
            "test.description",
            MergePolicy::KeepExisting,
            Liveness::WithName,
            Key::Name(name("beta")),
            &"new".to_string(),
        )
        .unwrap();
        let report = base.merge(incoming);
        let local: Option<String> = crate::datum::from_datum(
            match base
                .section_entry("test.description", &Key::Name(name("alpha")))
                .unwrap()
            {
                Value::Datum(d) => d.clone(),
                _ => panic!(),
            },
        )
        .ok();
        assert_eq!(local.as_deref(), Some("local"));
        assert_eq!(
            report.sections_added,
            vec![("test.description".to_string(), Key::Name(name("beta")))]
        );
        assert!(report.sections_replaced.is_empty());
    }

    #[test]
    fn merge_adopts_unknown_sections_wholesale() {
        let (mut base, _ca, _cb) = test_registry();
        let mut incoming = Registry::default();
        // A section from a domain this "app" knows nothing about.
        section_insert_datum(
            &mut incoming,
            "laser.palette",
            MergePolicy::Replace,
            Liveness::Pinned,
            Key::Name(name("show")),
            &vec![1u8, 2, 3],
        )
        .unwrap();
        base.merge(incoming);
        let section = base.section("laser.palette").unwrap();
        assert_eq!(section.policy, MergePolicy::Replace);
        assert_eq!(section.liveness, Liveness::Pinned);
        assert_eq!(section.entries.len(), 1);
    }

    #[test]
    fn merge_extends_blob_stores() {
        let (mut base, _ca, _cb) = test_registry();
        let mut incoming = Registry::default();
        let addr = incoming.add_blob("dsp.buffer", BlobLiveness::ContentReferenced, &b"pcm"[..]);
        base.merge(incoming);
        assert_eq!(
            base.blob("dsp.buffer", &addr).map(|b| &b[..]),
            Some(&b"pcm"[..])
        );
    }

    #[test]
    fn typed_section_round_trip_via_decl() {
        struct Doc;
        impl SectionDecl for Doc {
            const ID: &'static str = "test.doc";
            const POLICY: MergePolicy = MergePolicy::KeepExisting;
            const LIVENESS: Liveness = Liveness::WithName;
            type Key = Name;
            type Value = String;
        }
        let (mut reg, _ca, _cb) = test_registry();
        section_insert::<Doc>(&mut reg, name("alpha"), &"hello".to_string()).unwrap();
        assert_eq!(
            section_get::<Doc>(&reg, &name("alpha")),
            Some("hello".to_string())
        );
        let all: Vec<_> = section_iter::<Doc>(&reg).collect();
        assert_eq!(all, vec![(name("alpha"), "hello".to_string())]);
        assert_eq!(
            section_remove::<Doc>(&mut reg, &name("alpha")),
            Some("hello".to_string())
        );
        assert_eq!(section_get::<Doc>(&reg, &name("alpha")), None);
    }

    #[test]
    fn add_graph_canonicalizes_before_hashing() {
        // The same logical node in canonical and non-canonical form must land
        // under one address.
        let node = |entries: Vec<(&str, Datum)>| {
            NodeData::new(
                "test",
                Datum::Map(
                    entries
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), v))
                        .collect(),
                ),
            )
        };
        let mut messy = DataGraph::default();
        messy.add_node(node(vec![("b", Datum::Null), ("a", Datum::Bool(true))]));
        let mut clean = DataGraph::default();
        clean.add_node(node(vec![("a", Datum::Bool(true)), ("b", Datum::Null)]));
        let mut reg = Registry::default();
        let messy_ca = reg.add_graph(messy);
        let clean_ca = reg.add_graph(clean);
        assert_eq!(messy_ca, clean_ca);
        assert!(
            reg.graph(&clean_ca)
                .unwrap()
                .node_weights()
                .all(NodeData::is_canonical)
        );
    }

    #[test]
    fn add_commit_clears_absent_parent() {
        let mut reg = Registry::default();
        let ga = graph_addr(1);
        let absent_parent = commit_addr_raw(99);
        let ca = reg.add_commit(Commit::new(Duration::from_secs(1), Some(absent_parent), ga));
        assert_eq!(reg.commits()[&ca].parent, None);
        let root = Commit::new(Duration::from_secs(1), None, ga);
        assert_eq!(ca, crate::commit_addr(&root));
    }

    #[test]
    fn add_commit_keeps_present_parent() {
        let mut reg = Registry::default();
        let root = reg.add_commit(Commit::new(Duration::from_secs(1), None, graph_addr(1)));
        let child = reg.add_commit(Commit::new(
            Duration::from_secs(2),
            Some(root),
            graph_addr(2),
        ));
        assert_eq!(reg.commits()[&child].parent, Some(root));
    }

    #[test]
    fn add_commit_drops_absent_merge_parents() {
        let mut reg = Registry::default();
        let root = reg.add_commit(Commit::new(Duration::from_secs(1), None, graph_addr(1)));
        let absent = commit_addr_raw(99);
        let merge = reg.add_commit(Commit::new_merge(
            Duration::from_secs(2),
            root,
            absent,
            graph_addr(2),
        ));
        assert!(reg.commits()[&merge].merge_parents.is_empty());
        let ordinary = Commit::new(Duration::from_secs(2), Some(root), graph_addr(2));
        assert_eq!(merge, crate::commit_addr(&ordinary));
    }

    #[test]
    fn commit_merge_to_head_mints_two_parent_commit_and_advances_head() {
        let mut reg = Registry::default();
        let ours = reg.commit_graph_to_name(
            Duration::from_secs(1),
            graph_addr(1),
            || data_graph("ours"),
            &name("alpha"),
        );
        let theirs = reg.commit_graph(Duration::from_secs(2), None, graph_addr(2), || {
            data_graph("theirs")
        });
        let mut head = Head::Branch(name("alpha"));
        let merge = reg.commit_merge_to_head(
            Duration::from_secs(3),
            graph_addr(3),
            || data_graph("merged"),
            theirs,
            &mut head,
        );
        let commit = &reg.commits()[&merge];
        assert_eq!(commit.parent, Some(ours));
        assert_eq!(commit.merge_parents, vec![theirs]);
        assert_eq!(reg.head(&name("alpha")), Some(merge));
        assert_eq!(reg.head_commit_ca(&head), Some(merge));
    }

    #[test]
    fn empty_sections_and_blobs_are_omitted_from_serialized_output() {
        let (reg, _ca, _cb) = test_registry();
        // `heads` is non-empty here, so `sections` serializes. A registry
        // with no sections at all must omit both fields.
        let bare = Registry::default();
        let s = ron::to_string(&bare).unwrap();
        assert!(!s.contains("sections"));
        assert!(!s.contains("blobs"));
        let s = ron::to_string(&reg).unwrap();
        assert!(s.contains("sections"));
        assert!(!s.contains("blobs"));
    }

    #[test]
    fn registry_serde_round_trips() {
        let (mut reg, ca, _cb) = test_registry();
        reg.add_blob("dsp.buffer", BlobLiveness::ContentReferenced, &b"pcm"[..]);
        section_insert_datum(
            &mut reg,
            "egui.view",
            MergePolicy::KeepExisting,
            Liveness::WithCommit,
            Key::Commit(ca),
            &vec![1.0f64, 2.0],
        )
        .unwrap();
        let s = ron::to_string(&reg).unwrap();
        let back: Registry = ron::de::from_str(&s).unwrap();
        assert_eq!(back.commits(), reg.commits());
        assert_eq!(back.sections(), reg.sections());
        assert_eq!(back.blobs(), reg.blobs());
        // `DataGraph` lacks `PartialEq`. Compare the graph column by address
        // set and node content.
        assert_eq!(
            back.graphs().keys().collect::<HashSet<_>>(),
            reg.graphs().keys().collect::<HashSet<_>>()
        );
        for (ga, g) in reg.graphs() {
            let b = &back.graphs()[ga];
            assert_eq!(
                b.node_weights().collect::<Vec<_>>(),
                g.node_weights().collect::<Vec<_>>()
            );
        }
    }
}
