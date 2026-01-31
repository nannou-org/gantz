//! App-specific storage utilities.
//!
//! Most storage functionality is provided by `bevy_gantz::storage`.
//! This module only contains app-specific functions for egui memory.

use bevy::log;
use bevy_egui::egui;
use bevy_pkv::PkvStore;

// Re-export generic storage functions from bevy_gantz.
pub use bevy_gantz::storage::{
    load_focused_head, load_gui_state, load_open, load_registry, load_views, save_commit_addrs,
    save_commits, save_focused_head, save_graph_addrs, save_graphs, save_gui_state, save_names,
    save_open_heads, save_views,
};

mod key {
    /// The key at which egui memory (widget states) is saved/loaded.
    pub const EGUI_MEMORY: &str = "egui-memory-ron";
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
