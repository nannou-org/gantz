//! The concrete node environment the gantz_egui widgets and nodes read.
//!
//! Provides [`Env`] - a borrowed view combining the content-addressed
//! registry, the builtin palette, the app's node codec and the reified
//! caches serving both as typed nodes.

use crate::node::{NodeCodec, UiBuiltins};
use gantz_ca as ca;
use gantz_core::data::ReifiedGraphs;
use gantz_core::node;
use gantz_core::{Builtins, Node};
use petgraph::visit::{IntoNodeReferences, NodeRef};
use std::borrow::Cow;
use std::collections::BTreeMap;

/// The environment the gantz_egui widgets and nodes read: every shared
/// immutable input to a GUI pass.
///
/// The data side is concrete: [`registry`][Self::registry] exposes the
/// content-addressed [`gantz_ca::Registry`] directly, and widgets read
/// commits, heads and sections from it without indirection. The typed side
/// combines the reified-graph cache serving the registry's graphs as typed
/// nodes ([`graphs`][Self::graphs]), the builtin palette as data
/// ([`builtins`][Self::builtins]) plus one reified instance per builtin
/// ([`instances`][Self::instances]), and the app's value-level node codec
/// ([`codec`][Self::codec]).
#[derive(Clone, Copy)]
pub struct Env<'a> {
    /// The content-addressed data registry.
    pub registry: &'a ca::Registry,
    /// The builtin palette as data.
    pub builtins: &'a Builtins,
    /// The app's value-level node codec.
    pub codec: &'a NodeCodec,
    /// The registry's graphs reified through the codec.
    pub graphs: &'a ReifiedGraphs<crate::node::DynNode>,
    /// One reified instance per builtin, keyed by erased content address.
    pub instances: &'a UiBuiltins,
}

/// Named heads (branches), ordered by name.
pub type Names = BTreeMap<ca::Name, ca::CommitAddr>;

impl Env<'_> {
    /// Look up a node by content address.
    ///
    /// Checks reified registry graphs first (a graph in the registry IS a
    /// node), then falls back to builtins.
    pub fn node(&self, ca: &ca::ContentAddr) -> Option<&dyn Node> {
        let graph_ca = ca::GraphAddr::from(*ca);
        if let Some(graph) = self.graphs.get(&graph_ca) {
            return Some(graph as &dyn Node);
        }
        self.instances.get(ca).map(|n| &**n as &dyn Node)
    }

    /// Create a node of the given type name, in its stored data form.
    ///
    /// Checks registry names first (creating a [`crate::node::NamedRef`]
    /// pinning the name's head graph), then falls back to the builtin's
    /// stored data.
    pub fn create_node(&self, node_type: &str) -> Option<ca::NodeData> {
        let name: ca::Name = node_type.parse().expect("infallible");
        head_graph_addr(self.registry, &name)
            .and_then(|graph_addr| {
                let ref_ = gantz_core::node::Ref::new(graph_addr.into());
                let named = crate::node::NamedRef::new(name.clone(), ref_);
                match gantz_core::data::erase_node_typed(&named) {
                    Ok(node_data) => Some(node_data),
                    Err(e) => {
                        log::error!("failed to erase a `NamedRef` to `{name}`: {e}");
                        None
                    }
                }
            })
            .or_else(|| self.builtins.node_data(node_type).cloned())
    }

    /// Returns the current content address for the given name, if it exists.
    ///
    /// Required by [`crate::node::NamedRef`] to check whether a referenced
    /// graph still exists and to display up-to-date status.
    pub fn name_ca(&self, name: &str) -> Option<ca::ContentAddr> {
        let parsed: ca::Name = name.parse().expect("infallible");
        head_graph_addr(self.registry, &parsed)
            .map(Into::into)
            .or_else(|| self.builtins.content_addr(name))
    }

    /// Returns true if a node with the given content address exists in the
    /// environment.
    pub fn node_exists(&self, ca: &ca::ContentAddr) -> bool {
        self.node(ca).is_some()
    }

    /// Names of nodes that can be used with `Fn`.
    /// Filters to: stateless, branchless, single-output nodes.
    ///
    /// Required by [`crate::node::FnNamedRef`]'s UI dropdown.
    pub fn fn_node_names(&self) -> Vec<String> {
        let builtin_names = self.builtins.names().map(str::to_string);
        let registry_names = self.registry.heads().map(|(name, _)| name.to_string());
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

    /// The unique name of each node available.
    ///
    /// Provides the list of node type names available for creation via the
    /// node palette. Actual node creation is handled via [`crate::CreateNode`].
    pub fn node_types(&self) -> Vec<&'_ str> {
        // The reserved nested-graph entry replaces the old `graph` builtin.
        let mut types = vec![crate::widget::gantz::NESTED_GRAPH_TYPE];
        for name in self.builtins.names() {
            types.push(name);
        }
        // Nested graphs are hidden from the root graph-select list, so don't
        // offer them as creatable node types either. A root name is its
        // single segment.
        types.extend(
            self.registry
                .heads()
                .filter(|(name, _)| !name.is_nested())
                .map(|(name, _)| name.segments()[0].as_str()),
        );
        types.sort();
        types.dedup();
        types
    }

    /// The formatted keyboard shortcut for the node palette entry `node_type`,
    /// if any.
    pub fn command_formatted_kb_shortcut(
        &self,
        _ctx: &egui::Context,
        _node_type: &str,
    ) -> Option<String> {
        None
    }

    /// Whether referencing the graph named `target` from the graph named
    /// `editing` would create a reference cycle (see
    /// [`crate::cycle::would_cycle`]).
    ///
    /// Used by the node palette to hide node types that would form a cycle.
    pub fn would_ref_cycle(&self, target: &str, editing: &str) -> bool {
        let target: ca::Name = target.parse().expect("infallible");
        let editing: ca::Name = editing.parse().expect("infallible");
        crate::cycle::would_cycle(self.registry, &target, &editing)
    }

    /// Get the demo graph name associated with the graph of the given name.
    pub fn demo_graph(&self, name: &str) -> Option<String> {
        // Demo associations live in the registry's demo section, keyed by name.
        let parsed: ca::Name = name.parse().expect("infallible");
        crate::section::demo(self.registry, &parsed)
    }

    /// The [`crate::SocketDoc`] for the given socket of the graph referenced
    /// by `ca`.
    ///
    /// Lets a referencing node (e.g. [`crate::node::NamedRef`]) surface the
    /// referenced graph's inlet/outlet docs: the referenced graph is resolved
    /// and the relevant `Inlet`/`Outlet` marker's own doc read, so docs live
    /// on the nodes rather than in side-metadata.
    pub fn socket_doc(
        &self,
        ca: &ca::ContentAddr,
        kind: crate::SocketKind,
        ix: usize,
    ) -> Option<crate::SocketDoc> {
        // Resolve the referenced graph and read the ix-th inlet/outlet marker's
        // own doc (docs live on the `Inlet`/`Outlet` nodes).
        let graph_ca = ca::GraphAddr::from(*ca);
        let graph = self.graphs.get(&graph_ca)?;
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
        crate::NodeUi::socket_doc(&**marker, self, marker_kind, 0)
    }

    /// Display-ready documentation for the creatable node type named `name`.
    ///
    /// Combines the node's description with its derived input/output
    /// [`crate::SocketDoc`]s. Shown beside the highlighted entry in the node
    /// palette and as hover documentation in the "Graphs" select widget.
    pub fn command_info(&self, name: &str) -> crate::CommandInfo {
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

        let parsed: ca::Name = name.parse().expect("infallible");
        if let Some(graph_addr) = head_graph_addr(self.registry, &parsed) {
            // A named graph: socket docs resolved from the referenced graph's
            // inlet/outlet markers.
            if let Some(graph) = self.graphs.get(&graph_addr) {
                let ca: ca::ContentAddr = graph_addr.into();
                let socket = |kind: SocketKind, ix: usize| self.socket_doc(&ca, kind, ix);
                info.inputs = collect(graph.n_inputs(meta_ctx), SocketKind::Input, &socket);
                info.outputs = collect(graph.n_outputs(meta_ctx), SocketKind::Output, &socket);
            }
        } else if let Some(builtin) = self
            .builtins
            .content_addr(name)
            .and_then(|ca| self.instances.get(&ca))
        {
            // A builtin: introspect its stored instance.
            let socket = |kind: SocketKind, ix: usize| builtin.socket_doc(self, kind, ix);
            info.inputs = collect(builtin.n_inputs(meta_ctx), SocketKind::Input, &socket);
            info.outputs = collect(builtin.n_outputs(meta_ctx), SocketKind::Output, &socket);
        }

        info
    }

    /// Dry-run the merge of the branch named `source` into `ours` under the
    /// given conflict resolutions (see [`crate::merge::merge_preview`]), for
    /// hover previews in the merge row.
    pub fn merge_preview(
        &self,
        ours: &ca::Head,
        source: &str,
        resolutions: ca::Resolutions,
    ) -> Option<crate::merge::MergePreview> {
        crate::merge::merge_preview(self.registry, ours, source, resolutions)
    }

    /// A concise description of the creatable node type `name`, for inline
    /// display in the node palette. Lighter than
    /// [`command_info`](Self::command_info) (it derives no input/output docs).
    pub fn node_description(&self, name: &str) -> Option<Cow<'static, str>> {
        if name == crate::widget::gantz::NESTED_GRAPH_TYPE {
            return Some(Cow::Borrowed(
                "Create a new nested graph. Its inlets and outlets become this node's sockets.",
            ));
        }
        let parsed: ca::Name = name.parse().expect("infallible");
        if self.registry.head(&parsed).is_some() {
            return crate::section::description(self.registry, &parsed).map(Cow::Owned);
        }
        self.builtins
            .content_addr(name)
            .and_then(|ca| self.instances.get(&ca))
            .and_then(|n| n.description())
            .map(Cow::Borrowed)
    }
}

/// The graph address at the tip of the named line of history: the address a
/// [`Ref`](gantz_core::node::Ref) to the name should pin.
pub fn head_graph_addr(reg: &ca::Registry, name: &ca::Name) -> Option<ca::GraphAddr> {
    let head_ca = reg.head(name)?;
    reg.commits().get(&head_ca).map(|commit| commit.graph)
}

/// All name -> head commit pairs, in name order.
pub fn names(reg: &ca::Registry) -> Vec<(ca::Name, ca::CommitAddr)> {
    reg.heads().map(|(name, ca)| (name.clone(), ca)).collect()
}
