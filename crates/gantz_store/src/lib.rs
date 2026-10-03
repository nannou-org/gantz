//! Storage of the gantz registry and session state over key-value stores.
//!
//! Provides the [`Load`] and [`Save`] storage backend traits, the generic
//! [`load`] and [`save`] RON helpers, and functions that persist the gantz
//! registry, open heads and focused head. With the `pkv` feature,
//! [`bevy_pkv::PkvStore`] implements [`Load`] and [`Save`].
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
//! - `store-meta`: the [`StoreMeta`] stamp. See [`STORE_FORMAT`].

use base64::Engine as _;
use gantz_ca as ca;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use tracing::{debug, error, info};

#[cfg(feature = "pkv")]
mod pkv;

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

/// The store format this build writes. Bump it when a change to the stored
/// layout or encoding means an older build cannot safely write the store.
/// See [`StoreMeta`].
///
/// - 0: before formats. An integer may be in either datum form, so a graph
///   may sit under the address of another form.
/// - 1: every integer datum is in its canonical form. See
///   [`gantz_ca::datum`].
pub const STORE_FORMAT: u32 = 1;

/// Which format a store is in, and which build last wrote it.
///
/// A build must not write a store whose format is newer than
/// [`STORE_FORMAT`]. It would drop the entries it cannot read and rewrite
/// the rest in its older form.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct StoreMeta {
    /// The store format. A store from before formats reads as 0.
    #[serde(default)]
    pub format: u32,
    /// The app and version that last wrote the store, for display.
    #[serde(default)]
    pub written_by: String,
}

/// Why this build must not write a store. See [`writable_meta`].
#[derive(Clone, Debug)]
pub enum Unwritable {
    /// The store's [`StoreMeta`] cannot be read.
    Meta(String),
    /// A newer build wrote the store in a format this build does not know.
    Newer(StoreMeta),
    /// The keys of the store's indices that cannot be read. A save would
    /// rewrite them from what did load.
    Indices(Vec<String>),
}

/// Stored registry entries that [`load_registry`] could not read.
///
/// They stay listed in the store's indices, so a later save does not orphan
/// them. A newer build may be able to read them.
#[derive(Clone, Debug, Default)]
pub struct Unreadable {
    /// The keys of index entries that could not be read. A store with an
    /// unreadable index must not be written, since a save would rewrite the
    /// index from what did load.
    pub indices: Vec<String>,
    pub graphs: BTreeSet<ca::GraphAddr>,
    pub commits: BTreeSet<ca::CommitAddr>,
    pub sections: BTreeSet<ca::SectionId>,
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
#[derive(Default)]
pub struct PersistedRegistry {
    graphs: HashSet<ca::GraphAddr>,
    commits: HashSet<ca::CommitAddr>,
    blobs: BTreeMap<ca::SectionId, HashSet<ca::ContentAddr>>,
    sections: BTreeMap<ca::SectionId, ca::Section>,
    unreadable: Unreadable,
}

impl BatchWriter {
    /// Take the collected writes, leaving the buffer empty.
    pub fn take(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.writes)
    }
}

impl PersistedRegistry {
    /// Snapshot a registry whose contents are all known to be on disk.
    /// `unreadable` lists the stored entries that stay indexed. See
    /// [`Unreadable`].
    pub fn from_registry(registry: &ca::Registry, unreadable: Unreadable) -> Self {
        Self {
            graphs: registry.graphs().keys().copied().collect(),
            commits: registry.commits().keys().copied().collect(),
            blobs: registry
                .blobs()
                .iter()
                .map(|(id, store)| (id.clone(), store.entries.keys().copied().collect()))
                .collect(),
            sections: registry.sections().clone(),
            unreadable,
        }
    }

    /// A tracker that treats nothing as written, so the next save writes
    /// everything. Unreadable entries stay indexed.
    pub fn cleared(&self) -> Self {
        Self {
            unreadable: self.unreadable.clone(),
            ..Self::default()
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

impl std::fmt::Display for Unwritable {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Meta(e) => write!(f, "The store's format stamp cannot be read ({e})"),
            Self::Newer(meta) => write!(
                f,
                "The store was written by {} in store format {}. This gantz supports up \
                 to format {STORE_FORMAT}",
                meta.written_by, meta.format,
            ),
            Self::Indices(keys) => {
                write!(
                    f,
                    "The store's indices ({}) cannot be read",
                    keys.join(", ")
                )
            }
        }
    }
}

/// Serialize `value` as RON and persist it under `key`.
pub fn save<T: Serialize + ?Sized>(storage: &mut impl Save, key: &str, value: &T) {
    let s = match ron::to_string(value) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to serialize {key}: {e}");
            return;
        }
    };
    match storage.set_string(key, &s) {
        Ok(()) => debug!("Persisted {key}"),
        Err(e) => error!("Failed to persist {key}: {e}"),
    }
}

/// Load a RON-serialized value from `key`. A value that cannot be read or
/// parsed is logged and reads as `None`. See [`load_strict`] to tell the two
/// apart.
pub fn load<T: DeserializeOwned>(storage: &impl Load, key: &str) -> Option<T> {
    load_strict(storage, key).unwrap_or_else(|e| {
        error!("{e}");
        None
    })
}

/// Load a RON-serialized value from `key`. `Ok(None)` means the key is
/// absent. A value that cannot be read or parsed is an error.
pub fn load_strict<T: DeserializeOwned>(
    storage: &impl Load,
    key: &str,
) -> Result<Option<T>, String> {
    let s = match storage.get_string(key) {
        Ok(Some(s)) => s,
        Ok(None) => return Ok(None),
        Err(e) => return Err(format!("Failed to read {key}: {e}")),
    };
    match ron::de::from_str(&s) {
        Ok(v) => {
            debug!("Loaded {key}");
            Ok(Some(v))
        }
        Err(e) => Err(format!("Failed to deserialize {key}: {e}")),
    }
}

/// Load the store's [`StoreMeta`]. A store without one reads as format 0.
pub fn load_store_meta(storage: &impl Load) -> Result<StoreMeta, String> {
    Ok(load_strict(storage, key::STORE_META)?.unwrap_or_default())
}

/// Stamp the store with [`STORE_FORMAT`] and the writing build.
pub fn save_store_meta(storage: &mut impl Save, written_by: &str) {
    let meta = StoreMeta {
        format: STORE_FORMAT,
        written_by: written_by.to_string(),
    };
    save(storage, key::STORE_META, &meta);
}

/// Load the store's [`StoreMeta`] if this build may write the store.
///
/// A store written by a newer build, or one whose indices cannot be read,
/// would lose data if this build saved over it. `unreadable` is from
/// [`load_registry`].
pub fn writable_meta(
    storage: &impl Load,
    unreadable: &Unreadable,
) -> Result<StoreMeta, Unwritable> {
    let meta = load_store_meta(storage).map_err(Unwritable::Meta)?;
    if meta.format > STORE_FORMAT {
        return Err(Unwritable::Newer(meta));
    }
    if !unreadable.indices.is_empty() {
        return Err(Unwritable::Indices(unreadable.indices.clone()));
    }
    Ok(meta)
}

/// Bring a registry that loaded from a store in format `from` up to
/// [`STORE_FORMAT`], and save what changed.
///
/// A format 0 store may hold integers in either datum form, so its registry
/// is re-addressed in canonical form. Returns the graphs and commits that
/// moved, so the caller can point its own saved state at them. The caller
/// then stamps the store with [`save_store_meta`]. The stamp comes last, so
/// a crash in between upgrades again on the next start.
pub fn upgrade(
    storage: &mut impl Save,
    from: u32,
    registry: &mut ca::Registry,
    persisted: &mut PersistedRegistry,
) -> ca::Moved {
    if from >= 1 {
        return ca::Moved::default();
    }
    let moved = registry.canonicalize();
    if !moved.is_empty() {
        info!(
            "moved {} stored graphs and {} commits to canonical addresses",
            moved.graphs.len(),
            moved.commits.len(),
        );
        save_registry_incremental(storage, registry, persisted);
    }
    moved
}

/// Storage keys. See the module docs for the schema.
mod key {
    pub const GRAPH_ADDRS: &str = "graph-addrs";
    pub const COMMIT_ADDRS: &str = "commit-addrs";
    pub const BLOB_MANIFEST: &str = "blob-manifest";
    pub const SECTION_INDEX: &str = "ns-index";
    pub const OPEN_HEADS: &str = "open-heads";
    pub const FOCUSED_HEAD: &str = "focused-head";
    pub const STORE_META: &str = "store-meta";

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
/// nothing. A [`PersistedRegistry::cleared`] tracker makes this a full save.
pub fn save_registry_incremental(
    storage: &mut impl Save,
    registry: &ca::Registry,
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
        let unreadable = persisted.unreadable.graphs.iter().copied();
        let mut addrs: Vec<_> = registry
            .graphs()
            .keys()
            .copied()
            .chain(unreadable)
            .collect();
        addrs.sort();
        addrs.dedup();
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
        let unreadable = persisted.unreadable.commits.iter().copied();
        let mut addrs: Vec<_> = registry
            .commits()
            .keys()
            .copied()
            .chain(unreadable)
            .collect();
        addrs.sort();
        addrs.dedup();
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
        let unreadable = persisted.unreadable.sections.iter();
        let mut ids: Vec<&ca::SectionId> = registry.sections().keys().chain(unreadable).collect();
        ids.sort();
        ids.dedup();
        save(storage, key::SECTION_INDEX, &ids);
    }
}

/// Load the registry from storage, with the stored entries that could not
/// be read. Seed [`PersistedRegistry::from_registry`] with both, so a later
/// save keeps the unreadable entries indexed.
pub fn load_registry(storage: &impl Load) -> (ca::Registry, Unreadable) {
    let mut unreadable = Unreadable::default();

    let graph_addrs: Vec<ca::GraphAddr> =
        load_index(storage, key::GRAPH_ADDRS, &mut unreadable.indices);
    let graphs = graph_addrs
        .into_iter()
        .filter_map(|ca| load_entry(storage, &key::graph(ca), ca, &mut unreadable.graphs))
        .collect();

    let commit_addrs: Vec<ca::CommitAddr> =
        load_index(storage, key::COMMIT_ADDRS, &mut unreadable.indices);
    let commits = commit_addrs
        .into_iter()
        .filter_map(|ca| load_entry(storage, &key::commit(ca), ca, &mut unreadable.commits))
        .collect();

    let mut registry = ca::Registry::from_parts(graphs, commits, BTreeMap::new());

    // Sections load whole. The first write per section stamps its stored
    // policy and liveness.
    let section_ids: Vec<ca::SectionId> =
        load_index(storage, key::SECTION_INDEX, &mut unreadable.indices);
    for id in section_ids {
        let section_key = key::section(&id);
        let Some((id, section)) =
            load_entry::<_, ca::Section>(storage, &section_key, id, &mut unreadable.sections)
        else {
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

    (registry, unreadable)
}

/// Load an index, recording its key in `unreadable` when it cannot be read.
fn load_index<T: DeserializeOwned>(
    storage: &impl Load,
    key: &str,
    unreadable: &mut Vec<String>,
) -> Vec<T> {
    load_strict(storage, key)
        .unwrap_or_else(|e| {
            error!("{e}");
            unreadable.push(key.to_string());
            None
        })
        .unwrap_or_default()
}

/// Load the entry `id` under `key`. An entry that cannot be read is logged
/// and recorded in `unreadable`. An absent entry is skipped.
fn load_entry<Id: Ord, T: DeserializeOwned>(
    storage: &impl Load,
    key: &str,
    id: Id,
    unreadable: &mut BTreeSet<Id>,
) -> Option<(Id, T)> {
    match load_strict(storage, key) {
        Ok(entry) => Some((id, entry?)),
        Err(e) => {
            error!("{e}");
            unreadable.insert(id);
            None
        }
    }
}

/// Persist raw blob bytes under `key`, base64-encoded to fit the string store.
fn save_blob(storage: &mut impl Save, key: &str, bytes: &[u8]) {
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    match storage.set_string(key, &encoded) {
        Ok(()) => debug!("Persisted {key}"),
        Err(e) => error!("Failed to persist {key}: {e}"),
    }
}

/// Load raw blob bytes written by [`save_blob`].
fn load_blob(storage: &impl Load, key: &str) -> Option<Vec<u8>> {
    let encoded = match storage.get_string(key) {
        Ok(Some(s)) => s,
        Ok(None) => return None,
        Err(e) => {
            error!("Failed to read {key}: {e}");
            return None;
        }
    };
    match base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes()) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            error!("Failed to decode {key}: {e}");
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
    fn registry(graphs: &[(u8, u8)], heads: &[(&str, u8)]) -> ca::Registry {
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
        ca::Registry::from_parts(g, c, h)
    }

    /// Store a synthetic per-commit view entry the way the GUI persists scene
    /// views.
    fn set_view(reg: &mut ca::Registry, commit: u8, value: u8) {
        ca::section_insert_datum(
            reg,
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
        ca::prune(&mut reg, &live);
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
        let (loaded, _) = load_registry(&store);
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
        reg.set_section_value(
            "dsp.meta",
            MergePolicy::KeepExisting,
            Liveness::Pinned,
            Key::Addr(blob_addr),
            Value::Blob("dsp.buffer".to_string(), blob_addr),
        );
        reg.set_section_value(
            "test.pin",
            MergePolicy::KeepExisting,
            Liveness::Pinned,
            Key::Name(name("alpha")),
            Value::Commit(commit_addr(11)),
        );
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        let (loaded, _) = load_registry(&store);
        assert_eq!(loaded.sections(), reg.sections());
        assert_eq!(
            loaded.blob("dsp.buffer", &blob_addr).map(|b| &b[..]),
            Some(&b"pcm"[..]),
        );
    }

    // A stored graph that cannot be read stays in the index through later
    // saves, so it is not orphaned.
    #[test]
    fn unreadable_entries_stay_indexed() {
        let reg = registry(&[(1, 11), (2, 12)], &[("alpha", 11)]);
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut PersistedRegistry::default());
        let garbage = "not ron (".to_string();
        store.map.insert(key::graph(graph_addr(2)), garbage);

        let (loaded, unreadable) = load_registry(&store);
        assert!(loaded.graphs().contains_key(&graph_addr(1)));
        assert_eq!(unreadable.graphs, BTreeSet::from([graph_addr(2)]));
        assert!(unreadable.indices.is_empty());

        // A new graph rewrites the index, which keeps the unreadable graph.
        let mut persisted = PersistedRegistry::from_registry(&loaded, unreadable);
        let next = registry(&[(1, 11), (3, 13)], &[("alpha", 11)]);
        save_registry_incremental(&mut store, &next, &mut persisted);
        let index: Vec<GraphAddr> = load(&store, key::GRAPH_ADDRS).unwrap();
        assert_eq!(index, vec![graph_addr(1), graph_addr(2), graph_addr(3)]);
    }

    #[test]
    fn unreadable_indices_are_reported() {
        let mut store = MockStore::default();
        let garbage = "[".to_string();
        store.map.insert(key::GRAPH_ADDRS.to_string(), garbage);
        let (_, unreadable) = load_registry(&store);
        assert_eq!(unreadable.indices, vec![key::GRAPH_ADDRS.to_string()]);
    }

    #[test]
    fn store_meta_reads_absent_as_format_zero_and_rejects_garbage() {
        let mut store = MockStore::default();
        assert_eq!(load_store_meta(&store).unwrap(), StoreMeta::default());
        save_store_meta(&mut store, "gantz 9.9.9");
        let meta = load_store_meta(&store).unwrap();
        assert_eq!(meta.format, STORE_FORMAT);
        assert_eq!(meta.written_by, "gantz 9.9.9");
        let garbage = "(format:".to_string();
        store.map.insert(key::STORE_META.to_string(), garbage);
        assert!(load_store_meta(&store).is_err());
    }

    #[test]
    fn writable_meta_refuses_newer_unreadable_and_unindexed_stores() {
        let mut store = MockStore::default();
        let readable = Unreadable::default();
        assert_eq!(
            writable_meta(&store, &readable).unwrap(),
            StoreMeta::default()
        );
        let newer = StoreMeta {
            format: STORE_FORMAT + 1,
            written_by: "gantz 9.9.9".to_string(),
        };
        save(&mut store, key::STORE_META, &newer);
        let result = writable_meta(&store, &readable);
        assert!(matches!(result, Err(Unwritable::Newer(meta)) if meta == newer));
        let garbage = "(format:".to_string();
        store.map.insert(key::STORE_META.to_string(), garbage);
        let result = writable_meta(&store, &readable);
        assert!(matches!(result, Err(Unwritable::Meta(_))));
        save_store_meta(&mut store, "gantz 0.4.0");
        let unindexed = Unreadable {
            indices: vec![key::GRAPH_ADDRS.to_string()],
            ..Default::default()
        };
        let result = writable_meta(&store, &unindexed);
        assert!(matches!(result, Err(Unwritable::Indices(keys)) if keys == unindexed.indices));
    }

    // A format 0 store may hold a graph under the address of a non-canonical
    // integer form.
    #[test]
    fn upgrade_moves_a_format_zero_registry_to_canonical_addresses() {
        let mut old = ca::DataGraph::default();
        old.add_node(ca::NodeData::new("test", ca::Datum::U64(1)));
        let old_ga = ca::graph_addr(&old);
        let commit = Commit::new(Duration::from_secs(1), None, old_ga);
        let old_ca = ca::commit_addr(&commit);
        let mut reg = ca::Registry::from_parts(
            [(old_ga, old)].into(),
            [(old_ca, commit)].into(),
            [(name("alpha"), old_ca)].into(),
        );
        let mut persisted = PersistedRegistry::default();
        let mut store = MockStore::default();
        save_registry_incremental(&mut store, &reg, &mut persisted);
        store.take_writes();

        let mut current = reg.clone();
        assert!(upgrade(&mut store, 1, &mut current, &mut persisted).is_empty());
        assert!(store.take_writes().is_empty());

        let moved = upgrade(&mut store, 0, &mut reg, &mut persisted);
        let new_ca = moved.commit(old_ca);
        assert_ne!(new_ca, old_ca);
        let (loaded, _) = load_registry(&store);
        assert_eq!(loaded.head(&name("alpha")), Some(new_ca));
        for (ga, graph) in loaded.graphs() {
            ca::verify_graph(*ga, graph).unwrap();
        }
    }
}
