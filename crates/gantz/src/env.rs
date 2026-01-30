use crate::graph::Graph;
use bevy_gantz::{BuiltinNodes, Registry, Views};
use gantz_ca as ca;
use std::collections::BTreeMap;

/// Reference-based environment for VM operations.
///
/// This is a concrete type (not generic over N) to avoid trait bound cycles.
/// Constructed on-demand from borrowed Bevy resources.
pub struct Environment<'a> {
    /// The registry of all graphs, commits and names.
    pub registry: &'a ca::Registry<Graph>,
    /// Views (layout + camera) for all known commits.
    pub views: &'a Views,
    /// Builtins (primitive nodes).
    pub builtins: &'a dyn bevy_gantz::Builtins<Node = Box<dyn crate::node::Node>>,
}

impl<'a> Environment<'a> {
    /// Create a new environment from borrowed resources.
    pub fn new(
        registry: &'a Registry<Box<dyn crate::node::Node>>,
        views: &'a Views,
        builtins: &'a BuiltinNodes<Box<dyn crate::node::Node>>,
    ) -> Self {
        Self {
            registry: &registry.0,
            views,
            builtins: &*builtins.0,
        }
    }
}

// Provide the `NodeTypeRegistry` implementation required by `gantz_egui`.
impl gantz_egui::widget::gantz::NodeTypeRegistry for Environment<'_> {
    type Node = Box<dyn crate::node::Node>;

    fn node_types(&self) -> impl Iterator<Item = &str> {
        let mut types = vec![];
        types.extend(self.builtins.names());
        types.extend(self.registry.names().keys().map(|s| &s[..]));
        types.sort();
        types.into_iter()
    }

    fn new_node(&self, node_type: &str) -> Option<Self::Node> {
        self.registry
            .names()
            .get(node_type)
            .map(|commit_ca| {
                // Store CommitAddr directly (converted to ContentAddr).
                let ref_ = gantz_core::node::Ref::new((*commit_ca).into());
                let named = gantz_egui::node::NamedRef::new(node_type.to_string(), ref_);
                Box::new(named) as Box<_>
            })
            .or_else(|| self.builtins.create(node_type))
    }
}

// Provide the `NodeRegistry` implementation required by `gantz_core::node::Ref`.
impl gantz_core::node::ref_::NodeRegistry for Environment<'_> {
    type Node = dyn gantz_core::Node<Self>;
    fn node(&self, ca: &ca::ContentAddr) -> Option<&Self::Node> {
        // Try commit lookup (for graph refs stored as CommitAddr).
        let commit_ca = ca::CommitAddr::from(*ca);
        if let Some(graph) = self.registry.commit_graph_ref(&commit_ca) {
            return Some(graph as &dyn gantz_core::Node<Self>);
        }
        // Fall back to builtin lookup.
        self.builtins
            .instance(ca)
            .map(|n| &**n as &dyn gantz_core::Node<Self>)
    }
}

// Provide the `GraphRegistry` implementation required by the `GraphSelect` widget.
impl gantz_egui::widget::graph_select::GraphRegistry for Environment<'_> {
    fn commits(&self) -> Vec<(&ca::CommitAddr, &ca::Commit)> {
        // Sort commits by newest to oldest.
        let mut commits: Vec<_> = self.registry.commits().iter().collect();
        commits.sort_by(|(_, a), (_, b)| b.timestamp.cmp(&a.timestamp));
        commits
    }

    fn names(&self) -> &BTreeMap<String, ca::CommitAddr> {
        self.registry.names()
    }
}

// Provide the `NameRegistry` implementation required by `gantz_egui::node::NamedRef`.
impl gantz_egui::node::NameRegistry for Environment<'_> {
    fn name_ca(&self, name: &str) -> Option<ca::ContentAddr> {
        // Check registry names first (graphs shadow builtins).
        // Return CommitAddr (as ContentAddr) for graph nodes.
        if let Some(commit_ca) = self.registry.names().get(name) {
            return Some((*commit_ca).into());
        }
        // Then check builtin names.
        self.builtins.content_addr(name)
    }
}

// Provide the `FnNodeNames` implementation required by `Fn<NamedRef>` UI.
impl gantz_egui::node::FnNodeNames for Environment<'_> {
    fn fn_node_names(&self) -> Vec<String> {
        use gantz_core::node::ref_::NodeRegistry;
        use gantz_egui::node::NameRegistry;

        // Collect all names (builtins + registry names).
        let builtin_names = self
            .builtins
            .names()
            .into_iter()
            .filter_map(|name| self.builtins.content_addr(name).map(|_| name.to_string()));
        let registry_names = self.registry.names().keys().cloned();
        let all_names = builtin_names.chain(registry_names);

        // Filter to Fn-compatible nodes (stateless, branchless, 1 output).
        let mut names: Vec<_> = all_names
            .filter(|name| {
                self.name_ca(name)
                    .and_then(|ca| self.node(&ca))
                    .map(|n| {
                        !n.stateful(self) && n.branches(self).is_empty() && n.n_outputs(self) == 1
                    })
                    .unwrap_or(false)
            })
            .collect();

        names.sort();
        names
    }
}
