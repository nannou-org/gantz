//! Items related to collecting a high-level "meta" view of a gantz graph.

use crate::{
    Edge,
    compile::RoseTree,
    node::{self, Node},
    visit::{self, Visitor},
};
use petgraph::visit::{
    Data, EdgeRef, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, NodeRef,
};
use std::collections::{BTreeMap, BTreeSet};

/// Represents a high-level representation of a gantz graph.
///
/// This is produced as the first stage of code-generation and acts as a
/// high-level overview of the gantz graph that can be used for faster
/// traversal and node metadata lookup.
#[derive(Debug, Default)]
pub struct Meta {
    pub graph: MetaGraph,
    /// The set of nodes that require branching on their outputs.
    pub branches: BTreeMap<node::Id, Vec<node::Conns>>,
    /// The set of nodes that require a push evaluation fn.
    pub push: BTreeMap<node::Id, Vec<node::Conns>>,
    /// The set of nodes that require a pull evaluation fn.
    pub pull: BTreeMap<node::Id, Vec<node::Conns>>,
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

/// Whether an edge is known to always be traversed, or whether it is
/// conditional.
#[derive(Clone, Debug)]
pub enum EdgeKind {
    /// The edge is known to always be connected.
    Static,
    /// The edge is conditional on node branching.
    Conditional,
}

/// Represents a single flow graph.
///
/// Note that we use a `Vec<Edge>` in order to represent multiple edges
/// between the same two nodes.
pub type MetaGraph = petgraph::graphmap::DiGraphMap<node::Id, Vec<(Edge, EdgeKind)>>;

impl Meta {
    /// Construct a `Meta` for a single gantz graph.
    pub fn from_graph<Env, G>(env: &Env, g: G) -> Self
    where
        G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable,
        G::NodeWeight: Node<Env>,
    {
        let mut flow = Meta::default();
        for n_ref in g.node_references() {
            let n = n_ref.id();
            let inputs = g
                .edges_directed(n, petgraph::Direction::Incoming)
                .map(|e_ref| (g.to_index(e_ref.source()), e_ref.weight().clone()));
            let id = g.to_index(n);
            let node = n_ref.weight();
            flow.add_node(env, id, node, inputs);
        }
        flow
    }

    /// Add the node with the given ID and inputs to the `Meta`.
    pub fn add_node<Env>(
        &mut self,
        env: &Env,
        id: node::Id,
        node: &dyn Node<Env>,
        inputs: impl IntoIterator<Item = (node::Id, Edge)>,
    ) {
        // Add the node.
        self.graph.add_node(id);

        // Add edges for inputs.
        for (n, edge) in inputs {
            loop {
                if let Some(edges) = self.graph.edge_weight_mut(n, id) {
                    let n_branches = self.branches.get(&n).map(|bs| &bs[..]);
                    if let Some(kind) = edge_kind(n_branches, edge.output.0 as usize) {
                        edges.push((edge, kind));
                        break;
                    }
                }
                self.graph.add_edge(n, id, vec![]);
            }
        }

        // Register whether the node has inputs or outputs.
        let inputs = node.n_inputs(env);
        let outputs = node.n_outputs(env);
        if inputs > 0 {
            self.inputs.insert(id, inputs);
        }
        if outputs > 0 {
            self.outputs.insert(id, outputs);
        }

        // Track node branching.
        let branches = node.branches(env);
        if !branches.is_empty() {
            self.branches.insert(
                id,
                branches
                    .iter()
                    .map(|conf| conns_from_eval_conf(conf, outputs))
                    .collect(),
            );
        }

        // Register push/pull eval for the node if necessary.
        let push_eval = node.push_eval(env);
        if !push_eval.is_empty() {
            self.push.insert(
                id,
                push_eval
                    .iter()
                    .map(|conf| conns_from_eval_conf(conf, outputs))
                    .collect(),
            );
        }
        let pull_eval = node.pull_eval(env);
        if !pull_eval.is_empty() {
            self.pull.insert(
                id,
                pull_eval
                    .iter()
                    .map(|conf| conns_from_eval_conf(conf, inputs))
                    .collect(),
            );
        }
        if node.inlet(env) {
            self.inlets.insert(id);
        }
        if node.outlet(env) {
            self.outlets.insert(id);
        }
        if node.stateful(env) {
            self.stateful.insert(id);
        }
    }
}

/// Allow for constructing a rose-tree of `Meta`s (one for each graph) using
/// the `Node::visit` implementation.
impl<Env> Visitor<Env> for RoseTree<Meta> {
    fn visit_pre(&mut self, ctx: visit::Ctx<Env>, node: &dyn Node<Env>) {
        let node_path = ctx.path();

        // Ensure the plan for the graph owning this node exists, retrieve it.
        let tree_path = &node_path[..node_path.len() - 1];
        let tree = self.tree_mut(&tree_path);

        // Insert the node.
        let id = ctx.id();
        tree.elem
            .add_node(ctx.env(), id, node, ctx.inputs().iter().copied());
    }
}

impl super::Edges for Vec<(Edge, EdgeKind)> {
    fn edges(&self) -> impl Iterator<Item = Edge> {
        self.iter().map(|(e, _k)| *e)
    }
}

/// Given an eval conf and a known number of connections, convert the conf to
/// the set of conns.
fn conns_from_eval_conf(conf: &node::EvalConf, n_conns: usize) -> node::Conns {
    match conf {
        node::EvalConf::All => node::Conns::try_from_iter((0..n_conns).map(|_| true)).unwrap(),
        node::EvalConf::Set(conns) => *conns,
    }
}

/// Given the branching of the source node and the output index of a connected
/// edge, returns the `EdgeKind` of that edge, or `None` if there is no branch
/// under which the edge can be reached.
fn edge_kind(confs: Option<&[node::Conns]>, out_ix: usize) -> Option<EdgeKind> {
    let Some(confs) = confs else {
        return Some(EdgeKind::Static);
    };
    let mut reachable = false;
    let mut conditional = false;
    for branch in confs {
        let active = branch.get(out_ix).expect("missing output in branch");
        reachable |= active;
        conditional |= !active;
    }
    match (reachable, conditional) {
        (false, _) => None,
        (true, true) => Some(EdgeKind::Conditional),
        (true, false) => Some(EdgeKind::Static),
    }
}
