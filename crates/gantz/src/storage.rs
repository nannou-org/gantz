//! App-specific storage utilities.
//!
//! Provides the [`Pkv`] newtype implementing [`bevy_gantz::storage::Load`] and
//! [`bevy_gantz::storage::Save`] for [`bevy_pkv::PkvStore`], along with
//! re-exports and app-specific egui memory persistence.

use bevy::log;
use bevy::prelude::Resource;
use bevy_egui::egui;
use bevy_gantz::storage::{Load, Save};
use bevy_pkv::PkvStore;

// Re-export core storage functions from bevy_gantz.
pub use bevy_gantz::storage::{
    load_focused_head, load_registry, save_focused_head, save_open_heads, save_registry,
};

// Re-export GUI storage functions from bevy_gantz_egui.
pub use bevy_gantz_egui::storage::{
    load_gui_state, load_open, load_views, save_gui_state, save_views,
};

// ---------------------------------------------------------------------------
// Pkv newtype
// ---------------------------------------------------------------------------

/// A [`Resource`] wrapping [`PkvStore`] that implements [`Load`] and [`Save`].
#[derive(Resource)]
pub struct Pkv(pub PkvStore);

impl Load for Pkv {
    type Err = bevy_pkv::GetError;
    fn get_string(&self, key: &str) -> Result<Option<String>, Self::Err> {
        match self.0.get::<String>(key) {
            Ok(v) => Ok(Some(v)),
            Err(bevy_pkv::GetError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Save for Pkv {
    type Err = bevy_pkv::SetError;
    fn set_string(&mut self, key: &str, value: &str) -> Result<(), Self::Err> {
        self.0.set_string(key, value)
    }
}

// ---------------------------------------------------------------------------
// Egui memory
// ---------------------------------------------------------------------------

mod key {
    /// The key at which egui memory (widget states) is saved/loaded.
    pub const EGUI_MEMORY: &str = "egui-memory-ron";
}

/// Save the egui Memory to storage.
pub fn save_egui_memory(storage: &mut Pkv, ctx: &egui::Context) {
    match ctx.memory(ron::to_string) {
        Ok(ron_string) => match storage.0.set_string(key::EGUI_MEMORY, &ron_string) {
            Ok(()) => log::debug!("Successfully persisted egui memory"),
            Err(e) => log::error!("Failed to persist egui memory: {e}"),
        },
        Err(e) => log::error!("Failed to serialize egui memory as RON: {e}"),
    }
}

/// Load the egui Memory from storage.
pub fn load_egui_memory(storage: &mut Pkv, ctx: &egui::Context) {
    match storage.0.get::<String>(key::EGUI_MEMORY) {
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
