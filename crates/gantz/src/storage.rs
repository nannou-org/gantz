use crate::{
    Active,
    env::{self, Environment, NodeTypeRegistry},
    graph::{self, Graph, GraphNode},
    node::Node,
};
use bevy::log;
use bevy_pkv::PkvStore;
use gantz_egui::ContentAddr;
use std::collections::{BTreeMap, HashMap};

mod key {
    /// The key at which the gantz widget state is to be saved/loaded.
    pub const GANTZ_GUI_STATE: &str = "gantz-widget-state";
    /// All known graph content addresses.
    pub const GRAPH_ADDRS: &str = "graph-addrs";
    /// The key at which the mapping from names to graph CAs is stored.
    pub const GRAPH_NAMES: &str = "graph-names";
    /// The key at which the content address of the active graph is stored.
    pub const ACTIVE_GRAPH: &str = "active-graph";
    /// The key at which the name of the active graph is stored.
    pub const ACTIVE_GRAPH_NAME: &str = "active-graph-name";

    /// The key for a particular graph in storage.
    pub fn graph(ca: gantz_egui::ContentAddr) -> String {
        format!("{}", gantz_egui::fmt_content_addr(ca))
    }
}

/// Save the list of known content addresses to storage.
pub fn save_graph_addrs(storage: &mut PkvStore, addrs: &[ContentAddr]) {
    let graph_addrs_str = match ron::to_string(addrs) {
        Err(e) => {
            log::error!("Failed to serialize graph content addresses: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::GRAPH_ADDRS, &graph_addrs_str) {
        Ok(()) => log::debug!("Successfully persisted known graph content addresses"),
        Err(e) => log::error!("Failed to persist known graph content addresses: {e}"),
    }
}

/// Save all graphs to storage, keyed via their content address.
pub fn save_graphs(
    storage: &mut PkvStore,
    graphs: &HashMap<ContentAddr, gantz_core::node::graph::Graph<Box<dyn Node>>>,
) {
    for (&ca, graph) in graphs {
        save_graph(storage, ca, graph);
    }
}

/// Save the list of known content addresses to storage.
pub fn save_graph(
    storage: &mut PkvStore,
    ca: ContentAddr,
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

/// Save the graph names to storage.
pub fn save_graph_names(storage: &mut PkvStore, names: &BTreeMap<String, ContentAddr>) {
    let graph_names_str = match ron::to_string(names) {
        Err(e) => {
            log::error!("Failed to serialize graph names: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::GRAPH_NAMES, &graph_names_str) {
        Ok(()) => log::debug!("Successfully persisted graph names"),
        Err(e) => log::error!("Failed to persist graph names: {e}"),
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

/// Save the active graph to storage.
pub fn save_active_graph(storage: &mut PkvStore, ca: ContentAddr) {
    // TODO: Use hex formatter rather than `ron`.
    let active_graph_str = match ron::to_string(&ca) {
        Err(e) => {
            log::error!("Failed to serialize active graph CA: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::ACTIVE_GRAPH, &active_graph_str) {
        Ok(()) => log::debug!("Successfully persisted active graph CA"),
        Err(e) => log::error!("Failed to persist active graph CA: {e}"),
    }
}

/// Save the active graph name to storage.
pub fn save_active_graph_name(storage: &mut PkvStore, name: Option<&str>) {
    let name_opt_str = match ron::to_string(&name) {
        Err(e) => {
            log::error!("Failed to serialize active graph CA: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::ACTIVE_GRAPH_NAME, &name_opt_str) {
        Ok(()) => log::debug!("Successfully persisted active graph name"),
        Err(e) => log::error!("Failed to persist active graph name: {e}"),
    }
}

/// Load the graph addresses from storage.
pub fn load_graph_addrs(storage: &PkvStore) -> Vec<ContentAddr> {
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

/// Given access to storage and an iterator yielding known graph content
/// addresses, load those graphs into memory.
pub fn load_graphs(
    storage: &PkvStore,
    addrs: impl IntoIterator<Item = ContentAddr>,
) -> HashMap<ContentAddr, gantz_core::node::graph::Graph<Box<dyn Node>>> {
    addrs
        .into_iter()
        .filter_map(|ca| Some((ca, load_graph(storage, ca)?)))
        .collect()
}

/// Load the graph with the given content address from storage.
pub fn load_graph(
    storage: &PkvStore,
    ca: ContentAddr,
) -> Option<gantz_core::node::graph::Graph<Box<dyn Node>>> {
    let key = key::graph(ca);
    let Some(graph_str) = storage.get::<String>(&key).ok() else {
        log::debug!("No graph found for content address {key}");
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

/// Load the graph names from storage.
pub fn load_graph_names(storage: &PkvStore) -> BTreeMap<String, ContentAddr> {
    let Some(graph_names_str) = storage.get::<String>(key::GRAPH_NAMES).ok() else {
        log::debug!("No existing graph names list to load");
        return BTreeMap::default();
    };
    match ron::de::from_str(&graph_names_str) {
        Ok(names) => {
            log::debug!("Successfully loaded graph names from storage");
            names
        }
        Err(e) => {
            log::error!("Failed to deserialize graph names: {e}");
            BTreeMap::default()
        }
    }
}

/// Load the CA of the active graph if there is one.
pub fn load_active_graph(storage: &PkvStore) -> Option<ContentAddr> {
    let active_graph_str = storage.get::<String>(key::ACTIVE_GRAPH).ok()?;
    // TODO: Use from_hex instead of `ron`.
    ron::de::from_str(&active_graph_str).ok()
}

/// Load the CA of the active graph if there is one.
pub fn load_active_graph_name(storage: &PkvStore) -> Option<String> {
    let active_graph_name_str = storage.get::<String>(key::ACTIVE_GRAPH_NAME).ok()?;
    ron::de::from_str(&active_graph_name_str).ok().flatten()
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
    let graph_addrs = load_graph_addrs(&storage);
    let graphs = load_graphs(&storage, graph_addrs.iter().copied());
    let names = load_graph_names(&storage);
    let registry = env::NodeTypeRegistry { graphs, names };
    let primitives = env::primitives();
    Environment {
        primitives,
        registry,
    }
}

pub fn load_active(storage: &PkvStore, reg: &NodeTypeRegistry) -> Active {
    let graph_ca = load_active_graph(&storage);
    let graph_name = load_active_graph_name(&storage);
    let (graph_ca, graph) = graph_ca
        .and_then(|ca| {
            let graph = reg.graphs.get(&ca).map(|g| graph::clone(g))?;
            Some((ca, graph))
        })
        .unwrap_or_else(|| {
            let graph = Graph::default();
            let ca = gantz_egui::graph_content_addr(&graph);
            (ca, graph)
        });
    let graph = GraphNode { graph };
    Active {
        graph,
        graph_ca,
        graph_name,
    }
}
