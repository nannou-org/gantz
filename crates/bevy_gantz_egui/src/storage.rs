//! Storage utilities for GUI-related state.
//!
//! This module provides storage functions for the gantz GUI state and egui
//! memory. Registry storage (content, sections, blobs, open heads) is
//! provided by `bevy_gantz::storage` - views, demos and descriptions ride
//! the registry's metadata sections.

use crate::{GraphView, GuiState};
use base64::Engine as _;
use bevy_egui::egui;
use bevy_gantz::reg::Registry;
use bevy_gantz::storage::{Load, Save, load, save};
use bevy_log as log;
use gantz_ca as ca;
use std::time::Duration;

mod key {
    /// The key at which the gantz GUI state is stored.
    pub const GUI_STATE: &str = "gui-state";
    /// The key at which egui memory (widget states) is saved/loaded. Versioned
    /// for the RON -> bincode switch; the old `egui-memory-ron` blob is ignored.
    pub const EGUI_MEMORY: &str = "egui-memory-bin";
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
/// Returns a vector of (head, graph, view) tuples suitable for spawning
/// entities, with each head's view read from the registry's view section.
/// Working graphs are cloned straight from the registry's stored data, so
/// opens never fail on node content; a head is dropped only when its commit
/// or graph data is missing outright. If no valid heads remain, creates a
/// default empty graph head using the provided timestamp.
pub fn load_open(
    storage: &impl Load,
    registry: &mut Registry,
    ts: Duration,
) -> Vec<(ca::Head, ca::DataGraph, GraphView)> {
    /// The head's committed graph data, if present.
    fn head_graph(registry: &Registry, head: &ca::Head) -> Option<ca::DataGraph> {
        let addr = registry.head_commit(head)?.graph;
        let graph = registry.graph(&addr).cloned();
        if graph.is_none() {
            log::error!("graph data missing for head {head:?}");
        }
        graph
    }

    // Try to load all open heads from storage, dropping heads whose data is
    // missing from the registry.
    let heads: Vec<_> = bevy_gantz::storage::load_open_heads(storage)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|head| {
            let graph = head_graph(registry, &head)?;
            // Load the view for this head's commit, or create empty.
            let head_view = registry
                .head_commit_ca(&head)
                .and_then(|ca| gantz_egui::section::view(registry, &ca))
                .map(GraphView)
                .unwrap_or_default();
            Some((head, graph, head_view))
        })
        .collect();

    // If no valid heads remain, create a default one (an empty graph).
    if heads.is_empty() {
        let head = registry.init_head(ts);
        let graph = head_graph(registry, &head).expect("an empty graph always exists");
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
    /// `egui::Memory` must survive a bincode round-trip - the format used by
    /// `save_egui_memory`/`load_egui_memory`. Guards against a serde pattern
    /// bincode can't handle creeping into egui's `Memory` on an egui bump.
    #[test]
    fn egui_memory_round_trips_through_bincode() {
        use bevy_egui::egui;
        let mem = egui::Memory::default();
        let bytes = bincode::serialize(&mem).expect("serialize egui::Memory");
        let _decoded: egui::Memory =
            bincode::deserialize(&bytes).expect("deserialize egui::Memory");
    }
}
