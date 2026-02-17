//! Graph registry resources and node lookup.
//!
//! Provides:
//! - [`Registry<N>`] — Bevy resource wrapping `gantz_ca::Registry`
//! - [`lookup_node`] — Simple node lookup function

use crate::BuiltinNodes;
use crate::builtin::Builtins;
use crate::head::{HeadRef, OpenHead};
use bevy_ecs::prelude::*;
use gantz_ca as ca;
use gantz_core::node::{self, Node, graph::Graph};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Registry resource
// ---------------------------------------------------------------------------

/// A `Resource` wrapper around a [`gantz_ca::Registry`] that expects graphs of
/// type `gantz_core::node::graph::Graph<N>`.
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

/// Create a timestamp for a commit (current time since UNIX epoch).
pub fn timestamp() -> Duration {
    let now = web_time::SystemTime::now();
    now.duration_since(web_time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
}

// ---------------------------------------------------------------------------
// Node lookup
// ---------------------------------------------------------------------------

/// Look up a node by content address.
///
/// Checks commit graphs in the registry first, then falls back to builtins.
pub fn lookup_node<'a, N: 'static + Node + Send + Sync>(
    registry: &'a ca::Registry<Graph<N>>,
    builtins: &'a dyn Builtins<Node = N>,
    ca: &ca::ContentAddr,
) -> Option<&'a dyn Node> {
    let commit_ca = ca::CommitAddr::from(*ca);
    if let Some(graph) = registry.commit_graph_ref(&commit_ca) {
        return Some(graph as &dyn Node);
    }
    builtins.instance(ca).map(|n| n as &dyn Node)
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Prune unreachable graphs and commits from the registry.
pub fn prune_unused<N>(
    mut registry: ResMut<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    heads: Query<&HeadRef, With<OpenHead>>,
) where
    N: 'static + Node + Send + Sync,
{
    let get_node = |ca: &ca::ContentAddr| lookup_node(&registry, &**builtins, ca);
    let head_iter = heads.iter().map(|h| &**h);
    let required = gantz_core::reg::required_commits(&get_node, &registry, head_iter);
    registry.prune_unreachable(&required);
}
