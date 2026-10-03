//! Bevy state for the gantz store. `gantz_store` provides the storage itself.

use bevy_ecs::prelude::Resource;

/// Present while the store must not be written, for example because a newer
/// build wrote it. Holds the reason for display.
#[derive(Clone, Debug, Resource)]
pub struct StoreReadOnly(pub String);
