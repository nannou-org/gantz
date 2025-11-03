use crate::{
    graph::{Graph, GraphNode},
    node::Node,
};
use bevy::ecs::resource::Resource;
use gantz_egui::ContentAddr;
use petgraph::visit::{IntoNodeReferences, NodeRef};
use serde::{Deserialize, Serialize};
use std::{
    any::Any,
    collections::{BTreeMap, HashMap},
};

/// The type used to track mappings between node names, content addresses and
/// graphs. Also provides access to the node registry. This can be thought of as
/// a shared immutable input to all nodes.
#[derive(Resource)]
pub struct Environment {
    /// Constructors for all primitive nodes.
    pub primitives: Primitives,
    /// The registry of all nodes composed from other nodes.
    pub registry: NodeTypeRegistry,
}

/// The registry for all named graphs, i.e. nodes composed from other nodes.
#[derive(Default, Deserialize, Serialize)]
pub struct NodeTypeRegistry {
    /// A mapping from content addresses to graphs.
    pub graphs: HashMap<ContentAddr, Graph>,
    /// A mapping from names to graph content addresses.
    pub names: BTreeMap<String, ContentAddr>,
}

/// Constructors for all primitive nodes.
type Primitives = BTreeMap<String, Box<dyn Send + Sync + Fn() -> Box<dyn Node>>>;

// Provide the `NodeTypeRegistry` implementation required by `gantz_egui`.
impl gantz_egui::widget::gantz::NodeTypeRegistry for Environment {
    type Node = Box<dyn Node>;

    fn node_types(&self) -> impl Iterator<Item = &str> {
        let mut types = vec![];
        types.extend(self.primitives.keys().map(|s| &s[..]));
        types.extend(self.registry.names.keys().map(|s| &s[..]));
        types.sort();
        types.into_iter()
    }

    fn new_node(&self, node_type: &str) -> Option<Self::Node> {
        self.registry
            .names
            .get(node_type)
            .map(|&ca| {
                let named = gantz_egui::node::NamedGraph::new(node_type.to_string(), ca);
                Box::new(named) as Box<_>
            })
            .or_else(|| self.primitives.get(node_type).map(|f| (f)()))
    }
}

// Provide the `GraphRegistry` implementation required by `gantz_egui`.
impl gantz_egui::node::graph::GraphRegistry for Environment {
    type Node = Box<dyn Node>;
    fn graph(&self, ca: ContentAddr) -> Option<&gantz_core::node::graph::Graph<Self::Node>> {
        self.registry.graphs.get(&ca)
    }
}

// Provide the `GraphRegistry` implementation required by the `GraphSelect` widget.
impl gantz_egui::widget::graph_select::GraphRegistry for Environment {
    fn addrs(&self) -> Vec<ContentAddr> {
        let mut vec: Vec<_> = self.registry.graphs.keys().copied().collect();
        vec.sort();
        vec
    }

    fn names(&self) -> &BTreeMap<String, ContentAddr> {
        &self.registry.names
    }
}

/// The set of all known node types accessible to gantz.
pub fn primitives() -> Primitives {
    let mut p = Primitives::default();
    register_primitive(&mut p, "add", || {
        Box::new(gantz_std::ops::Add::default()) as Box<_>
    });
    register_primitive(&mut p, "bang", || {
        Box::new(gantz_std::Bang::default()) as Box<_>
    });
    register_primitive(&mut p, "expr", || {
        Box::new(gantz_core::node::Expr::new("()").unwrap()) as Box<_>
    });
    register_primitive(&mut p, "graph", || Box::new(GraphNode::default()) as Box<_>);
    register_primitive(&mut p, "inlet", || {
        Box::new(gantz_core::node::graph::Inlet::default()) as Box<_>
    });
    register_primitive(&mut p, "outlet", || {
        Box::new(gantz_core::node::graph::Outlet::default()) as Box<_>
    });
    register_primitive(&mut p, "log", || {
        Box::new(gantz_std::Log::default()) as Box<_>
    });
    register_primitive(&mut p, "number", || {
        Box::new(gantz_std::Number::default()) as Box<_>
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

/// Prune all unused graph entries from the registry.
pub fn prune_unused_graphs(reg: &mut NodeTypeRegistry, active_ca: ContentAddr) {
    let to_remove: Vec<_> = reg
        .graphs
        .keys()
        .copied()
        .filter(|&ca| ca != active_ca && !graph_in_use(reg, ca))
        .collect();
    for ca in to_remove {
        reg.graphs.remove(&ca);
    }
}

/// Tests whether or not the graph with the given content address is in use
/// within the registry.
///
/// This is used to determine whether or not to remove unused graphs.
fn graph_in_use(reg: &NodeTypeRegistry, ca: ContentAddr) -> bool {
    reg.names.values().any(|&n_ca| ca == n_ca)
        || reg.graphs.values().any(|g| graph_contains_ca(g, ca))
}

/// Whether or not the graph contains a subgraph with the given CA.
fn graph_contains_ca(g: &Graph, ca: ContentAddr) -> bool {
    g.node_references().any(|n_ref| {
        let node = n_ref.weight();
        ((&**node) as &dyn Any)
            .downcast_ref::<GraphNode>()
            .map(|graph| {
                let graph_ca = gantz_egui::graph_content_addr(&graph.graph);
                ca == graph_ca || graph_contains_ca(&graph.graph, ca)
            })
            .unwrap_or(false)
    })
}
