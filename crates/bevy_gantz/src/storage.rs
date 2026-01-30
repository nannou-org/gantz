//! Generic storage utilities for persisting gantz state.

use crate::head::GraphViews;
use crate::reg::Registry;
use crate::view::Views;
use bevy_log as log;
use bevy_pkv::PkvStore;
use gantz_ca as ca;
use gantz_core::node::graph::Graph;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
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
    /// The key at which all graph views (layout + camera) are stored.
    pub const VIEWS: &str = "views";

    /// The key for a particular graph in storage.
    pub fn graph(ca: gantz_ca::GraphAddr) -> String {
        format!("{}", ca)
    }

    /// The key for a particular commit in storage.
    pub fn commit(ca: gantz_ca::CommitAddr) -> String {
        format!("{}", ca)
    }
}

/// Save the list of known graph addresses to storage.
pub fn save_graph_addrs(storage: &mut PkvStore, addrs: &[ca::GraphAddr]) {
    let graph_addrs_str = match ron::to_string(addrs) {
        Err(e) => {
            log::error!("Failed to serialize graph addresses: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::GRAPH_ADDRS, &graph_addrs_str) {
        Ok(()) => log::debug!("Successfully persisted known graph addresses"),
        Err(e) => log::error!("Failed to persist known graph addresses: {e}"),
    }
}

/// Save the list of known commit addresses to storage.
pub fn save_commit_addrs(storage: &mut PkvStore, addrs: &[ca::CommitAddr]) {
    let commit_addrs_str = match ron::to_string(addrs) {
        Err(e) => {
            log::error!("Failed to serialize commit addresses: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::COMMIT_ADDRS, &commit_addrs_str) {
        Ok(()) => log::debug!("Successfully persisted known commit addresses"),
        Err(e) => log::error!("Failed to persist known commit addresses: {e}"),
    }
}

/// Save all graphs to storage, keyed via their content address.
pub fn save_graphs<N: Serialize>(
    storage: &mut PkvStore,
    graphs: &HashMap<ca::GraphAddr, Graph<N>>,
) {
    for (&ca, graph) in graphs {
        save_graph(storage, ca, graph);
    }
}

/// Save the graph to storage at the given address.
pub fn save_graph<N: Serialize>(storage: &mut PkvStore, ca: ca::GraphAddr, graph: &Graph<N>) {
    let key = key::graph(ca);
    let graph_str = match ron::to_string(graph) {
        Err(e) => {
            log::error!("Failed to serialize graph: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(&key, &graph_str) {
        Ok(()) => log::debug!("Successfully persisted graph {key}"),
        Err(e) => log::error!("Failed to persist graph {key}: {e}"),
    }
}

/// Save all commits to storage, keyed via their content address.
pub fn save_commits(storage: &mut PkvStore, commits: &HashMap<ca::CommitAddr, ca::Commit>) {
    for (&ca, commit) in commits {
        save_commit(storage, ca, commit);
    }
}

/// Save the commit to storage at the given address.
pub fn save_commit(storage: &mut PkvStore, ca: ca::CommitAddr, commit: &ca::Commit) {
    let key = key::commit(ca);
    let commit_str = match ron::to_string(commit) {
        Err(e) => {
            log::error!("Failed to serialize commit: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(&key, &commit_str) {
        Ok(()) => log::debug!("Successfully persisted commit {key}"),
        Err(e) => log::error!("Failed to persist commit {key}: {e}"),
    }
}

/// Save the names to storage.
pub fn save_names(storage: &mut PkvStore, names: &BTreeMap<String, ca::CommitAddr>) {
    let names_str = match ron::to_string(names) {
        Err(e) => {
            log::error!("Failed to serialize names: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::NAMES, &names_str) {
        Ok(()) => log::debug!("Successfully persisted names"),
        Err(e) => log::error!("Failed to persist names: {e}"),
    }
}

/// Save all open heads to storage.
pub fn save_open_heads(storage: &mut PkvStore, heads: &[ca::Head]) {
    let heads_str = match ron::to_string(heads) {
        Err(e) => {
            log::error!("Failed to serialize open heads: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::OPEN_HEADS, &heads_str) {
        Ok(()) => log::debug!("Successfully persisted {} open heads", heads.len()),
        Err(e) => log::error!("Failed to persist open heads: {e}"),
    }
}

/// Save the focused head to storage.
pub fn save_focused_head(storage: &mut PkvStore, head: &ca::Head) {
    let head_str = match ron::to_string(head) {
        Err(e) => {
            log::error!("Failed to serialize focused head: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::FOCUSED_HEAD, &head_str) {
        Ok(()) => log::debug!("Successfully persisted focused head"),
        Err(e) => log::error!("Failed to persist focused head: {e}"),
    }
}

/// Save all graph views to storage under a single key.
pub fn save_views(storage: &mut PkvStore, views: &Views) {
    // Serialize the inner HashMap, not the wrapper struct.
    let views_str = match ron::to_string(&**views) {
        Err(e) => {
            log::error!("Failed to serialize views: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::VIEWS, &views_str) {
        Ok(()) => log::debug!("Successfully persisted {} views", views.len()),
        Err(e) => log::error!("Failed to persist views: {e}"),
    }
}

/// Save the registry to storage.
pub fn save_registry<N: Serialize>(storage: &mut PkvStore, registry: &Registry<N>) {
    // Save graphs.
    let mut addrs: Vec<_> = registry.graphs().keys().copied().collect();
    addrs.sort();
    save_graph_addrs(storage, &addrs);
    save_graphs(storage, registry.graphs());

    // Save commits.
    let mut addrs: Vec<_> = registry.commits().keys().copied().collect();
    addrs.sort();
    save_commit_addrs(storage, &addrs);
    save_commits(storage, registry.commits());

    // Save names.
    save_names(storage, registry.names());
}

/// Load the graph addresses from storage.
pub fn load_graph_addrs(storage: &PkvStore) -> Vec<ca::GraphAddr> {
    let Some(graph_addrs_str) = storage.get::<String>(key::GRAPH_ADDRS).ok() else {
        log::debug!("No existing graph address list to load");
        return vec![];
    };
    match ron::de::from_str(&graph_addrs_str) {
        Ok(addrs) => {
            log::debug!("Successfully loaded graph addresses from storage");
            addrs
        }
        Err(e) => {
            log::error!("Failed to deserialize graph addresses: {e}");
            vec![]
        }
    }
}

/// Load the commit addresses from storage.
pub fn load_commit_addrs(storage: &PkvStore) -> Vec<ca::CommitAddr> {
    let Some(commit_addrs_str) = storage.get::<String>(key::COMMIT_ADDRS).ok() else {
        log::debug!("No existing commit address list to load");
        return vec![];
    };
    match ron::de::from_str(&commit_addrs_str) {
        Ok(addrs) => {
            log::debug!("Successfully loaded commit addresses from storage");
            addrs
        }
        Err(e) => {
            log::error!("Failed to deserialize commit addresses: {e}");
            vec![]
        }
    }
}

/// Given access to storage and an iterator yielding known graph content
/// addresses, load those graphs into memory.
pub fn load_graphs<N: DeserializeOwned>(
    storage: &PkvStore,
    addrs: impl IntoIterator<Item = ca::GraphAddr>,
) -> HashMap<ca::GraphAddr, Graph<N>> {
    addrs
        .into_iter()
        .filter_map(|ca| Some((ca, load_graph(storage, ca)?)))
        .collect()
}

/// Load the graph with the given content address from storage.
pub fn load_graph<N: DeserializeOwned>(storage: &PkvStore, ca: ca::GraphAddr) -> Option<Graph<N>> {
    let key = key::graph(ca);
    let Some(graph_str) = storage.get::<String>(&key).ok() else {
        log::debug!("No graph found for address {key}");
        return None;
    };
    match ron::de::from_str(&graph_str) {
        Ok(graph) => {
            log::debug!("Successfully loaded graph {key} from storage");
            Some(graph)
        }
        Err(e) => {
            log::error!("Failed to deserialize graph {key}: {e}");
            None
        }
    }
}

/// Given access to storage and an iterator yielding known commit content
/// addresses, load those commits into memory.
pub fn load_commits(
    storage: &PkvStore,
    addrs: impl IntoIterator<Item = ca::CommitAddr>,
) -> HashMap<ca::CommitAddr, ca::Commit> {
    addrs
        .into_iter()
        .filter_map(|ca| Some((ca, load_commit(storage, ca)?)))
        .collect()
}

/// Load the commit with the given content address from storage.
pub fn load_commit(storage: &PkvStore, ca: ca::CommitAddr) -> Option<ca::Commit> {
    let key = key::commit(ca);
    let Some(commit_str) = storage.get::<String>(&key).ok() else {
        log::debug!("No commit found for address {key}");
        return None;
    };
    match ron::de::from_str(&commit_str) {
        Ok(commit) => {
            log::debug!("Successfully loaded commit {key} from storage");
            Some(commit)
        }
        Err(e) => {
            log::error!("Failed to deserialize commit {key}: {e}");
            None
        }
    }
}

/// Load the names from storage.
pub fn load_names(storage: &PkvStore) -> BTreeMap<String, ca::CommitAddr> {
    let Some(names_str) = storage.get::<String>(key::NAMES).ok() else {
        log::debug!("No existing names list to load");
        return BTreeMap::default();
    };
    match ron::de::from_str(&names_str) {
        Ok(names) => {
            log::debug!("Successfully loaded names from storage");
            names
        }
        Err(e) => {
            log::error!("Failed to deserialize names: {e}");
            BTreeMap::default()
        }
    }
}

/// Load all graph views from storage.
pub fn load_views(storage: &PkvStore) -> Views {
    let Some(views_str) = storage.get::<String>(key::VIEWS).ok() else {
        log::debug!("No existing views to load");
        return Views::default();
    };
    match ron::de::from_str::<HashMap<ca::CommitAddr, gantz_egui::GraphViews>>(&views_str) {
        Ok(views) => {
            log::debug!("Successfully loaded views from storage");
            Views(views)
        }
        Err(e) => {
            log::error!("Failed to deserialize views: {e}");
            Views::default()
        }
    }
}

/// Load all open heads from storage.
pub fn load_open_heads(storage: &PkvStore) -> Option<Vec<ca::Head>> {
    let Some(heads_str) = storage.get::<String>(key::OPEN_HEADS).ok() else {
        log::debug!("No existing open heads to load");
        return None;
    };
    match ron::de::from_str(&heads_str) {
        Ok(heads) => {
            log::debug!("Successfully loaded open heads");
            Some(heads)
        }
        Err(e) => {
            log::error!("Failed to deserialize open heads: {e}");
            None
        }
    }
}

/// Load the focused head from storage.
pub fn load_focused_head(storage: &PkvStore) -> Option<ca::Head> {
    let head_str = storage.get::<String>(key::FOCUSED_HEAD).ok()?;
    match ron::de::from_str(&head_str) {
        Ok(head) => {
            log::debug!("Successfully loaded focused head");
            Some(head)
        }
        Err(e) => {
            log::error!("Failed to deserialize focused head: {e}");
            None
        }
    }
}

/// Load the registry from storage.
pub fn load_registry<N: DeserializeOwned>(storage: &PkvStore) -> Registry<N> {
    let graph_addrs = load_graph_addrs(storage);
    let commit_addrs = load_commit_addrs(storage);
    let graphs = load_graphs(storage, graph_addrs.iter().copied());
    let commits = load_commits(storage, commit_addrs.iter().copied());
    let names = load_names(storage);
    Registry(ca::Registry::new(graphs, commits, names))
}

/// Load the open heads data from storage.
///
/// Returns a vector of (head, graph, views) tuples suitable for spawning entities.
/// If no valid heads remain, creates a default empty graph head using the provided timestamp.
pub fn load_open<N>(
    storage: &PkvStore,
    registry: &mut Registry<N>,
    views: &Views,
    ts: Duration,
) -> Vec<(ca::Head, Graph<N>, GraphViews)>
where
    N: Clone + DeserializeOwned + ca::CaHash + 'static,
{
    // Try to load all open heads from storage.
    let heads: Vec<_> = load_open_heads(storage)
        .unwrap_or_default()
        .into_iter()
        // Filter out heads that no longer exist in the registry.
        .filter_map(|head| {
            let graph = crate::clone_graph(registry.head_graph(&head)?);
            // Load the views for this head's commit, or create empty.
            let head_views = registry
                .head_commit_ca(&head)
                .and_then(|ca| views.get(ca).cloned())
                .map(GraphViews)
                .unwrap_or_default();
            Some((head, graph, head_views))
        })
        .collect();

    // If no valid heads remain, create a default one.
    if heads.is_empty() {
        let head = registry.init_head(ts);
        let graph = crate::clone_graph(registry.head_graph(&head).unwrap());
        let head_views = GraphViews::default();
        vec![(head, graph, head_views)]
    } else {
        heads
    }
}
