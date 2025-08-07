//! Items related to constructing a view of the control flow of a gantz graph.

use super::{Meta, MetaGraph, push_eval_neighbors, push_reachable};
use crate::node;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt, ops,
};

/// Represents all control flow graphs for all entrypoints in a single gantz graph.
///
/// This includes all branches on the edges, and unique node configurations as
/// nodes.
#[derive(Debug)]
pub struct Flow {
    /// Control flow graph from all inlets to all outlets, or empty in the case
    /// that the graph has no inlets or outlets (i.e. is not nested).
    pub nested: FlowGraph,
    /// The control flow graph for each `push_eval` configuration for each node.
    pub push: BTreeMap<(node::Id, node::Conns), FlowGraph>,
    /// The control flow graph for each `pull_eval` configuration for each node.
    pub pull: BTreeMap<(node::Id, node::Conns), FlowGraph>,
}

/// Represents a basic, linear block of node function calls.
#[derive(Clone, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct Block(pub Vec<NodeConf>);

/// The control flow graph.
///
/// Nodes represent basic blocks of node function calls, edges represent the
/// unique output branching that leads between blocks.
pub type FlowGraph = petgraph::stable_graph::StableDiGraph<Block, BranchConns>;

/// One of the
type BranchConns = node::Conns;

/// A control flow graph describing indifidual node function call dependency.
///
/// Each `NodeConf` node represents a node function call statement that should
/// be made.
///
/// This is derived from the `Meta` graph and is used to construct the
/// `FlowGraph` via edge contraction.
pub type NodeConfGraph = petgraph::graphmap::DiGraphMap<NodeConf, BranchConns>;

/// A node within the control flow graph.
///
/// Maps directly to a node function.
#[derive(Copy, Clone, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct NodeConf {
    pub id: node::Id,
    pub conns: NodeConns,
}

/// The connectedness of a node for a particular evaluation step.
#[derive(Copy, Clone, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct NodeConns {
    /// The active inputs.
    pub inputs: node::Conns,
    /// Includes all connected outputs (whether conditional or not).
    pub outputs: node::Conns,
}

impl Flow {
    pub fn from_meta(meta: &Meta) -> Self {
        // Create the push eval entrypoint control flow graphs.
        let push = meta
            .push
            .iter()
            .flat_map(|(&n, connss)| {
                connss
                    .iter()
                    .map(move |conns| ((n, *conns), push_eval_flow_graph(meta, n, conns)))
            })
            .collect();

        let pull = meta
            .pull
            .iter()
            .flat_map(|(&n, connss)| {
                connss
                    .iter()
                    .map(move |conns| ((n, *conns), pull_eval_flow_graph(meta, n, conns)))
            })
            .collect();

        let nested = flow_graph(
            meta,
            meta.inlets
                .iter()
                .map(|&n| (n, node::Conns::connected(1).unwrap())),
            meta.outlets
                .iter()
                .map(|&n| (n, node::Conns::connected(1).unwrap())),
        );

        Self { nested, push, pull }
    }
}

impl fmt::Debug for NodeConf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:?}: {:?})", self.id, self.conns)
    }
}

impl fmt::Debug for NodeConns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "([{}], [{}])", self.inputs, self.outputs)
    }
}

impl ops::Deref for Block {
    type Target = Vec<NodeConf>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for Block {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Given a meta graph and set of push and pull eval fn nodes, construct a full
/// control flow graph.
pub fn flow_graph(
    meta: &Meta,
    push: impl IntoIterator<Item = (node::Id, node::Conns)>,
    pull: impl IntoIterator<Item = (node::Id, node::Conns)>,
) -> FlowGraph {
    let order: Vec<_> = super::eval_order(&meta.graph, push, pull).collect();
    let included: HashSet<_> = order.iter().copied().collect();
    let mg = reachable_subgraph(&meta.graph, &included);
    let conf_graph = node_conf_graph(meta, &mg, None);
    flow_graph_from_conf_graph(&conf_graph)
}

/// Given the meta graph and a node registered as a `push_eval` entrypoint,
/// produce the control flow graph.
fn push_eval_flow_graph(meta: &Meta, n: node::Id, conns: &node::Conns) -> FlowGraph {
    flow_graph(meta, Some((n, *conns)), std::iter::empty())
}

/// Given the meta graph and a node registered as a `pull_eval` entrypoint,
/// produce the control flow graph.
fn pull_eval_flow_graph(meta: &Meta, n: node::Id, conns: &node::Conns) -> FlowGraph {
    flow_graph(meta, std::iter::empty(), Some((n, *conns)))
}

/// Filter unreachable nodes from the given metagraph.
fn reachable_subgraph(g: &MetaGraph, reachable: &HashSet<node::Id>) -> MetaGraph {
    g.all_edges()
        .filter(|(a, b, _)| reachable.contains(a) && reachable.contains(b))
        .map(|(a, b, w)| (a, b, w.clone()))
        .collect()
}

/// Given a node configuration flow graph, return the reduced control flow graph
/// of basic blocks.
fn flow_graph_from_conf_graph(cg: &NodeConfGraph) -> FlowGraph {
    // Initialise the flow graph with the same nodes and edges.
    let mut g = FlowGraph::with_capacity(cg.node_count(), cg.edge_count());
    let mut visited = HashMap::with_capacity(cg.node_count());
    for (a, b, &branch) in cg.all_edges() {
        let na = *visited
            .entry(a)
            .or_insert_with(|| g.add_node(Block(vec![a])));
        let nb = *visited
            .entry(b)
            .or_insert_with(|| g.add_node(Block(vec![b])));
        g.add_edge(na, nb, branch);
    }
    flow_graph_edge_contraction(&mut g);
    g
}

/// For the given flow graph, contract all edges into basic blocks where
/// possible.
///
/// Ie for each edge, if that edge is the only output for the source node, and
/// the only input for the destination node, remove the edge and merge the src
/// and dst nodes.
fn flow_graph_edge_contraction(g: &mut FlowGraph) {
    // Maintain a stack of all edges that require reducing.
    let mut edges: Vec<_> = g.edge_references().map(|e_ref| e_ref.id()).collect();
    while let Some(e) = edges.pop() {
        let (src, dst) = g.edge_endpoints(e).unwrap();

        // Check whether or not this is the only edge between src and dst.
        let mergeable = g.edges_directed(src, petgraph::Outgoing).take(2).count() == 1
            && g.edges_directed(dst, petgraph::Incoming).take(2).count() == 1;
        if !mergeable {
            continue;
        }

        // Merge the src and dst blocks.
        let (src_blk, dst_blk) = g.index_twice_mut(src, dst);
        src_blk.0.append(&mut dst_blk.0);

        // Re-attach the dst output edges to the src.
        let new_edges: Vec<_> = g
            .edges_directed(dst, petgraph::Outgoing)
            .map(|e_ref| (e_ref.target(), *e_ref.weight()))
            .collect();
        for (new_dst, w) in new_edges {
            let new_e = g.add_edge(src, new_dst, w);
            // Add the edges to the stack in case they need to be re-checked.
            // FIXME: Shouldn't be necessary to re-add edges to the stack if we
            // check all edges in reverse topo order? I think we are but we
            // don't explicitly assert this anywhere.
            edges.push(new_e);
        }

        // Remove the dst node now that it's been merged.
        g.remove_node(dst);
    }
}

/// Given some node within a given meta graph with an expected total number of
/// inputs, return the list of inputs that are actually connected.
fn node_inputs(g: &MetaGraph, n: node::Id, n_inputs: usize) -> node::Conns {
    let mut inputs = node::Conns::unconnected(n_inputs).unwrap();
    for e_ref in g.edges_directed(n, petgraph::Incoming) {
        for (edge, _kind) in e_ref.weight() {
            inputs.set(edge.input.0 as usize, true).unwrap();
        }
    }
    inputs
}

/// Given some node within a given meta graph with an expected total number of
/// outputs, return the list of outputs that are actually connected.
fn node_outputs(g: &MetaGraph, n: node::Id, n_outputs: usize) -> node::Conns {
    let mut outputs = node::Conns::unconnected(n_outputs).unwrap();
    for e_ref in g.edges_directed(n, petgraph::Outgoing) {
        for (edge, _kind) in e_ref.weight() {
            outputs.set(edge.output.0 as usize, true).unwrap();
        }
    }
    outputs
}

/// For the given meta graph `mg`, produce its control flow graph.
///
/// The given meta graph should only contain the nodes reachable in the desired
/// evaluation path.
//
// FIXME:
// - The first edge from branching nodes should use `branch` - is this
//   happening?
// - Refactor this to not use recursion on branching.
// TODO:
// - Refactor to avoid gross `first_node_conns` input.
fn node_conf_graph(
    meta: &Meta,
    mg: &MetaGraph,
    first_node_conns: Option<NodeConns>,
) -> NodeConfGraph {
    let mut g = NodeConfGraph::new();

    // Walk nodes in topological order until we hit a branch.
    let mut topo = petgraph::visit::Topo::new(mg);
    let mut last = None;
    while let Some(n) = topo.next(mg) {
        // Determine the configuration and add it.
        let n_inputs = meta.inputs.get(&n).copied().unwrap_or(0);
        let n_outputs = meta.outputs.get(&n).copied().unwrap_or(0);
        let inputs = node_inputs(&mg, n, n_inputs);
        let outputs = node_outputs(&mg, n, n_outputs);
        let conns = NodeConns { inputs, outputs };
        let mut conf = NodeConf { id: n, conns };

        // Add an edge from the prev node.
        if let Some(prev) = last {
            g.add_edge(prev, conf, outputs);

        // If there is no last node, this is the first node.
        } else {
            // If a set of outputs were provided for the first node, this is for
            // the beginning node in a branch.
            if let Some(conns) = first_node_conns {
                conf.conns = conns;
            }
            g.add_node(conf);
        }

        // If this is not the first node, and the node branches,
        // determine all possible branching outputs from this node.
        if let (Some(branches), true) = (meta.branches.get(&n), last.is_some()) {
            // For each branch, collect the subgraph.
            // FIXME: Make this non-recursive.

            for branch in branches {
                let nbs = push_eval_neighbors(mg, n, branch);
                let reachable: HashSet<_> = push_reachable(mg, n, &nbs).collect();
                let sub_mg = reachable_subgraph(mg, &reachable);
                // FIXME: This adds the branching node, but with a subset of
                // outputs. We only want the branching node at the end of the
                // block with all connected outputs in the config, then the
                // *edges* should have the branch subsets.
                let sub_ncg = node_conf_graph(meta, &sub_mg, Some(conns));
                g.extend(sub_ncg.all_edges().map(|(a, b, &w)| (a, b, w)));
            }

            // We've handled the branches in the recursive cases - we're done.
            break;
        }

        last = Some(conf);
    }

    g
}
