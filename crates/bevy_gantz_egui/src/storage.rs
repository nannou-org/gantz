//! Storage utilities for GUI-related state.
//!
//! This module provides storage functions for views and GUI state.
//! Core storage functions (registry, graphs, commits, names) are provided
//! by `bevy_gantz::storage`.

use crate::{GraphViews, GuiState, Views};
use bevy_egui::egui;
use bevy_gantz::clone_graph;
use bevy_gantz::reg::Registry;
use bevy_gantz::storage::{Load, Save, load, save};
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
    /// The key at which egui memory (widget states) is saved/loaded.
    pub const EGUI_MEMORY: &str = "egui-memory-ron";
}

/// Save all graph views to storage under a single key.
pub fn save_views(storage: &mut impl bevy_gantz::storage::Save, views: &Views) {
    save(storage, key::VIEWS, &**views);
}

/// Load all graph views from storage.
pub fn load_views(storage: &impl Load) -> Views {
    Views(
        load::<HashMap<ca::CommitAddr, gantz_egui::GraphViews>>(storage, key::VIEWS)
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

/// Save the egui Memory to storage.
pub fn save_egui_memory(storage: &mut impl Save, ctx: &egui::Context) {
    ctx.memory(|m| save(storage, key::EGUI_MEMORY, m));
}

/// Load the egui Memory from storage.
pub fn load_egui_memory(storage: &impl Load, ctx: &egui::Context) {
    if let Some(memory) = load::<egui::Memory>(storage, key::EGUI_MEMORY) {
        ctx.memory_mut(|m| *m = memory);
    }
}
