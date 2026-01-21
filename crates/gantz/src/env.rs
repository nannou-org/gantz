use crate::{
    graph::{Graph, GraphNode},
    node::Node,
};
use bevy::ecs::resource::Resource;
use gantz_ca as ca;
use gantz_core::node;
use petgraph::visit::{IntoNodeReferences, NodeRef};
use std::{any::Any, collections::BTreeMap, collections::HashMap};

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
    /// The registry of all nodes composed from other nodes.
    pub registry: Registry,
    /// Views (layout + camera) for all known commits, keyed by commit address.
    pub views: Views,
}

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
            .and_then(|commit_ca| {
                let graph_ca = self.registry.commits().get(commit_ca)?.graph;
                let named = gantz_egui::node::NamedGraph::new(node_type.to_string(), graph_ca);
                Some(Box::new(named) as Box<_>)
            })
            .or_else(|| self.primitives.get(node_type).map(|f| (f)()))
    }
}

// Provide the `GraphRegistry` implementation required by `gantz_egui`.
impl gantz_egui::node::graph::GraphRegistry for Environment {
    type Node = Box<dyn Node>;
    fn graph(&self, ca: ca::GraphAddr) -> Option<&gantz_core::node::graph::Graph<Self::Node>> {
        self.registry.graphs().get(&ca)
    }
}

// Provide the `GraphRegistry` implementation required by `gantz_core::node::Fn`.
impl gantz_core::node::fn_::GraphRegistry for Environment {
    type Node = Box<dyn Node>;
    fn graph(&self, ca: ca::GraphAddr) -> Option<&gantz_core::node::graph::Graph<Self::Node>> {
        self.registry.graphs().get(&ca)
    }
    fn new_primitive(&self, name: &str) -> Option<Self::Node> {
        self.primitives.get(name).map(|f| (f)())
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
    register_primitive(&mut p, "fn", || {
        Box::new(gantz_core::node::Fn::default()) as Box<_>
    });
    register_primitive(&mut p, "graph", || Box::new(GraphNode::default()) as Box<_>);
    register_primitive(&mut p, gantz_core::node::IDENTITY_NAME, || {
        Box::new(gantz_core::node::Identity::default()) as Box<_>
    });
    register_primitive(&mut p, "inlet", || {
        Box::new(gantz_core::node::graph::Inlet::default()) as Box<_>
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

/// Create a timestamp for a commit.
pub fn timestamp() -> std::time::Duration {
    let now = web_time::SystemTime::now();
    now.duration_since(web_time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
}

/// Whether or not the graph contains a subgraph with the given CA.
pub fn graph_contains(g: &Graph, ca: &ca::GraphAddr) -> bool {
    g.node_references().any(|n_ref| {
        let node = n_ref.weight();
        ((&**node) as &dyn Any)
            .downcast_ref::<GraphNode>()
            .map(|graph| {
                let graph_ca = ca::graph_addr(&graph.graph);
                *ca == graph_ca || graph_contains(&graph.graph, ca)
            })
            .unwrap_or(false)
    })
}
