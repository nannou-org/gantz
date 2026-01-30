//! App-specific storage utilities.
//!
//! Most storage functionality is provided by `bevy_gantz::storage`.
//! This module only contains app-specific functions for GUI state.

use bevy::log;
use bevy_egui::egui;
use bevy_gantz::{GraphViews, Registry, Views, timestamp};
use bevy_pkv::PkvStore;
use gantz_ca as ca;

use crate::graph::{self, Graph};
use crate::node::Node;

// Re-export generic storage functions from bevy_gantz.
pub use bevy_gantz::storage::{
    load_focused_head, load_open_heads, load_registry, load_views, save_commit_addrs, save_commits,
    save_focused_head, save_graph_addrs, save_graphs, save_names, save_open_heads, save_views,
};

mod key {
    /// The key at which the gantz widget state is to be saved/loaded.
    pub const GANTZ_GUI_STATE: &str = "gantz-widget-state";
    /// The key at which egui memory (widget states) is saved/loaded.
    pub const EGUI_MEMORY: &str = "egui-memory-ron";
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
        Err(e) => log::error!("Failed to persist gantz GUI state: {e}"),
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

/// Save the egui Memory to storage.
pub fn save_egui_memory(storage: &mut PkvStore, ctx: &egui::Context) {
    match ctx.memory(ron::to_string) {
        Ok(ron_string) => match storage.set_string(key::EGUI_MEMORY, &ron_string) {
            Ok(()) => log::debug!("Successfully persisted egui memory"),
            Err(e) => log::error!("Failed to persist egui memory: {e}"),
        },
        Err(e) => log::error!("Failed to serialize egui memory as RON: {e}"),
    }
}

/// Load the egui Memory from storage.
pub fn load_egui_memory(storage: &mut PkvStore, ctx: &egui::Context) {
    match storage.get::<String>(key::EGUI_MEMORY) {
        Ok(ron_string) => match ron::from_str(&ron_string) {
            Ok(memory) => {
                ctx.memory_mut(|m| *m = memory);
                log::debug!("Successfully loaded egui memory");
            }
            Err(e) => log::warn!("Failed to parse egui memory RON: {e}"),
        },
        Err(e) => {
            log::debug!("No egui memory found in storage (this is normal on first run): {e}");
        }
    }
}

/// Load the open heads data from storage.
///
/// Returns a vector of (head, graph, views) tuples suitable for spawning entities.
/// If no valid heads remain, creates a default empty graph head.
pub fn load_open(
    storage: &PkvStore,
    registry: &mut Registry<Box<dyn Node>>,
    views: &Views,
) -> Vec<(ca::Head, Graph, GraphViews)> {
    // Try to load all open heads from storage.
    let heads: Vec<_> = load_open_heads(storage)
        .unwrap_or_default()
        .into_iter()
        // Filter out heads that no longer exist in the registry.
        .filter_map(|head| {
            let g = graph::clone(registry.head_graph(&head)?);
            // Load the views for this head's commit, or create empty.
            let head_views = registry
                .head_commit_ca(&head)
                .and_then(|ca| views.get(ca).cloned())
                .map(GraphViews)
                .unwrap_or_default();
            Some((head, g, head_views))
        })
        .collect();

    // If no valid heads remain, create a default one.
    if heads.is_empty() {
        let head = registry.init_head(timestamp());
        let g = graph::clone(registry.head_graph(&head).unwrap());
        let head_views = GraphViews::default();
        vec![(head, g, head_views)]
    } else {
        heads
    }
}
