//! The graph registry resource.

use bevy_ecs::prelude::*;
use gantz_ca as ca;
use std::time::Duration;

/// A `Resource` wrapper around the data-level [`gantz_ca::Registry`].
///
/// The registry stores graphs as [`gantz_ca::DataGraph`] data. The UI
/// layer's reified-graph cache `bevy_gantz_egui::GraphCache` serves typed
/// graphs.
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

/// A commit timestamp, the current time since the UNIX epoch.
pub fn timestamp() -> Duration {
    let now = web_time::SystemTime::now();
    now.duration_since(web_time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
}
