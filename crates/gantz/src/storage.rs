use crate::{
    Open,
    env::{self, Environment},
    graph,
    node::Node,
};
use bevy::log;
use bevy_pkv::PkvStore;
use gantz_ca as ca;
use std::collections::{BTreeMap, HashMap};

mod key {
    /// The key at which the gantz widget state is to be saved/loaded.
    pub const GANTZ_GUI_STATE: &str = "gantz-widget-state";
    /// All known graph addresses.
    pub const GRAPH_ADDRS: &str = "graph-addrs";
    /// All known commit addresses.
    pub const COMMIT_ADDRS: &str = "commit-addrs";
    /// The key at which the mapping from names to graph CAs is stored.
    pub const NAMES: &str = "graph-names";
    /// The key at which the list of open heads is stored.
    pub const OPEN_HEADS: &str = "open-heads";

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
pub fn save_graphs(
    storage: &mut PkvStore,
    graphs: &HashMap<ca::GraphAddr, gantz_core::node::graph::Graph<Box<dyn Node>>>,
) {
    for (&ca, graph) in graphs {
        save_graph(storage, ca, graph);
    }
}

/// Save the graph to storage at the given address.
pub fn save_graph(
    storage: &mut PkvStore,
    ca: ca::GraphAddr,
    graph: &gantz_core::node::graph::Graph<Box<dyn Node>>,
) {
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

/// Save the gantz GUI state.
pub fn save_gantz_gui_state(storage: &mut PkvStore, state: &gantz_egui::widget::GantzState) {
    let gantz_str = match ron::to_string(state) {
        Err(e) => {
            log::error!("Failed to serialize and save gantz GUI state: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::GANTZ_GUI_STATE, &gantz_str) {
        Ok(()) => log::debug!("Successfully persisted gantz GUI state"),
        Err(e) => log::error!("Failed to persis gantz GUI state: {e}"),
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
pub fn load_graphs(
    storage: &PkvStore,
    addrs: impl IntoIterator<Item = ca::GraphAddr>,
) -> HashMap<ca::GraphAddr, gantz_core::node::graph::Graph<Box<dyn Node>>> {
    addrs
        .into_iter()
        .filter_map(|ca| Some((ca, load_graph(storage, ca)?)))
        .collect()
}

/// Load the graph with the given content address from storage.
pub fn load_graph(
    storage: &PkvStore,
    ca: ca::GraphAddr,
) -> Option<gantz_core::node::graph::Graph<Box<dyn Node>>> {
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

/// Load all open heads from storage.
fn load_open_heads(storage: &PkvStore) -> Option<Vec<ca::Head>> {
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

/// Load the state of the gantz GUI from storage.
pub fn load_gantz_gui_state(storage: &PkvStore) -> gantz_egui::widget::GantzState {
    storage
        .get::<String>(key::GANTZ_GUI_STATE)
        .ok()
        .or_else(|| {
            log::debug!("No existing gantz GUI state to load");
            None
        })
        .and_then(|gantz_str| match ron::de::from_str(&gantz_str) {
            Ok(gantz) => {
                log::debug!("Successfully loaded gantz GUI state from storage");
                Some(gantz)
            }
            Err(e) => {
                log::error!("Failed to deserialize gantz GUI state: {e}");
                None
            }
        })
        .unwrap_or_else(|| {
            log::debug!("Initialising default gantz GUI state");
            gantz_egui::widget::GantzState::new()
        })
}

pub fn load_environment(storage: &PkvStore) -> Environment {
    let graph_addrs = load_graph_addrs(storage);
    let commit_addrs = load_commit_addrs(storage);
    let graphs = load_graphs(storage, graph_addrs.iter().copied());
    let commits = load_commits(storage, commit_addrs.iter().copied());
    let names = load_names(storage);
    let registry = env::Registry::new(graphs, commits, names);
    let primitives = env::primitives();
    Environment {
        primitives,
        registry,
    }
}

pub fn load_open(storage: &PkvStore, reg: &mut env::Registry) -> Open {
    // Try to load all open heads from storage.
    let heads: Vec<_> = load_open_heads(storage)
        .unwrap_or_default()
        .into_iter()
        // Filter out heads that no longer exist in the registry.
        .filter_map(|head| {
            let graph = graph::clone(reg.head_graph(&head)?);
            Some((head, graph))
        })
        .collect();

    // If no valid heads remain, create a default one.
    if heads.is_empty() {
        let head = reg.init_head(env::timestamp());
        let graph = graph::clone(reg.head_graph(&head).unwrap());
        Open {
            heads: vec![(head, graph)],
        }
    } else {
        Open { heads }
    }
}
