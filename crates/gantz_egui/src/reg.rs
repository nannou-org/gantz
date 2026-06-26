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
use petgraph::visit::{IntoNodeReferences, NodeRef};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

/// Registry reference providing unified node access.
///
/// Combines access to a content-addressed registry (for user-defined graphs)
/// with builtin nodes, implementing all the registry traits required by
/// gantz_egui widgets.
pub struct RegistryRef<'a, N: 'static + Send + Sync> {
    ca_registry: &'a ca::Registry<Graph<N>>,
    builtins: &'a dyn Builtins<Node = N>,
    demos: &'a HashMap<String, String>,
}

impl<'a, N: 'static + Send + Sync> RegistryRef<'a, N> {
    /// Construct from a CA registry, builtins provider and demo associations.
    pub fn new(
        ca_registry: &'a ca::Registry<Graph<N>>,
        builtins: &'a dyn Builtins<Node = N>,
        demos: &'a HashMap<String, String>,
    ) -> Self {
        Self {
            ca_registry,
            builtins,
            demos,
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
        // The reserved nested-graph entry replaces the old `graph` builtin.
        let mut types = vec![crate::widget::gantz::NESTED_GRAPH_TYPE];
        types.extend(self.builtins.names());
        // Nested graphs are hidden from the root graph-select list, so don't
        // offer them as creatable node types either.
        types.extend(
            self.ca_registry
                .names()
                .keys()
                .filter(|n| !n.contains(crate::node::NESTED_SEP))
                .map(|s| &s[..]),
        );
        types.sort();
        types.dedup();
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

impl<N: 'static + Node + crate::NodeUi + crate::sync::AsNamedRef + Send + Sync> Registry
    for RegistryRef<'_, N>
{
    fn node(&self, ca: &ca::ContentAddr) -> Option<&dyn Node> {
        RegistryRef::node(self, ca)
    }

    fn would_ref_cycle(&self, target: &str, editing: &str) -> bool {
        crate::cycle::would_cycle(self.ca_registry, target, editing)
    }

    fn demo_graph(&self, name: &str) -> Option<&str> {
        // User-graph associations (from Export) are keyed by name; builtins
        // expose their own demo by builtin name.
        self.demos
            .get(name)
            .map(String::as_str)
            .or_else(|| self.builtins.demo_graph(name))
    }

    fn socket_doc(
        &self,
        ca: &ca::ContentAddr,
        kind: crate::SocketKind,
        ix: usize,
    ) -> Option<crate::SocketDoc> {
        // Resolve the referenced graph and read the ix-th inlet/outlet marker's
        // own doc (docs live on the `Inlet`/`Outlet` nodes).
        let commit_ca = ca::CommitAddr::from(*ca);
        let graph = self.ca_registry.commit_graph_ref(&commit_ca)?;
        let get_node = |c: &ca::ContentAddr| self.node(c);
        let meta_ctx = node::MetaCtx::new(&get_node);
        let node_ref = graph
            .node_references()
            .filter(|n| match kind {
                crate::SocketKind::Input => n.weight().inlet(meta_ctx),
                crate::SocketKind::Output => n.weight().outlet(meta_ctx),
            })
            .nth(ix)?;
        let marker = node_ref.weight();
        // An inlet exposes its doc on its output socket; an outlet on its input.
        let marker_kind = match kind {
            crate::SocketKind::Input => crate::SocketKind::Output,
            crate::SocketKind::Output => crate::SocketKind::Input,
        };
        marker.socket_doc(self, marker_kind, 0)
    }

    fn command_info(&self, name: &str) -> crate::CommandInfo {
        use crate::SocketKind;
        let mut info = crate::CommandInfo {
            name: name.to_string(),
            description: self.node_description(name),
            ..Default::default()
        };

        // The reserved nested-graph entry mints a fresh, empty child graph.
        if name == crate::widget::gantz::NESTED_GRAPH_TYPE {
            return info;
        }

        let get_node = |c: &ca::ContentAddr| self.node(c);
        let meta_ctx = node::MetaCtx::new(&get_node);
        // Collect `n` socket docs, defaulting a missing doc to a bare "any".
        let collect =
            |n: usize,
             kind: SocketKind,
             f: &dyn Fn(SocketKind, usize) -> Option<crate::SocketDoc>| {
                (0..n)
                    .map(|ix| f(kind, ix).unwrap_or_else(|| crate::SocketDoc::ty("any")))
                    .collect::<Vec<_>>()
            };

        if let Some(commit_ca) = self.ca_registry.names().get(name) {
            // A named graph: socket docs resolved from the referenced graph's
            // inlet/outlet markers.
            if let Some(graph) = self.ca_registry.commit_graph_ref(commit_ca) {
                let ca: ca::ContentAddr = (*commit_ca).into();
                let socket =
                    |kind: SocketKind, ix: usize| Registry::socket_doc(self, &ca, kind, ix);
                info.inputs = collect(graph.n_inputs(meta_ctx), SocketKind::Input, &socket);
                info.outputs = collect(graph.n_outputs(meta_ctx), SocketKind::Output, &socket);
            }
        } else if let Some(builtin) = self.builtins.create(name) {
            // A builtin: introspect a fresh instance.
            let socket = |kind: SocketKind, ix: usize| builtin.socket_doc(self, kind, ix);
            info.inputs = collect(builtin.n_inputs(meta_ctx), SocketKind::Input, &socket);
            info.outputs = collect(builtin.n_outputs(meta_ctx), SocketKind::Output, &socket);
        }

        info
    }

    fn graph_description(&self, name: &str) -> Option<&str> {
        self.ca_registry.description(name)
    }

    fn node_description(&self, name: &str) -> Option<Cow<'static, str>> {
        if name == crate::widget::gantz::NESTED_GRAPH_TYPE {
            return Some(Cow::Borrowed(
                "Create a new nested graph. Its inlets and outlets become this node's sockets.",
            ));
        }
        if self.ca_registry.names().contains_key(name) {
            return self
                .ca_registry
                .description(name)
                .map(|s| Cow::Owned(s.to_string()));
        }
        self.builtins
            .create(name)
            .and_then(|n| n.description())
            .map(Cow::Borrowed)
    }
}
