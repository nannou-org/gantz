//! Generic storage utilities for persisting gantz state.
//!
//! Provides [`Load`] and [`Save`] traits for abstracting over storage backends,
//! generic [`load`] and [`save`] helpers for RON serialization, and functions
//! for persisting the gantz registry, open heads and focused head.
//!
//! GUI-related storage (views, gui state) is provided by `bevy_gantz_egui::storage`.

use crate::reg::Registry;
use bevy_log as log;
use gantz_ca as ca;
use serde::{Serialize, de::DeserializeOwned};

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

/// Save the registry to storage.
pub fn save_registry<N: Serialize>(storage: &mut impl Save, registry: &Registry<N>) {
    let mut graph_addrs: Vec<_> = registry.graphs().keys().copied().collect();
    graph_addrs.sort();
    save(storage, key::GRAPH_ADDRS, &graph_addrs);
    for (&ca, graph) in registry.graphs() {
        save(storage, &key::graph(ca), graph);
    }

    let mut commit_addrs: Vec<_> = registry.commits().keys().copied().collect();
    commit_addrs.sort();
    save(storage, key::COMMIT_ADDRS, &commit_addrs);
    for (&ca, commit) in registry.commits() {
        save(storage, &key::commit(ca), commit);
    }

    save(storage, key::NAMES, registry.names());
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
    Registry(ca::Registry::new(graphs, commits, names))
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
