//! Generic storage utilities for persisting gantz state.
//!
//! Provides [`Load`] and [`Save`] traits for abstracting over storage backends,
//! generic [`load`] and [`save`] helpers for RON serialization, and functions
//! for persisting the gantz registry, open heads and focused head.
//!
//! GUI-related storage (views, gui state) is provided by `bevy_gantz_egui::storage`.

use crate::reg::Registry;
use bevy_ecs::prelude::Resource;
use bevy_log as log;
use gantz_ca as ca;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, HashSet};

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Read strings from a key-value store.
pub trait Load {
    type Err: std::fmt::Display;
    fn get_string(&self, key: &str) -> Result<Option<String>, Self::Err>;
}

/// Write strings to a key-value store.
pub trait Save {
    type Err: std::fmt::Display;
    fn set_string(&mut self, key: &str, value: &str) -> Result<(), Self::Err>;
}

/// A [`Save`] that buffers writes instead of committing them.
///
/// Lets a caller build a batch on the main thread - serializing in place via the
/// usual `save_*` functions - then hand the collected `(key, value)` pairs to a
/// background writer. Never fails.
#[derive(Default)]
pub struct BatchWriter {
    pub writes: Vec<(String, String)>,
}

impl BatchWriter {
    /// Take the collected writes, leaving the buffer empty.
    pub fn take(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.writes)
    }
}

impl Save for BatchWriter {
    type Err = std::convert::Infallible;
    fn set_string(&mut self, key: &str, value: &str) -> Result<(), Self::Err> {
        self.writes.push((key.to_string(), value.to_string()));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Generic helpers
// ---------------------------------------------------------------------------

/// Serialize `value` as RON and persist it under `key`.
pub fn save<T: Serialize + ?Sized>(storage: &mut impl Save, key: &str, value: &T) {
    let s = match ron::to_string(value) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to serialize {key}: {e}");
            return;
        }
    };
    match storage.set_string(key, &s) {
        Ok(()) => log::debug!("Persisted {key}"),
        Err(e) => log::error!("Failed to persist {key}: {e}"),
    }
}

/// Load a RON-serialized value from `key`.
pub fn load<T: DeserializeOwned>(storage: &impl Load, key: &str) -> Option<T> {
    let s = match storage.get_string(key) {
        Ok(Some(s)) => s,
        Ok(None) => return None,
        Err(e) => {
            log::error!("Failed to read {key}: {e}");
            return None;
        }
    };
    match ron::de::from_str(&s) {
        Ok(v) => {
            log::debug!("Loaded {key}");
            Some(v)
        }
        Err(e) => {
            log::error!("Failed to deserialize {key}: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

mod key {
    /// All known graph addresses.
    pub const GRAPH_ADDRS: &str = "graph-addrs";
    /// All known commit addresses.
    pub const COMMIT_ADDRS: &str = "commit-addrs";
    /// The key at which the mapping from names to graph CAs is stored.
    pub const NAMES: &str = "graph-names";
    /// The key at which the mapping from names to descriptions is stored.
    pub const DESCRIPTIONS: &str = "graph-descriptions";
    /// The key at which the list of open heads is stored.
    pub const OPEN_HEADS: &str = "open-heads";
    /// The key at which the focused head is stored.
    pub const FOCUSED_HEAD: &str = "focused-head";

    /// The key for a particular graph in storage.
    pub fn graph(ca: gantz_ca::GraphAddr) -> String {
        format!("{}", ca)
    }

    /// The key for a particular commit in storage.
    pub fn commit(ca: gantz_ca::CommitAddr) -> String {
        format!("{}", ca)
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Tracks which graph/commit content addresses (and the small name-keyed maps)
/// are already written to storage, so [`save_registry_incremental`] only writes
/// what changed.
///
/// Seed it from the disk-loaded registry via [`PersistedRegistry::from_registry`]:
/// everything `load_registry` returns is, by definition, already on disk.
#[derive(Resource, Default)]
pub struct PersistedRegistry {
    graphs: HashSet<ca::GraphAddr>,
    commits: HashSet<ca::CommitAddr>,
    names: BTreeMap<String, ca::CommitAddr>,
    descriptions: BTreeMap<String, String>,
}

impl PersistedRegistry {
    /// Snapshot the keys of a registry whose blobs are all known to be on disk.
    pub fn from_registry<N>(registry: &Registry<N>) -> Self {
        Self {
            graphs: registry.graphs().keys().copied().collect(),
            commits: registry.commits().keys().copied().collect(),
            names: registry.names().clone(),
            descriptions: registry.descriptions().clone(),
        }
    }

    /// The number of graph blobs known to be on disk.
    pub fn graphs_len(&self) -> usize {
        self.graphs.len()
    }

    /// The number of commit blobs known to be on disk.
    pub fn commits_len(&self) -> usize {
        self.commits.len()
    }
}

/// Incrementally persist the registry, writing only what `persisted` doesn't yet
/// have. Graphs and commits are content-addressed and immutable (the key *is*
/// the content hash), so an already-written blob never needs rewriting and this
/// is O(new blobs) rather than O(graphs + commits).
///
/// A fresh [`PersistedRegistry::default`] makes this a full save.
pub fn save_registry_incremental<N: Serialize>(
    storage: &mut impl Save,
    registry: &Registry<N>,
    persisted: &mut PersistedRegistry,
) {
    // Graph blobs: write only newly-seen content addresses.
    let mut graphs_changed = false;
    for (&ca, graph) in registry.graphs() {
        if persisted.graphs.insert(ca) {
            save(storage, &key::graph(ca), graph);
            graphs_changed = true;
        }
    }
    // Prune detection: every live key is now in `persisted`, so it is a superset
    // of the live keys; a length mismatch means stale (pruned) addrs remain.
    if persisted.graphs.len() != registry.graphs().len() {
        persisted
            .graphs
            .retain(|ca| registry.graphs().contains_key(ca));
        graphs_changed = true;
    }
    if graphs_changed {
        let mut addrs: Vec<_> = registry.graphs().keys().copied().collect();
        addrs.sort();
        save(storage, key::GRAPH_ADDRS, &addrs);
    }

    // Commit blobs: same pattern.
    let mut commits_changed = false;
    for (&ca, commit) in registry.commits() {
        if persisted.commits.insert(ca) {
            save(storage, &key::commit(ca), commit);
            commits_changed = true;
        }
    }
    if persisted.commits.len() != registry.commits().len() {
        persisted
            .commits
            .retain(|ca| registry.commits().contains_key(ca));
        commits_changed = true;
    }
    if commits_changed {
        let mut addrs: Vec<_> = registry.commits().keys().copied().collect();
        addrs.sort();
        save(storage, key::COMMIT_ADDRS, &addrs);
    }

    // Tiny name-keyed maps: write only when they differ from last-persisted.
    if registry.names() != &persisted.names {
        persisted.names = registry.names().clone();
        save(storage, key::NAMES, registry.names());
    }
    if registry.descriptions() != &persisted.descriptions {
        persisted.descriptions = registry.descriptions().clone();
        save(storage, key::DESCRIPTIONS, registry.descriptions());
    }
}

/// Load the registry from storage.
pub fn load_registry<N: DeserializeOwned>(storage: &impl Load) -> Registry<N> {
    let graph_addrs: Vec<ca::GraphAddr> = load(storage, key::GRAPH_ADDRS).unwrap_or_default();
    let graphs = graph_addrs
        .into_iter()
        .filter_map(|ca| Some((ca, load(storage, &key::graph(ca))?)))
        .collect();

    let commit_addrs: Vec<ca::CommitAddr> = load(storage, key::COMMIT_ADDRS).unwrap_or_default();
    let commits = commit_addrs
        .into_iter()
        .filter_map(|ca| Some((ca, load(storage, &key::commit(ca))?)))
        .collect();

    let names = load(storage, key::NAMES).unwrap_or_default();
    let mut registry = ca::Registry::new(graphs, commits, names);
    let descriptions: std::collections::BTreeMap<String, String> =
        load(storage, key::DESCRIPTIONS).unwrap_or_default();
    for (name, description) in descriptions {
        registry.set_description(name, description);
    }
    Registry(registry)
}

// ---------------------------------------------------------------------------
// Open heads
// ---------------------------------------------------------------------------

/// Save all open heads to storage.
pub fn save_open_heads(storage: &mut impl Save, heads: &[ca::Head]) {
    save(storage, key::OPEN_HEADS, heads);
}

/// Load all open heads from storage.
pub fn load_open_heads(storage: &impl Load) -> Option<Vec<ca::Head>> {
    load(storage, key::OPEN_HEADS)
}

// ---------------------------------------------------------------------------
// Focused head
// ---------------------------------------------------------------------------

/// Save the focused head to storage.
pub fn save_focused_head(storage: &mut impl Save, head: &ca::Head) {
    save(storage, key::FOCUSED_HEAD, head);
}

/// Load the focused head from storage.
pub fn load_focused_head(storage: &impl Load) -> Option<ca::Head> {
    load(storage, key::FOCUSED_HEAD)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_ca::{Commit, CommitAddr, ContentAddr, GraphAddr};
    use gantz_core::node::graph::Graph;
    use std::collections::{HashMap, HashSet};
    use std::time::Duration;

    /// A mock key-value store recording the keys written to it.
    #[derive(Default)]
    struct MockStore {
        map: HashMap<String, String>,
        writes: Vec<String>,
    }

    impl MockStore {
        fn take_writes(&mut self) -> Vec<String> {
            std::mem::take(&mut self.writes)
        }
    }

    impl Save for MockStore {
        type Err = std::convert::Infallible;
        fn set_string(&mut self, key: &str, value: &str) -> Result<(), Self::Err> {
            self.map.insert(key.to_string(), value.to_string());
            self.writes.push(key.to_string());
            Ok(())
        }
    }

    impl Load for MockStore {
        type Err = std::convert::Infallible;
        fn get_string(&self, key: &str) -> Result<Option<String>, Self::Err> {
            Ok(self.map.get(key).cloned())
        }
    }

    fn graph_addr(n: u8) -> GraphAddr {
        GraphAddr::from(ContentAddr::from([n; 32]))
    }

    fn commit_addr(n: u8) -> CommitAddr {
        CommitAddr::from(ContentAddr::from([n; 32]))
    }

    fn wrote(writes: &[String], key: &str) -> bool {
        writes.iter().any(|w| w == key)
    }

    /// Build a registry from `(graph, commit)` synthetic-addr pairs (one commit
    /// per graph) plus `(name, commit)` pairs. Graph blob values are empty - the
    /// dedup is keyed on the map keys, not the values.
    fn registry(graphs: &[(u8, u8)], names: &[(&str, u8)]) -> Registry<()> {
        let g = graphs
            .iter()
            .map(|&(ga, _)| (graph_addr(ga), Graph::<()>::default()))
            .collect();
        let c = graphs
            .iter()
            .map(|&(ga, ca)| {
                let commit = Commit::new(Duration::from_secs(ca as u64), None, graph_addr(ga));
                (commit_addr(ca), commit)
            })
            .collect();
        let nm = names
            .iter()
            .map(|&(n, ca)| (n.to_string(), commit_addr(ca)))
            .collect();
        Registry(ca::Registry::new(g, c, nm))
    }

    #[test]
    fn first_save_writes_all_blobs_indices_and_names() {
        let reg = registry(&[(1, 11), (2, 12)], &[("alpha", 11)]);
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        let writes = store.take_writes();
        assert!(wrote(&writes, &key::graph(graph_addr(1))));
        assert!(wrote(&writes, &key::graph(graph_addr(2))));
        assert!(wrote(&writes, key::GRAPH_ADDRS));
        assert!(wrote(&writes, &key::commit(commit_addr(11))));
        assert!(wrote(&writes, &key::commit(commit_addr(12))));
        assert!(wrote(&writes, key::COMMIT_ADDRS));
        assert!(wrote(&writes, key::NAMES));
        // Descriptions is empty, so it is not written.
        assert!(!wrote(&writes, key::DESCRIPTIONS));
    }

    #[test]
    fn resave_unchanged_writes_nothing() {
        let reg = registry(&[(1, 11), (2, 12)], &[("alpha", 11)]);
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        store.take_writes();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        assert!(store.take_writes().is_empty());
    }

    #[test]
    fn adding_graph_and_commit_writes_only_the_new_ones() {
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(
            &mut store,
            &registry(&[(1, 11)], &[("alpha", 11)]),
            &mut persisted,
        );
        store.take_writes();
        let reg = registry(&[(1, 11), (2, 12)], &[("alpha", 11)]);
        save_registry_incremental(&mut store, &reg, &mut persisted);
        let writes = store.take_writes();
        assert!(wrote(&writes, &key::graph(graph_addr(2))));
        assert!(wrote(&writes, &key::commit(commit_addr(12))));
        assert!(wrote(&writes, key::GRAPH_ADDRS));
        assert!(wrote(&writes, key::COMMIT_ADDRS));
        // Already-persisted blobs and unchanged names are not rewritten.
        assert!(!wrote(&writes, &key::graph(graph_addr(1))));
        assert!(!wrote(&writes, &key::commit(commit_addr(11))));
        assert!(!wrote(&writes, key::NAMES));
    }

    #[test]
    fn changing_description_writes_only_descriptions() {
        let mut reg = registry(&[(1, 11)], &[("alpha", 11)]);
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        store.take_writes();
        reg.set_description("alpha".to_string(), "doc".to_string());
        save_registry_incremental(&mut store, &reg, &mut persisted);
        assert_eq!(store.take_writes(), vec![key::DESCRIPTIONS.to_string()]);
    }

    #[test]
    fn pruning_rewrites_indices_and_trims_tracker() {
        let mut reg = registry(&[(1, 11), (2, 12)], &[("alpha", 11)]);
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        store.take_writes();
        // Keep only commit 11 (and graph 1, which it references).
        let required: HashSet<CommitAddr> = [commit_addr(11)].into_iter().collect();
        reg.prune_unreachable(&required);
        save_registry_incremental(&mut store, &reg, &mut persisted);
        let writes = store.take_writes();
        // Nothing new to write, but both indices shrank and are rewritten.
        assert!(wrote(&writes, key::GRAPH_ADDRS));
        assert!(wrote(&writes, key::COMMIT_ADDRS));
        assert!(!wrote(&writes, &key::graph(graph_addr(1))));
        assert!(!wrote(&writes, &key::commit(commit_addr(11))));
        // Tracker trimmed to the surviving keys.
        assert_eq!(persisted.graphs.len(), 1);
        assert_eq!(persisted.commits.len(), 1);
    }

    #[test]
    fn load_round_trips_incremental_save() {
        let reg = registry(&[(1, 11), (2, 12)], &[("alpha", 11)]);
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        let loaded: Registry<()> = load_registry(&store);
        assert_eq!(loaded.graphs().len(), reg.graphs().len());
        assert_eq!(loaded.commits().len(), reg.commits().len());
        assert_eq!(loaded.names(), reg.names());
    }

    #[test]
    fn batch_writer_collects_pairs_and_take_empties() {
        // Building a batch via the usual `save_*` path collects the same writes
        // a direct store would, as ordered (key, ron) pairs.
        let reg = registry(&[(1, 11)], &[("alpha", 11)]);
        let mut persisted = PersistedRegistry::default();
        let mut batch = BatchWriter::default();
        save_registry_incremental(&mut batch, &reg, &mut persisted);

        let keys: Vec<&str> = batch.writes.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&key::graph(graph_addr(1)).as_str()));
        assert!(keys.contains(&key::commit(commit_addr(11)).as_str()));
        assert!(keys.contains(&key::GRAPH_ADDRS));
        assert!(keys.contains(&key::COMMIT_ADDRS));
        assert!(keys.contains(&key::NAMES));
        // Values are the RON the direct `save` path would have written.
        let (_, names_ron) = batch
            .writes
            .iter()
            .find(|(k, _)| k == key::NAMES)
            .expect("names written");
        assert_eq!(names_ron, &ron::to_string(reg.names()).unwrap());

        // `take` hands off the buffer and leaves it empty.
        let taken = batch.take();
        assert!(!taken.is_empty());
        assert!(batch.writes.is_empty());
    }
}
