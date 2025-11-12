use crate::{
    graph::{self, Graph, GraphNode},
    node::Node,
};
use bevy::ecs::resource::Resource;
use gantz_egui::ca;
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
    /// A mapping from graph addresses to graphs.
    pub graphs: HashMap<ca::GraphAddr, Graph>,
    /// A mapping from commit addresses to commits.
    pub commits: HashMap<ca::CommitAddr, ca::Commit>,
    /// A mapping from names to graph content addresses.
    pub names: BTreeMap<String, ca::CommitAddr>,
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
            .and_then(|commit_ca| {
                let graph_ca = self.registry.commits.get(commit_ca)?.graph;
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
        self.registry.graphs.get(&ca)
    }
}

// Provide the `GraphRegistry` implementation required by the `GraphSelect` widget.
impl gantz_egui::widget::graph_select::GraphRegistry for Environment {
    fn commits(&self) -> Vec<(&ca::CommitAddr, &ca::Commit)> {
        // Sort commits by newest to oldest.
        let mut commits: Vec<_> = self.registry.commits.iter().collect();
        commits.sort_by(|(_, a), (_, b)| b.timestamp.cmp(&a.timestamp));
        commits
    }

    fn names(&self) -> &BTreeMap<String, ca::CommitAddr> {
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

/// Initialise head to a new initial commit pointing to an empty graph.
pub fn init_head(registry: &mut NodeTypeRegistry) -> ca::Head {
    // Register an empty graph.
    let graph = Graph::default();
    let graph_ca = ca::graph_addr(&graph);
    registry.graphs.insert(graph_ca, graph);

    // Register an initial commit.
    let commit = ca::Commit::timestamped(None, graph_ca);
    let commit_ca = ca::commit_addr(&commit);
    registry.commits.insert(commit_ca, commit);

    ca::Head::Commit(commit_ca)
}

/// Commit the given graph to the given head.
pub fn commit_graph_to_head(
    reg: &mut NodeTypeRegistry,
    head: &mut ca::Head,
    graph: &Graph,
    graph_ca: ca::GraphAddr,
) {
    // Ensure the graph is registerd.
    reg.graphs
        .entry(graph_ca)
        .or_insert_with(|| graph::clone(graph));

    // Create a new commit.
    let parent_ca = *head_commit_ca(&reg.names, head).unwrap();
    let commit = ca::Commit::timestamped(Some(parent_ca), graph_ca);
    let commit_ca = ca::commit_addr(&commit);
    reg.commits.insert(commit_ca, commit);

    // Update head, or insure the name mapping is up-to-date.
    match *head {
        ca::Head::Commit(ref mut ca) => *ca = commit_ca,
        ca::Head::Branch(ref name) => {
            reg.names.insert(name.to_string(), commit_ca);
        }
    }
}

/// Look-up the commit address pointed to by the given head.
pub fn head_commit_ca<'a>(
    names: &'a BTreeMap<String, ca::CommitAddr>,
    head: &'a ca::Head,
) -> Option<&'a ca::CommitAddr> {
    match head {
        ca::Head::Branch(name) => names.get(name),
        ca::Head::Commit(ca) => Some(ca),
    }
}

/// Look-up the commit pointed to by the given head.
pub fn head_commit<'a>(reg: &'a NodeTypeRegistry, head: &'a ca::Head) -> Option<&'a ca::Commit> {
    head_commit_ca(&reg.names, head).and_then(|ca| reg.commits.get(&ca))
}

/// Look-up the graph pointed to by the head.
pub fn head_graph<'a>(reg: &'a NodeTypeRegistry, head: &'a ca::Head) -> Option<&'a Graph> {
    head_commit(reg, head).and_then(|commit| reg.graphs.get(&commit.graph))
}

/// Prune all unused graph entries from the registry.
pub fn prune_unused_graphs(reg: &mut NodeTypeRegistry, head: &ca::Head) {
    let head_graph_ca = head_commit(&reg, head).map(|c| c.graph);
    let to_remove: Vec<_> = reg
        .graphs
        .keys()
        .copied()
        .filter(|&ca| Some(ca) != head_graph_ca && !graph_in_use(reg, ca))
        .collect();
    for ca in to_remove {
        reg.graphs.remove(&ca);
    }
}

/// Tests whether or not the graph with the given content address is in use
/// within the registry.
///
/// This is used to determine whether or not to remove unused graphs.
fn graph_in_use(reg: &NodeTypeRegistry, ca: ca::GraphAddr) -> bool {
    reg.names
        .values()
        .any(|commit_ca| ca == reg.commits[commit_ca].graph)
        || reg.graphs.values().any(|g| graph_contains_ca(g, ca))
}

/// Whether or not the graph contains a subgraph with the given CA.
fn graph_contains_ca(g: &Graph, ca: ca::GraphAddr) -> bool {
    g.node_references().any(|n_ref| {
        let node = n_ref.weight();
        ((&**node) as &dyn Any)
            .downcast_ref::<GraphNode>()
            .map(|graph| {
                let graph_ca = ca::graph_addr(&graph.graph);
                ca == graph_ca || graph_contains_ca(&graph.graph, ca)
            })
            .unwrap_or(false)
    })
}

/// Prunes all commits pointing to graphs that no longer exist.
///
/// Intended for running after `prune_unused_graphs`.
pub fn prune_graphless_commits(reg: &mut NodeTypeRegistry) {
    reg.commits
        .retain(|_ca, commit| reg.graphs.contains_key(&commit.graph));
}
