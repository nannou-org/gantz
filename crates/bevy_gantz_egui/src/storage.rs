//! Storage utilities for GUI-related state.
//!
//! This module provides storage functions for views and GUI state.
//! Core storage functions (registry, graphs, commits, names) are provided
//! by `bevy_gantz::storage`.

use crate::{GraphView, GuiState, Views};
use base64::Engine as _;
use bevy_ecs::prelude::Resource;
use bevy_egui::egui;
use bevy_gantz::clone_graph;
use bevy_gantz::reg::Registry;
use bevy_gantz::storage::{Load, Save, load, save};
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::node::graph::Graph;
use serde::de::DeserializeOwned;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

mod key {
    /// The key at which all graph views were stored as a single blob (legacy;
    /// still read for backwards compatibility, see [`super::load_views`]).
    pub const VIEWS: &str = "views";
    /// Index of commit addresses that have a per-commit persisted view.
    pub const VIEW_ADDRS: &str = "view-addrs";
    /// The key at which the gantz GUI state is stored.
    pub const GUI_STATE: &str = "gui-state";
    /// The key at which egui memory (widget states) is saved/loaded. Versioned
    /// for the RON -> bincode switch; the old `egui-memory-ron` blob is ignored.
    pub const EGUI_MEMORY: &str = "egui-memory-bin";

    /// The key for a particular commit's view.
    pub fn view(ca: gantz_ca::CommitAddr) -> String {
        format!("view-{ca}")
    }
}

/// Tracks the views already written to storage (and their last-persisted form),
/// so [`save_views_incremental`] only serializes/writes those that changed.
///
/// Seed it from the disk-loaded views via [`PersistedViews::from_views`].
#[derive(Resource, Default)]
pub struct PersistedViews(HashMap<ca::CommitAddr, gantz_egui::SceneView>);

impl PersistedViews {
    /// Snapshot the views known to be on disk.
    pub fn from_views(views: &Views) -> Self {
        Self(views.iter().map(|(&ca, v)| (ca, v.clone())).collect())
    }
}

/// Save all graph views to storage under a single key (legacy whole-map form).
pub fn save_views(storage: &mut impl bevy_gantz::storage::Save, views: &Views) {
    let sorted: std::collections::BTreeMap<_, _> = views.iter().collect();
    save(storage, key::VIEWS, &sorted);
}

/// Incrementally persist views, one key per commit.
///
/// Writes only the views whose commit is in `valid_commits` and that differ from
/// the last-persisted form, and rewrites the small index only when the set of
/// persisted commits changes. No change since the last persist writes nothing -
/// a camera pan on one head writes just that one view, not the whole map.
pub fn save_views_incremental(
    storage: &mut impl Save,
    views: &Views,
    valid_commits: &HashSet<ca::CommitAddr>,
    persisted: &mut PersistedViews,
) {
    let mut index_changed = false;
    for (&ca, view) in views.iter() {
        // Only persist views for commits that still exist.
        if !valid_commits.contains(&ca) {
            continue;
        }
        if persisted.0.get(&ca) != Some(view) {
            save(storage, &key::view(ca), view);
            // A newly-seen commit grows the index.
            index_changed |= persisted.0.insert(ca, view.clone()).is_none();
        }
    }
    // Drop tracked views whose commit no longer exists.
    let before = persisted.0.len();
    persisted.0.retain(|ca, _| valid_commits.contains(ca));
    index_changed |= persisted.0.len() != before;

    if index_changed {
        let mut addrs: Vec<_> = persisted.0.keys().copied().collect();
        addrs.sort();
        save(storage, key::VIEW_ADDRS, &addrs);
    }
}

/// Load all graph views from storage.
///
/// Reads the per-commit view keys via the `view-addrs` index, falling back to
/// the legacy single-key blob for stores written before views were split per
/// commit.
pub fn load_views(storage: &impl Load) -> Views {
    if let Some(addrs) = load::<Vec<ca::CommitAddr>>(storage, key::VIEW_ADDRS) {
        return Views(
            addrs
                .into_iter()
                .filter_map(|ca| Some((ca, load(storage, &key::view(ca))?)))
                .collect(),
        );
    }
    // Legacy: a single whole-map blob.
    Views(
        load::<HashMap<ca::CommitAddr, gantz_egui::SceneView>>(storage, key::VIEWS)
            .unwrap_or_default(),
    )
}

/// Save the GUI state to storage.
pub fn save_gui_state(storage: &mut impl bevy_gantz::storage::Save, state: &GuiState) {
    save(storage, key::GUI_STATE, &**state);
}

/// Load the GUI state from storage.
pub fn load_gui_state(storage: &impl Load) -> GuiState {
    GuiState(load(storage, key::GUI_STATE).unwrap_or_default())
}

/// Load the open heads data from storage.
///
/// Returns a vector of (head, graph, views) tuples suitable for spawning entities.
/// If no valid heads remain, creates a default empty graph head using the provided timestamp.
pub fn load_open<N>(
    storage: &impl Load,
    registry: &mut Registry<N>,
    views: &Views,
    ts: Duration,
) -> Vec<(ca::Head, Graph<N>, GraphView)>
where
    N: 'static + Clone + DeserializeOwned + ca::CaHash,
{
    // Try to load all open heads from storage.
    let heads: Vec<_> = bevy_gantz::storage::load_open_heads(storage)
        .unwrap_or_default()
        .into_iter()
        // Filter out heads that no longer exist in the registry.
        .filter_map(|head| {
            let graph = clone_graph(registry.head_graph(&head)?);
            // Load the views for this head's commit, or create empty.
            let head_view = registry
                .head_commit_ca(&head)
                .and_then(|ca| views.get(ca).cloned())
                .map(GraphView)
                .unwrap_or_default();
            Some((head, graph, head_view))
        })
        .collect();

    // If no valid heads remain, create a default one.
    if heads.is_empty() {
        let head = registry.init_head(ts);
        let graph = clone_graph(registry.head_graph(&head).unwrap());
        let head_view = GraphView::default();
        vec![(head, graph, head_view)]
    } else {
        heads
    }
}

/// Save the egui Memory to storage.
///
/// Serialized with bincode rather than RON: egui memory is large and
/// RON-encoding it - escaping the nested per-entry RON strings egui stores -
/// dominated the persist cost. The compact binary is base64-encoded to fit the
/// string-keyed store. (egui only re-serializes entries touched this session, so
/// the inner cost is bounded; the win is removing the outer RON encoding.)
pub fn save_egui_memory(storage: &mut impl Save, ctx: &egui::Context) {
    let bytes = match ctx.memory(|m| bincode::serialize(m)) {
        Ok(bytes) => bytes,
        Err(e) => {
            log::error!("Failed to serialize egui memory: {e}");
            return;
        }
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    match storage.set_string(key::EGUI_MEMORY, &encoded) {
        Ok(()) => log::debug!("Persisted {}", key::EGUI_MEMORY),
        Err(e) => log::error!("Failed to persist egui memory: {e}"),
    }
}

/// Load the egui Memory from storage (see [`save_egui_memory`]).
pub fn load_egui_memory(storage: &impl Load, ctx: &egui::Context) {
    let Some(encoded) = storage.get_string(key::EGUI_MEMORY).ok().flatten() else {
        return;
    };
    let memory = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .ok()
        .and_then(|bytes| bincode::deserialize::<egui::Memory>(&bytes).ok());
    if let Some(memory) = memory {
        ctx.memory_mut(|m| {
            // Preserve the live zoom factor rather than restoring the persisted
            // one. egui's `zoom_factor` is the display-driven scale here (set by
            // bevy_egui from `native_pixels_per_point`), not a user preference.
            // Persisted memory can carry a stale value from older bevy_egui that
            // folded the display scale into egui's zoom via `set_pixels_per_point`,
            // which now double-applies on top of `native_pixels_per_point` and
            // over-scales the UI on fractional/HiDPI displays.
            let zoom_factor = m.options.zoom_factor;
            *m = memory;
            m.options.zoom_factor = zoom_factor;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_ca::{CommitAddr, ContentAddr};

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

    fn commit_addr(n: u8) -> CommitAddr {
        CommitAddr::from(ContentAddr::from([n; 32]))
    }

    /// A view distinguishable by `n` (so changing it is detectable).
    fn view(n: f32) -> gantz_egui::SceneView {
        gantz_egui::SceneView {
            camera: gantz_egui::Camera {
                center: egui::pos2(n, n),
                zoom: 1.0,
            },
            layout: Default::default(),
        }
    }

    fn views(entries: &[(u8, f32)]) -> Views {
        Views(
            entries
                .iter()
                .map(|&(c, n)| (commit_addr(c), view(n)))
                .collect(),
        )
    }

    fn valid(commits: &[u8]) -> HashSet<CommitAddr> {
        commits.iter().map(|&c| commit_addr(c)).collect()
    }

    fn wrote(writes: &[String], key: &str) -> bool {
        writes.iter().any(|w| w == key)
    }

    #[test]
    fn first_save_writes_each_view_and_the_index() {
        let v = views(&[(1, 10.0), (2, 20.0)]);
        let mut persisted = PersistedViews::default();
        let mut store = MockStore::default();
        save_views_incremental(&mut store, &v, &valid(&[1, 2]), &mut persisted);
        let writes = store.take_writes();
        assert!(wrote(&writes, &key::view(commit_addr(1))));
        assert!(wrote(&writes, &key::view(commit_addr(2))));
        assert!(wrote(&writes, key::VIEW_ADDRS));
    }

    #[test]
    fn unchanged_views_write_nothing() {
        let v = views(&[(1, 10.0), (2, 20.0)]);
        let mut persisted = PersistedViews::default();
        let mut store = MockStore::default();
        save_views_incremental(&mut store, &v, &valid(&[1, 2]), &mut persisted);
        store.take_writes();
        save_views_incremental(&mut store, &v, &valid(&[1, 2]), &mut persisted);
        assert!(store.take_writes().is_empty());
    }

    #[test]
    fn changing_one_view_writes_only_that_view_not_the_index() {
        let mut persisted = PersistedViews::default();
        let mut store = MockStore::default();
        save_views_incremental(
            &mut store,
            &views(&[(1, 10.0), (2, 20.0)]),
            &valid(&[1, 2]),
            &mut persisted,
        );
        store.take_writes();
        // Commit 2's view moves; commit 1's is untouched.
        let changed = views(&[(1, 10.0), (2, 25.0)]);
        save_views_incremental(&mut store, &changed, &valid(&[1, 2]), &mut persisted);
        let writes = store.take_writes();
        assert_eq!(writes, vec![key::view(commit_addr(2))]);
    }

    #[test]
    fn views_for_unknown_commits_are_skipped() {
        let v = views(&[(1, 10.0), (9, 90.0)]);
        let mut persisted = PersistedViews::default();
        let mut store = MockStore::default();
        // Only commit 1 exists.
        save_views_incremental(&mut store, &v, &valid(&[1]), &mut persisted);
        let writes = store.take_writes();
        assert!(wrote(&writes, &key::view(commit_addr(1))));
        assert!(!wrote(&writes, &key::view(commit_addr(9))));
    }

    #[test]
    fn pruning_a_commit_rewrites_the_index_and_trims_the_tracker() {
        let v = views(&[(1, 10.0), (2, 20.0)]);
        let mut persisted = PersistedViews::default();
        let mut store = MockStore::default();
        save_views_incremental(&mut store, &v, &valid(&[1, 2]), &mut persisted);
        store.take_writes();
        // Commit 2 is gone; its view should drop from the tracker + index.
        save_views_incremental(&mut store, &v, &valid(&[1]), &mut persisted);
        let writes = store.take_writes();
        assert_eq!(writes, vec![key::VIEW_ADDRS.to_string()]);
        assert_eq!(persisted.0.len(), 1);
    }

    #[test]
    fn load_round_trips_incremental_save() {
        let v = views(&[(1, 10.0), (2, 20.0)]);
        let mut persisted = PersistedViews::default();
        let mut store = MockStore::default();
        save_views_incremental(&mut store, &v, &valid(&[1, 2]), &mut persisted);
        let loaded = load_views(&store);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get(&commit_addr(2)), Some(&view(20.0)));
    }

    /// `egui::Memory` must survive a bincode round-trip - the format used by
    /// `save_egui_memory`/`load_egui_memory`. Guards against a serde pattern
    /// bincode can't handle creeping into egui's `Memory` on an egui bump.
    #[test]
    fn egui_memory_round_trips_through_bincode() {
        let mem = egui::Memory::default();
        let bytes = bincode::serialize(&mem).expect("serialize egui::Memory");
        let _decoded: egui::Memory =
            bincode::deserialize(&bytes).expect("deserialize egui::Memory");
    }
}
