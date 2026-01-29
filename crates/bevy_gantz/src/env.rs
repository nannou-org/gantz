//! Environment reference type for VM operations.

use crate::builtin::Builtins;
use crate::reg::Registry;
use crate::view::Views;
use gantz_ca as ca;
use gantz_core::node;
use std::collections::BTreeMap;

/// Environment constructed from borrowed resources.
///
/// Used for VM operations that need trait implementations like `NodeRegistry`.
/// Constructed on-demand from separate Bevy resources.
pub struct Environment<'a, N> {
    pub registry: &'a ca::Registry<node::graph::Graph<N>>,
    pub views: &'a Views,
    pub builtins: &'a dyn Builtins<Node = N>,
}

impl<'a, N> Environment<'a, N> {
    /// Create a new environment from borrowed resources.
    pub fn new(
        registry: &'a Registry<N>,
        views: &'a Views,
        builtins: &'a dyn Builtins<Node = N>,
    ) -> Self {
        Self {
            registry: &registry.0,
            views,
            builtins,
        }
    }
}

impl<'a, N> gantz_egui::widget::graph_select::GraphRegistry for Environment<'a, N> {
    fn commits(&self) -> Vec<(&ca::CommitAddr, &ca::Commit)> {
        let mut commits: Vec<_> = self.registry.commits().iter().collect();
        commits.sort_by(|(_, a), (_, b)| b.timestamp.cmp(&a.timestamp));
        commits
    }

    fn names(&self) -> &BTreeMap<String, ca::CommitAddr> {
        self.registry.names()
    }
}

impl<'a, N: Send + Sync + 'static> gantz_egui::node::NameRegistry for Environment<'a, N> {
    fn name_ca(&self, name: &str) -> Option<ca::ContentAddr> {
        // Check registry names first (graphs shadow builtins).
        if let Some(commit_ca) = self.registry.names().get(name) {
            return Some((*commit_ca).into());
        }
        // Then check builtin names.
        self.builtins.content_addr(name)
    }
}

impl<'a, N: Send + Sync + 'static> gantz_egui::widget::gantz::NodeTypeRegistry
    for Environment<'a, N>
{
    type Node = N;

    fn node_types(&self) -> impl Iterator<Item = &str> {
        let mut types = vec![];
        types.extend(self.builtins.names());
        types.extend(self.registry.names().keys().map(|s| &s[..]));
        types.sort();
        types.into_iter()
    }

    fn new_node(&self, node_type: &str) -> Option<Self::Node> {
        // Try registry first, then builtins.
        // For registry nodes, we'd need to create a Ref node, but that requires
        // knowing the concrete node type. For now, try builtins.
        self.builtins.create(node_type)
    }
}
