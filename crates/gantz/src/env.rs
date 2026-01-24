use crate::{
    graph::{Graph, GraphNode},
    node::Node,
};
use bevy::ecs::resource::Resource;
use gantz_ca as ca;
use gantz_core::node;
use std::{collections::BTreeMap, collections::HashMap};

/// View state (layout + camera) for a graph and all its nested subgraphs, keyed by path.
pub type GraphViews = HashMap<Vec<node::Id>, egui_graph::View>;

/// All graph views keyed by commit address.
pub type Views = HashMap<ca::CommitAddr, GraphViews>;

/// The type used to track mappings between node names, content addresses and
/// graphs. Also provides access to the node registry. This can be thought of as
/// a shared immutable input to all nodes.
#[derive(Resource)]
pub struct Environment {
    /// Constructors for all primitive nodes.
    pub primitives: Primitives,
    /// Instantiated primitives for lookup by content address.
    pub primitive_instances: PrimitiveInstances,
    /// Mapping from primitive content addresses to their names.
    pub primitive_names: PrimitiveNames,
    /// The registry of all nodes composed from other nodes.
    pub registry: Registry,
    /// Views (layout + camera) for all known commits, keyed by commit address.
    pub views: Views,
}

/// Instantiated primitive nodes keyed by their content address.
pub type PrimitiveInstances = HashMap<ca::ContentAddr, Box<dyn Node>>;

/// Mapping from primitive content addresses to their names.
pub type PrimitiveNames = HashMap<ca::ContentAddr, String>;

/// The registry for all graphs, commits and commit names.
pub type Registry = ca::Registry<Graph>;

/// Constructors for all primitive nodes.
type Primitives = BTreeMap<String, Box<dyn Send + Sync + Fn() -> Box<dyn Node>>>;

// Provide the `NodeTypeRegistry` implementation required by `gantz_egui`.
impl gantz_egui::widget::gantz::NodeTypeRegistry for Environment {
    type Node = Box<dyn Node>;

    fn node_types(&self) -> impl Iterator<Item = &str> {
        let mut types = vec![];
        types.extend(self.primitives.keys().map(|s| &s[..]));
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
            .or_else(|| self.primitives.get(node_type).map(|f| (f)()))
    }
}

// Provide the `NodeRegistry` implementation required by `gantz_core::node::Ref`.
impl gantz_core::node::ref_::NodeRegistry for Environment {
    type Node = dyn gantz_core::Node<Self>;
    fn node(&self, ca: &gantz_ca::ContentAddr) -> Option<&Self::Node> {
        // Try commit lookup (for graph refs stored as CommitAddr).
        let commit_ca = gantz_ca::CommitAddr::from(*ca);
        if let Some(graph) = self.registry.commit_graph_ref(&commit_ca) {
            return Some(graph as &dyn gantz_core::Node<Self>);
        }
        // Fall back to primitive lookup.
        self.primitive_instances
            .get(ca)
            .map(|n| &**n as &dyn gantz_core::Node<Self>)
    }
}

// Provide the `GraphRegistry` implementation required by the `GraphSelect` widget.
impl gantz_egui::widget::graph_select::GraphRegistry for Environment {
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
impl gantz_egui::node::NameRegistry for Environment {
    fn name_ca(&self, name: &str) -> Option<ca::ContentAddr> {
        // Check registry names first (graphs shadow primitives).
        // Return CommitAddr (as ContentAddr) for graph nodes.
        if let Some(commit_ca) = self.registry.names().get(name) {
            return Some((*commit_ca).into());
        }
        // Then check primitive names.
        self.primitive_names
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(content_addr, _)| *content_addr)
    }
}

// Provide the `FnNodeNames` implementation required by `Fn<NamedRef>` UI.
impl gantz_egui::node::FnNodeNames for Environment {
    fn fn_node_names(&self) -> Vec<String> {
        use gantz_core::node::ref_::NodeRegistry;
        use gantz_egui::node::NameRegistry;

        // Collect all names (primitives + registry names).
        let all_names = self
            .primitive_names
            .values()
            .chain(self.registry.names().keys());

        // Filter to Fn-compatible nodes (stateless, branchless, 1 output).
        // TODO: Graph::branches impl needs fixing (follow-up PR).
        let mut names: Vec<_> = all_names
            .filter(|name| {
                self.name_ca(name)
                    .and_then(|ca| self.node(&ca))
                    .map(|n| {
                        !n.stateful(self) && n.branches(self).is_empty() && n.n_outputs(self) == 1
                    })
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        names.sort();
        names
    }
}

/// The set of all known node types accessible to gantz.
pub fn primitives() -> Primitives {
    let mut p = Primitives::default();
    register_primitive(&mut p, "add", || {
        Box::new(gantz_std::ops::Add::default()) as Box<_>
    });
    register_primitive(&mut p, "apply", || {
        Box::new(gantz_core::node::Apply::default()) as Box<_>
    });
    register_primitive(&mut p, "bang", || {
        Box::new(gantz_std::Bang::default()) as Box<_>
    });
    register_primitive(&mut p, "comment", || {
        Box::new(gantz_egui::node::Comment::default()) as Box<_>
    });
    register_primitive(&mut p, "expr", || {
        Box::new(gantz_core::node::Expr::new("()").unwrap()) as Box<_>
    });
    // Compute Identity's CA for the default Fn<NamedRef>.
    let identity_ca = ca::content_addr(&gantz_core::node::Identity);
    register_primitive(&mut p, "fn", move || {
        let named_ref = gantz_egui::node::NamedRef::new(
            gantz_core::node::IDENTITY_NAME.to_string(),
            gantz_core::node::Ref::new(identity_ca),
        );
        Box::new(gantz_core::node::Fn::new(named_ref)) as Box<_>
    });
    register_primitive(&mut p, "graph", || Box::new(GraphNode::default()) as Box<_>);
    register_primitive(&mut p, gantz_core::node::IDENTITY_NAME, || {
        Box::new(gantz_core::node::Identity::default()) as Box<_>
    });
    register_primitive(&mut p, "inlet", || {
        Box::new(gantz_core::node::graph::Inlet::default()) as Box<_>
    });
    register_primitive(&mut p, "inspect", || {
        Box::new(gantz_egui::node::Inspect::default()) as Box<_>
    });
    register_primitive(&mut p, "log", || {
        Box::new(gantz_std::Log::default()) as Box<_>
    });
    register_primitive(&mut p, "number", || {
        Box::new(gantz_std::Number::default()) as Box<_>
    });
    register_primitive(&mut p, "outlet", || {
        Box::new(gantz_core::node::graph::Outlet::default()) as Box<_>
    });
    p
}

fn register_primitive(
    primitives: &mut Primitives,
    name: impl Into<String>,
    new: impl 'static + Send + Sync + Fn() -> Box<dyn Node>,
) -> Option<Box<dyn Send + Sync + Fn() -> Box<dyn Node>>> {
    primitives.insert(name.into(), Box::new(new) as Box<_>)
}

/// Build the primitive instances and names maps from the primitives constructors.
pub fn primitive_instances_and_names(
    primitives: &Primitives,
) -> (PrimitiveInstances, PrimitiveNames) {
    let mut instances = PrimitiveInstances::default();
    let mut names = PrimitiveNames::default();
    for (name, ctor) in primitives.iter() {
        let node = ctor();
        let content_addr = ca::content_addr(&node);
        instances.insert(content_addr, node);
        names.insert(content_addr, name.clone());
    }
    (instances, names)
}

/// Create a timestamp for a commit.
pub fn timestamp() -> std::time::Duration {
    let now = web_time::SystemTime::now();
    now.duration_since(web_time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
}
