//! Registry reference for node lookup and trait implementations.
//!
//! Provides [`RegistryRef`] — a unified view combining a content-addressed
//! registry with builtin nodes, implementing the various registry traits
//! required by gantz_egui widgets.

use crate::Registry;
use crate::node::{FnNodeNames, NameRegistry};
use crate::widget::gantz::NodeTypeRegistry;
use crate::widget::graph_select::GraphRegistry;
use gantz_ca as ca;
use gantz_core::node::{self, graph::Graph};
use gantz_core::{Builtins, Node};
use std::collections::BTreeMap;

/// Registry reference providing unified node access.
///
/// Combines access to a content-addressed registry (for user-defined graphs)
/// with builtin nodes, implementing all the registry traits required by
/// gantz_egui widgets.
pub struct RegistryRef<'a, N: 'static + Send + Sync> {
    ca_registry: &'a ca::Registry<Graph<N>>,
    builtins: &'a dyn Builtins<Node = N>,
}

impl<'a, N: 'static + Send + Sync> RegistryRef<'a, N> {
    /// Construct from a CA registry and builtins provider.
    pub fn new(
        ca_registry: &'a ca::Registry<Graph<N>>,
        builtins: &'a dyn Builtins<Node = N>,
    ) -> Self {
        Self {
            ca_registry,
            builtins,
        }
    }

    /// Access the underlying CA registry.
    pub fn ca_registry(&self) -> &ca::Registry<Graph<N>> {
        self.ca_registry
    }

    /// Access the builtins provider.
    pub fn builtins(&self) -> &dyn Builtins<Node = N> {
        self.builtins
    }
}

impl<N: 'static + Node + Send + Sync> RegistryRef<'_, N> {
    /// Look up a node by content address.
    ///
    /// Checks commit graphs in the registry first, then falls back to builtins.
    pub fn node(&self, ca: &ca::ContentAddr) -> Option<&dyn Node> {
        let commit_ca = ca::CommitAddr::from(*ca);
        if let Some(graph) = self.ca_registry.commit_graph_ref(&commit_ca) {
            return Some(graph as &dyn Node);
        }
        self.builtins.instance(ca).map(|n| n as &dyn Node)
    }

    /// Create a node of the given type name.
    ///
    /// Checks registry names first (creating a [`crate::node::NamedRef`]),
    /// then falls back to builtins.
    pub fn create_node(&self, node_type: &str) -> Option<N>
    where
        N: From<crate::node::NamedRef>,
    {
        self.ca_registry
            .names()
            .get(node_type)
            .map(|commit_ca| {
                let ref_ = gantz_core::node::Ref::new((*commit_ca).into());
                let named = crate::node::NamedRef::new(node_type.to_string(), ref_);
                N::from(named)
            })
            .or_else(|| self.builtins.create(node_type))
    }
}

// ---------------------------------------------------------------------------
// Trait implementations
// ---------------------------------------------------------------------------

impl<N: 'static + Node + Send + Sync> NodeTypeRegistry for RegistryRef<'_, N> {
    fn node_types(&self) -> Vec<&str> {
        let mut types = vec![];
        types.extend(self.builtins.names());
        types.extend(self.ca_registry.names().keys().map(|s| &s[..]));
        types.sort();
        types
    }
}

impl<N: 'static + Node + Send + Sync> GraphRegistry for RegistryRef<'_, N> {
    fn commits(&self) -> Vec<(&ca::CommitAddr, &ca::Commit)> {
        let mut commits: Vec<_> = self.ca_registry.commits().iter().collect();
        commits.sort_by(|(_, a), (_, b)| b.timestamp.cmp(&a.timestamp));
        commits
    }

    fn names(&self) -> &BTreeMap<String, ca::CommitAddr> {
        self.ca_registry.names()
    }
}

impl<N: 'static + Node + Send + Sync> NameRegistry for RegistryRef<'_, N> {
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

impl<N: 'static + Node + Send + Sync> FnNodeNames for RegistryRef<'_, N> {
    fn fn_node_names(&self) -> Vec<String> {
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

impl<N: 'static + Node + Send + Sync> Registry for RegistryRef<'_, N> {
    fn node(&self, ca: &ca::ContentAddr) -> Option<&dyn Node> {
        RegistryRef::node(self, ca)
    }
}
