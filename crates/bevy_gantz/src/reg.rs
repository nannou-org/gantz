//! Graph registry resources and node lookup.
//!
//! Provides:
//! - [`Registry<N>`] — Bevy resource wrapping `gantz_ca::Registry`
//! - [`RegistryRef<'a, N>`] — Reference-based node lookup, constructed on-demand

use crate::BuiltinNodes;
use crate::builtin::Builtins;
use crate::head::{HeadRef, OpenHead};
use crate::view::Views;
use bevy_ecs::prelude::*;
use gantz_ca as ca;
use gantz_core::node::{self, Node as CoreNode, graph::Graph};
use std::collections::BTreeMap;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Registry resource
// ---------------------------------------------------------------------------

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

/// Create a timestamp for a commit (current time since UNIX epoch).
pub fn timestamp() -> Duration {
    let now = web_time::SystemTime::now();
    now.duration_since(web_time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
}

// ---------------------------------------------------------------------------
// RegistryRef
// ---------------------------------------------------------------------------

/// Reference-based node registry, constructed on-demand from borrowed Bevy resources.
pub struct RegistryRef<'a, N: Send + Sync + 'static> {
    ca_registry: &'a ca::Registry<Graph<N>>,
    builtins: &'a dyn Builtins<Node = N>,
}

impl<'a, N: Send + Sync + 'static> RegistryRef<'a, N> {
    /// Construct from borrowed Bevy resources.
    pub fn new(registry: &'a Registry<N>, builtins: &'a BuiltinNodes<N>) -> Self {
        Self {
            ca_registry: &registry.0,
            builtins: &*builtins.0,
        }
    }
}

impl<N: CoreNode + Send + Sync + 'static> RegistryRef<'_, N> {
    /// Look up a node by content address.
    ///
    /// Checks commit graphs first, then falls back to builtins.
    pub fn node(&self, ca: &ca::ContentAddr) -> Option<&dyn CoreNode> {
        let commit_ca = ca::CommitAddr::from(*ca);
        if let Some(graph) = self.ca_registry.commit_graph_ref(&commit_ca) {
            return Some(graph as &dyn CoreNode);
        }
        self.builtins.instance(ca).map(|n| n as &dyn CoreNode)
    }
}

impl<N: Send + Sync + 'static> RegistryRef<'_, N> {
    /// Create a node of the given type name.
    ///
    /// Checks registry names first (creating a [`gantz_egui::node::NamedRef`]),
    /// then falls back to builtins.
    pub fn create_node(&self, node_type: &str) -> Option<N>
    where
        N: From<gantz_egui::node::NamedRef>,
    {
        self.ca_registry
            .names()
            .get(node_type)
            .map(|commit_ca| {
                let ref_ = gantz_core::node::Ref::new((*commit_ca).into());
                let named = gantz_egui::node::NamedRef::new(node_type.to_string(), ref_);
                N::from(named)
            })
            .or_else(|| self.builtins.create(node_type))
    }
}

// ---------------------------------------------------------------------------
// gantz_egui trait impls
// ---------------------------------------------------------------------------

impl<N: CoreNode + Send + Sync + 'static> gantz_egui::NodeTypeRegistry for RegistryRef<'_, N> {
    fn node_types(&self) -> Vec<&str> {
        let mut types = vec![];
        types.extend(self.builtins.names());
        types.extend(self.ca_registry.names().keys().map(|s| &s[..]));
        types.sort();
        types
    }
}

impl<N: CoreNode + Send + Sync + 'static> gantz_egui::widget::graph_select::GraphRegistry
    for RegistryRef<'_, N>
{
    fn commits(&self) -> Vec<(&ca::CommitAddr, &ca::Commit)> {
        let mut commits: Vec<_> = self.ca_registry.commits().iter().collect();
        commits.sort_by(|(_, a), (_, b)| b.timestamp.cmp(&a.timestamp));
        commits
    }

    fn names(&self) -> &BTreeMap<String, ca::CommitAddr> {
        self.ca_registry.names()
    }
}

impl<N: CoreNode + Send + Sync + 'static> gantz_egui::node::NameRegistry for RegistryRef<'_, N> {
    fn name_ca(&self, name: &str) -> Option<ca::ContentAddr> {
        if let Some(commit_ca) = self.ca_registry.names().get(name) {
            return Some((*commit_ca).into());
        }
        self.builtins.content_addr(name)
    }

    fn node_exists(&self, ca: &ca::ContentAddr) -> bool {
        self.node(ca).is_some()
    }
}

impl<N: CoreNode + Send + Sync + 'static> gantz_egui::node::FnNodeNames for RegistryRef<'_, N> {
    fn fn_node_names(&self) -> Vec<String> {
        use gantz_egui::node::NameRegistry;

        let builtin_names = self
            .builtins
            .names()
            .into_iter()
            .filter_map(|name| self.builtins.content_addr(name).map(|_| name.to_string()));
        let registry_names = self.ca_registry.names().keys().cloned();
        let all_names = builtin_names.chain(registry_names);

        let get_node = |ca: &ca::ContentAddr| self.node(ca);
        let mut names: Vec<_> = all_names
            .filter(|name| {
                let meta_ctx = node::MetaCtx::new(&get_node);
                self.name_ca(name)
                    .and_then(|ca| self.node(&ca))
                    .map(|n| {
                        !n.stateful(meta_ctx)
                            && n.branches(meta_ctx).is_empty()
                            && n.n_outputs(meta_ctx) == 1
                    })
                    .unwrap_or(false)
            })
            .collect();

        names.sort();
        names
    }
}

impl<N: CoreNode + Send + Sync + 'static> gantz_egui::Registry for RegistryRef<'_, N> {
    fn node(&self, ca: &ca::ContentAddr) -> Option<&dyn CoreNode> {
        RegistryRef::node(self, ca)
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Prune unreachable graphs and commits from the registry.
pub fn prune_unused<N>(
    mut registry: ResMut<Registry<N>>,
    mut views: ResMut<Views>,
    builtins: Res<BuiltinNodes<N>>,
    heads: Query<&HeadRef, With<OpenHead>>,
) where
    N: CoreNode + Send + Sync + 'static,
{
    let node_reg = RegistryRef::new(&*registry, &*builtins);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    let head_iter = heads.iter().map(|h| &**h);
    let required = gantz_core::reg::required_commits(&get_node, &registry, head_iter);
    registry.prune_unreachable(&required);
    views.retain(|ca, _| required.contains(ca));
}
