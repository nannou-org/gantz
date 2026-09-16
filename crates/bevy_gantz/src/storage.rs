//! Generic storage utilities for persisting gantz state.
//!
//! Provides the [`Load`] and [`Save`] storage backend traits, the generic
//! [`load`] and [`save`] RON helpers, and functions that persist the gantz
//! registry, open heads and focused head.
//!
//! # Registry schema
//!
//! Content is append-only and written once per address. Mutable metadata
//! sections are small and written whole, so an edit rewrites exactly one
//! section blob.
//!
//! - `o/c/<hex>`: one commit, RON, append-only.
//! - `o/g/<hex>`: one graph, RON, append-only.
//! - `b/<section>/<hex>`: one blob, base64 of the raw bytes, append-only.
//! - `commit-addrs`: sorted `Vec<CommitAddr>` index, rewritten on membership
//!   change.
//! - `graph-addrs`: sorted `Vec<GraphAddr>` index.
//! - `blob-manifest`: RON `Vec<(SectionId, BlobLiveness, Vec<ContentAddr>)>`,
//!   rewritten on membership change. Store liveness lives here because blob
//!   values are raw bytes.
//! - `ns/<section>`: one whole [`gantz_ca::Section`], rewritten when it
//!   differs from the last persisted form.
//! - `ns-index`: sorted `Vec<SectionId>`, rewritten on membership change.
//! - `open-heads`, `focused-head`: session state.
//!
//! `bevy_gantz_egui::storage` provides the GUI-related storage.

use crate::reg::Registry;
use base64::Engine as _;
use bevy_ecs::prelude::Resource;
use bevy_log as log;
use gantz_ca as ca;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, HashSet};

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
/// A caller builds a batch on the main thread with the usual `save_*`
/// functions, then hands the collected `(key, value)` pairs to a background
/// writer. Never fails.
#[derive(Default)]
pub struct BatchWriter {
    pub writes: Vec<(String, String)>,
}

/// Tracks what is already written to storage, so [`save_registry_incremental`]
/// only writes what changed.
///
/// Graph, commit and blob content addresses are tracked as sets. Content is
/// immutable, so a known address never needs rewriting. Sections are mutable,
/// so each is tracked as its last-persisted clone and rewritten whole when it
/// differs.
///
/// Seed it from the disk-loaded registry via
/// [`PersistedRegistry::from_registry`]. Everything `load_registry` returns
/// is already on disk.
#[derive(Resource, Default)]
pub struct PersistedRegistry {
    graphs: HashSet<ca::GraphAddr>,
    commits: HashSet<ca::CommitAddr>,
    blobs: BTreeMap<ca::SectionId, HashSet<ca::ContentAddr>>,
    sections: BTreeMap<ca::SectionId, ca::Section>,
}

impl BatchWriter {
    /// Take the collected writes, leaving the buffer empty.
    pub fn take(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.writes)
    }
}

impl PersistedRegistry {
    /// Snapshot a registry whose contents are all known to be on disk.
    pub fn from_registry(registry: &Registry) -> Self {
        Self {
            graphs: registry.graphs().keys().copied().collect(),
            commits: registry.commits().keys().copied().collect(),
            blobs: registry
                .blobs()
                .iter()
                .map(|(id, store)| (id.clone(), store.entries.keys().copied().collect()))
                .collect(),
            sections: registry.sections().clone(),
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

impl Save for BatchWriter {
    type Err = std::convert::Infallible;
    fn set_string(&mut self, key: &str, value: &str) -> Result<(), Self::Err> {
        self.writes.push((key.to_string(), value.to_string()));
        Ok(())
    }
}

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

/// Storage keys. See the module docs for the schema.
mod key {
    pub const GRAPH_ADDRS: &str = "graph-addrs";
    pub const COMMIT_ADDRS: &str = "commit-addrs";
    pub const BLOB_MANIFEST: &str = "blob-manifest";
    pub const SECTION_INDEX: &str = "ns-index";
    pub const OPEN_HEADS: &str = "open-heads";
    pub const FOCUSED_HEAD: &str = "focused-head";

    pub fn graph(ca: gantz_ca::GraphAddr) -> String {
        format!("o/g/{ca}")
    }

    pub fn commit(ca: gantz_ca::CommitAddr) -> String {
        format!("o/c/{ca}")
    }

    pub fn blob(section: &str, addr: &gantz_ca::ContentAddr) -> String {
        format!("b/{section}/{addr}")
    }

    pub fn section(id: &str) -> String {
        format!("ns/{id}")
    }
}

/// Persist the registry, writing only what `persisted` does not yet have.
///
/// The key of each content entry is its content hash, so an already-written
/// entry never needs rewriting. The cost is proportional to the new content
/// and changed sections, not to the registry. An unchanged registry writes
/// nothing. A fresh [`PersistedRegistry::default`] makes this a full save.
pub fn save_registry_incremental(
    storage: &mut impl Save,
    registry: &Registry,
    persisted: &mut PersistedRegistry,
) {
    let mut graphs_changed = false;
    for (&ca, graph) in registry.graphs() {
        if persisted.graphs.insert(ca) {
            save(storage, &key::graph(ca), graph);
            graphs_changed = true;
        }
    }
    // Every live key is now in `persisted`, so a length mismatch means pruned
    // addrs remain.
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

    // Blob stores keep raw bytes per address. Membership and store liveness
    // live in the manifest.
    let mut manifest_changed = false;
    for (id, store) in registry.blobs() {
        let tracked = persisted.blobs.entry(id.clone()).or_default();
        for (addr, bytes) in &store.entries {
            if tracked.insert(*addr) {
                save_blob(storage, &key::blob(id, addr), bytes);
                manifest_changed = true;
            }
        }
        if tracked.len() != store.entries.len() {
            tracked.retain(|addr| store.entries.contains_key(addr));
            manifest_changed = true;
        }
    }
    let tracked_stores = persisted.blobs.len();
    persisted
        .blobs
        .retain(|id, _| registry.blobs().contains_key(id));
    manifest_changed |= persisted.blobs.len() != tracked_stores;
    if manifest_changed {
        let manifest: Vec<(&ca::SectionId, ca::BlobLiveness, Vec<&ca::ContentAddr>)> = registry
            .blobs()
            .iter()
            .map(|(id, store)| (id, store.liveness, store.entries.keys().collect()))
            .collect();
        save(storage, key::BLOB_MANIFEST, &manifest);
    }

    // Sections are mutable. Each is compared against its last-persisted form
    // and rewritten whole when it differs.
    let mut index_changed = false;
    for (id, section) in registry.sections() {
        if persisted.sections.get(id) != Some(section) {
            save(storage, &key::section(id), section);
            index_changed |= persisted
                .sections
                .insert(id.clone(), section.clone())
                .is_none();
        }
    }
    let tracked_sections = persisted.sections.len();
    persisted
        .sections
        .retain(|id, _| registry.sections().contains_key(id));
    index_changed |= persisted.sections.len() != tracked_sections;
    if index_changed {
        let ids: Vec<&ca::SectionId> = registry.sections().keys().collect();
        save(storage, key::SECTION_INDEX, &ids);
    }
}

/// Load the registry from storage.
pub fn load_registry(storage: &impl Load) -> Registry {
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

    let mut registry = ca::Registry::from_parts(graphs, commits, BTreeMap::new());

    // Sections load whole. The first write per section stamps its stored
    // policy and liveness.
    let section_ids: Vec<ca::SectionId> = load(storage, key::SECTION_INDEX).unwrap_or_default();
    for id in section_ids {
        let Some(section) = load::<ca::Section>(storage, &key::section(&id)) else {
            continue;
        };
        for (key, value) in section.entries {
            registry.set_section_value(id.as_str(), section.policy, section.liveness, key, value);
        }
    }

    // `add_blob` re-derives each address from the bytes, so the load is
    // self-verifying. Corrupt bytes land under a different address and are
    // unreachable.
    let manifest: Vec<(ca::SectionId, ca::BlobLiveness, Vec<ca::ContentAddr>)> =
        load(storage, key::BLOB_MANIFEST).unwrap_or_default();
    for (id, liveness, addrs) in manifest {
        for addr in addrs {
            let Some(bytes) = load_blob(storage, &key::blob(&id, &addr)) else {
                continue;
            };
            registry.add_blob(id.as_str(), liveness, bytes);
        }
    }

    Registry(registry)
}

/// Persist raw blob bytes under `key`, base64-encoded to fit the string store.
fn save_blob(storage: &mut impl Save, key: &str, bytes: &[u8]) {
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    match storage.set_string(key, &encoded) {
        Ok(()) => log::debug!("Persisted {key}"),
        Err(e) => log::error!("Failed to persist {key}: {e}"),
    }
}

/// Load raw blob bytes written by [`save_blob`].
fn load_blob(storage: &impl Load, key: &str) -> Option<Vec<u8>> {
    let encoded = match storage.get_string(key) {
        Ok(Some(s)) => s,
        Ok(None) => return None,
        Err(e) => {
            log::error!("Failed to read {key}: {e}");
            return None;
        }
    };
    match base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes()) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            log::error!("Failed to decode {key}: {e}");
            None
        }
    }
}

/// Save the open heads.
pub fn save_open_heads(storage: &mut impl Save, heads: &[ca::Head]) {
    save(storage, key::OPEN_HEADS, heads);
}

/// Load the open heads.
pub fn load_open_heads(storage: &impl Load) -> Option<Vec<ca::Head>> {
    load(storage, key::OPEN_HEADS)
}

/// Save the focused head.
pub fn save_focused_head(storage: &mut impl Save, head: &ca::Head) {
    save(storage, key::FOCUSED_HEAD, head);
}

/// Load the focused head.
pub fn load_focused_head(storage: &impl Load) -> Option<ca::Head> {
    load(storage, key::FOCUSED_HEAD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_ca::{
        BlobLiveness, Commit, CommitAddr, ContentAddr, GraphAddr, Key, Liveness, MergePolicy, Name,
        Value,
    };
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

    fn name(s: &str) -> Name {
        s.parse().unwrap()
    }

    fn wrote(writes: &[String], key: &str) -> bool {
        writes.iter().any(|w| w == key)
    }

    /// The `ns/<section>` key for the core heads section.
    fn heads_key() -> String {
        key::section(ca::HEADS_ID)
    }

    /// Build a registry from `(graph, commit)` synthetic-addr pairs, one commit
    /// per graph, plus `(name, commit)` head pairs. Graph values are empty
    /// because the dedup is keyed on the map keys.
    fn registry(graphs: &[(u8, u8)], heads: &[(&str, u8)]) -> Registry {
        let g = graphs
            .iter()
            .map(|&(ga, _)| (graph_addr(ga), ca::DataGraph::default()))
            .collect();
        let c = graphs
            .iter()
            .map(|&(ga, ca)| {
                let commit = Commit::new(Duration::from_secs(ca as u64), None, graph_addr(ga));
                (commit_addr(ca), commit)
            })
            .collect();
        let h = heads
            .iter()
            .map(|&(n, ca)| (name(n), commit_addr(ca)))
            .collect();
        Registry(ca::Registry::from_parts(g, c, h))
    }

    /// Store a synthetic per-commit view entry the way the GUI persists scene
    /// views.
    fn set_view(reg: &mut Registry, commit: u8, value: u8) {
        ca::section_insert_datum(
            &mut reg.0,
            "egui.view",
            MergePolicy::KeepExisting,
            Liveness::WithCommit,
            Key::Commit(commit_addr(commit)),
            &value,
        )
        .unwrap();
    }

    #[test]
    fn first_save_writes_all_content_indices_and_sections() {
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
        assert!(wrote(&writes, &heads_key()));
        assert!(wrote(&writes, key::SECTION_INDEX));
        assert!(!wrote(&writes, key::BLOB_MANIFEST));
    }

    #[test]
    fn resave_unchanged_writes_nothing() {
        let mut reg = registry(&[(1, 11), (2, 12)], &[("alpha", 11)]);
        set_view(&mut reg, 11, 1);
        reg.add_blob("dsp.buffer", BlobLiveness::Pinned, &b"pcm"[..]);
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
        assert!(!wrote(&writes, &key::graph(graph_addr(1))));
        assert!(!wrote(&writes, &key::commit(commit_addr(11))));
        assert!(!wrote(&writes, &heads_key()));
        assert!(!wrote(&writes, key::SECTION_INDEX));
    }

    #[test]
    fn changing_one_section_entry_writes_only_that_section() {
        let mut reg = registry(&[(1, 11), (2, 12)], &[("alpha", 11)]);
        set_view(&mut reg, 11, 1);
        set_view(&mut reg, 12, 2);
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        store.take_writes();
        set_view(&mut reg, 12, 3);
        save_registry_incremental(&mut store, &reg, &mut persisted);
        let writes = store.take_writes();
        assert_eq!(writes, vec![key::section("egui.view")]);
    }

    #[test]
    fn new_section_writes_the_section_and_the_index() {
        let mut reg = registry(&[(1, 11)], &[("alpha", 11)]);
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        store.take_writes();
        set_view(&mut reg, 11, 1);
        save_registry_incremental(&mut store, &reg, &mut persisted);
        let writes = store.take_writes();
        assert!(wrote(&writes, &key::section("egui.view")));
        assert!(wrote(&writes, key::SECTION_INDEX));
        assert_eq!(writes.len(), 2);
    }

    #[test]
    fn new_blob_writes_the_blob_and_the_manifest() {
        let mut reg = registry(&[(1, 11)], &[("alpha", 11)]);
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        store.take_writes();
        let addr = reg.add_blob("dsp.buffer", BlobLiveness::Pinned, &b"pcm"[..]);
        save_registry_incremental(&mut store, &reg, &mut persisted);
        let writes = store.take_writes();
        assert!(wrote(&writes, &key::blob("dsp.buffer", &addr)));
        assert!(wrote(&writes, key::BLOB_MANIFEST));
        assert_eq!(writes.len(), 2);
    }

    #[test]
    fn pruning_rewrites_indices_and_trims_tracker() {
        let mut reg = registry(&[(1, 11), (2, 12)], &[("alpha", 11)]);
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        store.take_writes();
        let live = ca::LiveSet {
            commits: HashSet::from([commit_addr(11)]),
            graphs: HashSet::from([graph_addr(1)]),
            blobs: BTreeMap::new(),
        };
        ca::prune(&mut reg.0, &live);
        save_registry_incremental(&mut store, &reg, &mut persisted);
        let writes = store.take_writes();
        // Both indices shrank, so they are rewritten.
        assert!(wrote(&writes, key::GRAPH_ADDRS));
        assert!(wrote(&writes, key::COMMIT_ADDRS));
        assert!(!wrote(&writes, &key::graph(graph_addr(1))));
        assert!(!wrote(&writes, &key::commit(commit_addr(11))));
        assert_eq!(persisted.graphs.len(), 1);
        assert_eq!(persisted.commits.len(), 1);
    }

    #[test]
    fn load_round_trips_incremental_save() {
        let mut reg = registry(&[(1, 11), (2, 12)], &[("alpha", 11)]);
        set_view(&mut reg, 11, 7);
        reg.add_blob("dsp.buffer", BlobLiveness::Pinned, &b"pcm"[..]);
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        let loaded = load_registry(&store);
        assert_eq!(loaded.graphs().len(), reg.graphs().len());
        assert_eq!(loaded.commits(), reg.commits());
        assert_eq!(loaded.sections(), reg.sections());
        assert_eq!(loaded.blobs(), reg.blobs());
        assert_eq!(loaded.head(&name("alpha")), Some(commit_addr(11)));
    }

    #[test]
    fn batch_writer_collects_pairs_and_take_empties() {
        let reg = registry(&[(1, 11)], &[("alpha", 11)]);
        let mut persisted = PersistedRegistry::default();
        let mut batch = BatchWriter::default();
        save_registry_incremental(&mut batch, &reg, &mut persisted);

        let keys: Vec<&str> = batch.writes.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&key::graph(graph_addr(1)).as_str()));
        assert!(keys.contains(&key::commit(commit_addr(11)).as_str()));
        assert!(keys.contains(&key::GRAPH_ADDRS));
        assert!(keys.contains(&key::COMMIT_ADDRS));
        assert!(keys.contains(&heads_key().as_str()));
        let (_, heads_ron) = batch
            .writes
            .iter()
            .find(|(k, _)| *k == heads_key())
            .expect("heads section written");
        let heads_section = reg.section(ca::HEADS_ID).expect("heads section");
        assert_eq!(heads_ron, &ron::to_string(heads_section).unwrap());

        let taken = batch.take();
        assert!(!taken.is_empty());
        assert!(batch.writes.is_empty());
    }

    /// Every `Value` form survives the whole-section round trip.
    #[test]
    fn section_value_forms_round_trip() {
        let mut reg = registry(&[(1, 11)], &[("alpha", 11)]);
        let blob_addr = reg.add_blob("dsp.buffer", BlobLiveness::SectionReferenced, &b"pcm"[..]);
        reg.0.set_section_value(
            "dsp.meta",
            MergePolicy::KeepExisting,
            Liveness::Pinned,
            Key::Addr(blob_addr),
            Value::Blob("dsp.buffer".to_string(), blob_addr),
        );
        reg.0.set_section_value(
            "test.pin",
            MergePolicy::KeepExisting,
            Liveness::Pinned,
            Key::Name(name("alpha")),
            Value::Commit(commit_addr(11)),
        );
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        let loaded = load_registry(&store);
        assert_eq!(loaded.sections(), reg.sections());
        assert_eq!(
            loaded.blob("dsp.buffer", &blob_addr).map(|b| &b[..]),
            Some(&b"pcm"[..]),
        );
    }
}
