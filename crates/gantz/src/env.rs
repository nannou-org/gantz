use crate::Graph;
use bevy_gantz::{BuiltinNodes, Registry};
use gantz_ca as ca;
use gantz_core::node::{self, Node as CoreNode};
use std::collections::BTreeMap;

/// Reference-based environment for VM operations.
///
/// This is a concrete type (not generic over N) to avoid trait bound cycles.
/// Constructed on-demand from borrowed Bevy resources.
pub struct Environment<'a> {
    /// The registry of all graphs, commits and names.
    pub registry: &'a ca::Registry<Graph>,
    /// Builtins (primitive nodes).
    pub builtins: &'a dyn bevy_gantz::Builtins<Node = Box<dyn crate::node::Node>>,
}

impl<'a> Environment<'a> {
    /// Create a new environment from borrowed resources.
    pub fn new(
        registry: &'a Registry<Box<dyn crate::node::Node>>,
        builtins: &'a BuiltinNodes<Box<dyn crate::node::Node>>,
    ) -> Self {
        Self {
            registry: &registry.0,
            builtins: &*builtins.0,
        }
    }

    /// Look up a node by content address.
    ///
    /// Returns the node if found in either the registry (as a commit graph) or builtins.
    pub fn node(&self, ca: &ca::ContentAddr) -> Option<&dyn CoreNode> {
        // Try commit lookup (for graph refs stored as CommitAddr).
        let commit_ca = ca::CommitAddr::from(*ca);
        if let Some(graph) = self.registry.commit_graph_ref(&commit_ca) {
            return Some(graph as &dyn CoreNode);
        }
        // Fall back to builtin lookup.
        self.builtins.instance(ca).map(|n| &**n as &dyn CoreNode)
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
}

/// Create a node of the given type name.
///
/// This is used to handle [`gantz_egui::Cmd::CreateNode`] commands.
pub fn create_node(
    registry: &ca::Registry<crate::Graph>,
    builtins: &dyn bevy_gantz::Builtins<Node = Box<dyn crate::node::Node>>,
    node_type: &str,
) -> Option<Box<dyn crate::node::Node>> {
    registry
        .names()
        .get(node_type)
        .map(|commit_ca| {
            // Store CommitAddr directly (converted to ContentAddr).
            let ref_ = gantz_core::node::Ref::new((*commit_ca).into());
            let named = gantz_egui::node::NamedRef::new(node_type.to_string(), ref_);
            Box::new(named) as Box<dyn crate::node::Node>
        })
        .or_else(|| builtins.create(node_type))
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

    fn node_exists(&self, ca: &ca::ContentAddr) -> bool {
        self.node(ca).is_some()
    }
}

// Provide the `FnNodeNames` implementation required by `Fn<NamedRef>` UI.
impl gantz_egui::node::FnNodeNames for Environment<'_> {
    fn fn_node_names(&self) -> Vec<String> {
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
        // Use a lookup closure that delegates to self.node().
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

// Provide the `Registry` implementation required by the Gantz widget.
impl gantz_egui::Registry for Environment<'_> {
    fn node(&self, ca: &ca::ContentAddr) -> Option<&dyn CoreNode> {
        Environment::node(self, ca)
    }
}
