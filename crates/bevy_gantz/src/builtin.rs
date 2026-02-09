//! Builtins trait and Bevy resource wrapper.

use bevy_ecs::prelude::*;

// Re-export the Builtins trait from gantz_core.
pub use gantz_core::Builtins;

/// Resource wrapper for `dyn Builtins`.
///
/// This allows storing builtins as a Bevy resource while keeping the
/// `Builtins` trait object-safe.
#[derive(Resource)]
pub struct BuiltinNodes<N: 'static + Send + Sync>(pub Box<dyn Builtins<Node = N>>);

impl<N: 'static + Send + Sync> std::ops::Deref for BuiltinNodes<N> {
    type Target = dyn Builtins<Node = N>;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}
