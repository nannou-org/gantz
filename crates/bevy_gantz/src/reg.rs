//! The graph registry resource.

use bevy_ecs::prelude::*;
use gantz_ca as ca;
use std::time::Duration;

/// A `Resource` wrapper around the data-level [`gantz_ca::Registry`].
///
/// The registry stores graphs as concrete data ([`gantz_ca::DataGraph`]).
/// Typed graphs are served from the UI layer's reified-graph cache (see
/// `bevy_gantz_egui::GraphCache`).
#[derive(Default, Resource)]
pub struct Registry(pub ca::Registry);

impl std::ops::Deref for Registry {
    type Target = ca::Registry;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Registry {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Create a timestamp for a commit (current time since UNIX epoch).
pub fn timestamp() -> Duration {
    let now = web_time::SystemTime::now();
    now.duration_since(web_time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
}
