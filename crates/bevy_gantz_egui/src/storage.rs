//! Storage utilities for GUI-related state.
//!
//! This module provides storage functions for views and GUI state.
//! Core storage functions (registry, graphs, commits, names) are provided
//! by `bevy_gantz::storage`.

use crate::{GraphViews, GuiState, Views};
use bevy_gantz::clone_graph;
use bevy_gantz::reg::Registry;
use bevy_log as log;
use bevy_pkv::PkvStore;
use gantz_ca as ca;
use gantz_core::node::graph::Graph;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::time::Duration;

mod key {
    /// The key at which all graph views (layout + camera) are stored.
    pub const VIEWS: &str = "views";
    /// The key at which the gantz GUI state is stored.
    pub const GUI_STATE: &str = "gui-state";
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

/// Save the GUI state to storage.
pub fn save_gui_state(storage: &mut PkvStore, state: &GuiState) {
    let state_str = match ron::to_string(&**state) {
        Err(e) => {
            log::error!("Failed to serialize GUI state: {e}");
            return;
        }
        Ok(s) => s,
    };
    match storage.set_string(key::GUI_STATE, &state_str) {
        Ok(()) => log::debug!("Successfully persisted GUI state"),
        Err(e) => log::error!("Failed to persist GUI state: {e}"),
    }
}

/// Load the GUI state from storage.
pub fn load_gui_state(storage: &PkvStore) -> GuiState {
    let Some(state_str) = storage.get::<String>(key::GUI_STATE).ok() else {
        log::debug!("No existing GUI state to load");
        return GuiState::default();
    };
    match ron::de::from_str(&state_str) {
        Ok(state) => {
            log::debug!("Successfully loaded GUI state from storage");
            GuiState(state)
        }
        Err(e) => {
            log::error!("Failed to deserialize GUI state: {e}");
            GuiState::default()
        }
    }
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
        let graph = clone_graph(registry.head_graph(&head).unwrap());
        let head_views = GraphViews::default();
        vec![(head, graph, head_views)]
    } else {
        heads
    }
}
