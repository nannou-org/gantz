use crate::{
    Edge,
    codegen::RoseTree,
    node::{self, Node},
    visit::{self, Visitor},
};
use petgraph::{
    Directed,
    visit::{Data, EdgeRef, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, NodeRef},
};
use std::collections::{BTreeMap, BTreeSet};

/// Represents the overall flow/structure of a single gantz graph.
#[derive(Debug, Default)]
pub struct Flow {
    pub graph: FlowGraph,
    /// The set of nodes that require a push evaluation fn.
    pub push: BTreeMap<node::Id, Vec<node::EvalConf>>,
    /// The set of nodes that require a pull evaluation fn.
    pub pull: BTreeMap<node::Id, Vec<node::EvalConf>>,
    /// The set of nodes that require access to state.
    pub stateful: BTreeSet<node::Id>,
    /// The set of nodes that act as inlets (for nested graphs).
    pub inlets: BTreeSet<node::Id>,
    /// The set of nodes that act as outlets (for nested graphs).
    pub outlets: BTreeSet<node::Id>,
    /// The total number of inputs on node (whether or not they're connected).
    pub inputs: BTreeMap<node::Id, usize>,
    /// The total number of outputs on node (whether or not they're connected).
    pub outputs: BTreeMap<node::Id, usize>,
}

/// Represents a single flow graph.
///
/// Note that we use a `Vec<Edge>` in order to represent multiple edges
/// between the same two nodes.
type FlowGraph = petgraph::graphmap::GraphMap<node::Id, Vec<Edge>, Directed>;

impl Flow {
    /// Construct a `Flow` for a single gantz graph.
    pub fn from_graph<G>(g: G) -> Self
    where
        G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable,
        G::NodeWeight: Node,
    {
        let mut flow = Flow::default();
        for n_ref in g.node_references() {
            let n = n_ref.id();
            let inputs = g
                .edges_directed(n, petgraph::Direction::Incoming)
                .map(|e_ref| (g.to_index(e_ref.source()), e_ref.weight().clone()));
            let id = g.to_index(n);
            let node = n_ref.weight();
            flow.add_node(id, node, inputs);
        }
        flow
    }

    /// Add the node with the given ID and inputs to the `Flow`.
    pub fn add_node(
        &mut self,
        id: node::Id,
        node: &dyn Node,
        inputs: impl IntoIterator<Item = (node::Id, Edge)>,
    ) {
        // Add the node.
        self.graph.add_node(id);

        // Add edges for inputs.
        for (n, edge) in inputs {
            loop {
                if let Some(edges) = self.graph.edge_weight_mut(n, id) {
                    edges.push(edge);
                    break;
                }
                self.graph.add_edge(n, id, vec![]);
            }
        }

        // Register whether the node has inputs or outputs.
        let inputs = node.n_inputs();
        let outputs = node.n_outputs();
        if inputs > 0 {
            self.inputs.insert(id, inputs);
        }
        if outputs > 0 {
            self.outputs.insert(id, outputs);
        }

        // Register push/pull eval for the node if necessary.
        let push_eval = node.push_eval();
        if !push_eval.is_empty() {
            self.push.insert(id, push_eval);
        }
        let pull_eval = node.pull_eval();
        if !pull_eval.is_empty() {
            self.pull.insert(id, pull_eval);
        }
        if node.inlet() {
            self.inlets.insert(id);
        }
        if node.outlet() {
            self.outlets.insert(id);
        }
        if node.stateful() {
            self.stateful.insert(id);
        }
    }
}

/// Allow for constructing a rose-tree of `Flow`s (one for each graph) using
/// the `Node::visit` implementation.
impl Visitor for RoseTree<Flow> {
    fn visit_pre(&mut self, ctx: visit::Ctx, node: &dyn Node) {
        let node_path = ctx.path();

        // Ensure the plan for the graph owning this node exists, retrieve it.
        let tree_path = &node_path[..node_path.len() - 1];
        let tree = self.tree_mut(&tree_path);

        // Insert the node.
        let id = ctx.id();
        tree.elem.add_node(id, node, ctx.inputs().iter().copied());
    }
}
