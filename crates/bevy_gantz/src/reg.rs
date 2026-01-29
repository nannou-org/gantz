//! Graph registry resource.

use bevy_ecs::prelude::*;
use gantz_ca as ca;
use gantz_core::node;

/// The graph registry containing all graphs, commits, and names.
#[derive(Resource)]
pub struct Registry<N>(pub ca::Registry<node::graph::Graph<N>>);

impl<N> Default for Registry<N> {
    fn default() -> Self {
        Self(ca::Registry::default())
    }
}

impl<N> std::ops::Deref for Registry<N> {
    type Target = ca::Registry<node::graph::Graph<N>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<N> std::ops::DerefMut for Registry<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
