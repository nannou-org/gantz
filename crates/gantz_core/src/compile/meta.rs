//! Items related to collecting a high-level "meta" view of a gantz graph.

use crate::{
    Edge,
    compile::{
        RoseTree,
        error::{InvalidOutputIndex, MetaError, NodeConnsError, TooManyConns},
    },
    node::{self, Node, Role},
    visit::{self, Visitor},
};
use petgraph::visit::{
    Data, EdgeRef, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, NodeRef,
};
use std::collections::{BTreeMap, BTreeSet};

/// A high-level representation of a gantz graph.
///
/// This is produced as the first stage of code-generation and acts as a
/// high-level overview of the gantz graph that can be used for faster
/// traversal and node metadata lookup.
#[derive(Debug, Default)]
pub struct Meta {
    pub graph: MetaGraph,
    /// The set of nodes that require branching on their outputs.
    pub branches: BTreeMap<node::Id, Vec<node::Conns>>,
    /// The set of nodes that require access to state.
    pub stateful: BTreeSet<node::Id>,
    /// The set of nodes that act as inlets of a nested graph.
    pub inlets: BTreeSet<node::Id>,
    /// The set of nodes that act as outlets of a nested graph.
    pub outlets: BTreeSet<node::Id>,
    /// The set of unit-delay nodes. Evaluation never propagates through
    /// them. Cycles containing one are legal.
    pub delays: BTreeSet<node::Id>,
    /// The total number of inputs on each node, connected or not.
    pub inputs: BTreeMap<node::Id, usize>,
    /// The total number of outputs on each node, connected or not.
    pub outputs: BTreeMap<node::Id, usize>,
    /// The names of the registered Steel modules required by this graph's
    /// nodes. See [`Node::required_modules`].
    pub requires: BTreeSet<String>,
}

/// The graph of a single level.
///
/// The edge weight is a `Vec` so that multiple edges between the same two
/// nodes are represented. An edge from an output that no branch of its
/// source activates is left out, as it never carries a value.
pub type MetaGraph = petgraph::graphmap::DiGraphMap<node::Id, Vec<Edge>>;

/// A rose tree of [`Meta`] with error accumulation during visitation.
///
/// Errors encountered during the visitor traversal are accumulated in the
/// `errors` field rather than panicking, allowing all errors to be reported.
#[derive(Debug, Default)]
pub(crate) struct MetaTree {
    /// The rose tree of meta information for the graph and its nested graphs.
    pub tree: RoseTree<Meta>,
    /// Errors accumulated during the visitor traversal.
    pub errors: Vec<MetaError>,
}

impl Meta {
    /// Construct a `Meta` for a single gantz graph.
    pub fn from_graph<'a, G>(get_node: node::GetNode<'a>, g: G) -> Result<Self, NodeConnsError>
    where
        G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable,
        G::NodeWeight: Node,
    {
        let mut flow = Meta::default();
        let ctx = node::MetaCtx::new(get_node);
        for n_ref in g.node_references() {
            let n = n_ref.id();
            let inputs = g
                .edges_directed(n, petgraph::Direction::Incoming)
                .map(|e_ref| (g.to_index(e_ref.source()), e_ref.weight().clone()));
            let id = g.to_index(n);
            let node = n_ref.weight();
            flow.add_node(ctx, id, node, inputs)?;
        }
        Ok(flow)
    }

    /// Add the node with the given ID and inputs to the `Meta`.
    ///
    /// The result is the same for every order in which the nodes of a graph
    /// are added.
    pub fn add_node(
        &mut self,
        ctx: node::MetaCtx,
        id: node::Id,
        node: &dyn Node,
        inputs: impl IntoIterator<Item = (node::Id, Edge)>,
    ) -> Result<(), NodeConnsError> {
        self.graph.add_node(id);

        // A source added after this node checks its edges when it is added.
        for (n, edge) in inputs {
            let n_branches = self.branches.get(&n).map(|bs| &bs[..]);
            if !output_live(n_branches, edge.output.0 as usize)? {
                continue;
            }
            if let Some(edges) = self.graph.edge_weight_mut(n, id) {
                edges.push(edge);
            } else {
                self.graph.add_edge(n, id, vec![edge]);
            }
        }

        let inputs = node.n_inputs(ctx);
        let outputs = node.n_outputs(ctx);
        if inputs > 0 {
            self.inputs.insert(id, inputs);
        }
        if outputs > 0 {
            self.outputs.insert(id, outputs);
        }

        let branches = node.branches(ctx);
        if !branches.is_empty() {
            let branches: Vec<node::Conns> = branches
                .iter()
                .map(|conf| conns_from_eval_conf(conf, outputs))
                .collect::<Result<_, _>>()?;
            // The edges to the nodes added before this one.
            let targets: Vec<node::Id> = self
                .graph
                .neighbors_directed(id, petgraph::Direction::Outgoing)
                .collect();
            for target in targets {
                let edges = self.graph.edge_weight_mut(id, target).expect("edge exists");
                let mut live = Vec::with_capacity(edges.len());
                for edge in edges.drain(..) {
                    if output_live(Some(&branches), edge.output.0 as usize)? {
                        live.push(edge);
                    }
                }
                *edges = live;
                if edges.is_empty() {
                    self.graph.remove_edge(id, target);
                }
            }
            self.branches.insert(id, branches);
        }

        match node.role(ctx) {
            Role::Plain => (),
            Role::Inlet => {
                self.inlets.insert(id);
            }
            Role::Outlet => {
                self.outlets.insert(id);
            }
            Role::Delay => {
                self.delays.insert(id);
            }
        }
        if node.stateful(ctx) {
            self.stateful.insert(id);
        }
        self.requires.extend(node.required_modules(ctx));
        Ok(())
    }
}

/// Constructs a rose-tree of `Meta`s, one for each graph, via the visitor.
impl Visitor for MetaTree {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &dyn Node) {
        let node_path = ctx.path();

        let tree_path = &node_path[..node_path.len() - 1];
        let tree = self.tree.tree_mut(tree_path);

        let id = ctx.id();
        let meta_ctx = node::MetaCtx::new(ctx.get_node());
        if let Err(error) = tree
            .elem
            .add_node(meta_ctx, id, node, ctx.inputs().iter().copied())
        {
            self.errors.push(MetaError {
                path: node_path.to_vec(),
                error,
            });
        }
    }
}

/// Given an eval conf and a known number of connections, convert the conf to
/// the set of conns.
pub(crate) fn conns_from_eval_conf(
    conf: &node::EvalConf,
    n_conns: usize,
) -> Result<node::Conns, TooManyConns> {
    match conf {
        node::EvalConf::All => node::Conns::try_from_iter((0..n_conns).map(|_| true))
            .map_err(|_| TooManyConns(n_conns)),
        node::EvalConf::Set(conns) => Ok(*conns),
    }
}

/// Whether any branch of a source node activates its output `out_ix`. An
/// output of a node with no branches is always active.
fn output_live(confs: Option<&[node::Conns]>, out_ix: usize) -> Result<bool, InvalidOutputIndex> {
    let Some(confs) = confs else {
        return Ok(true);
    };
    let mut live = false;
    for branch in confs {
        live |= branch.get(out_ix).ok_or_else(|| InvalidOutputIndex {
            index: out_ix,
            n_outputs: branch.len(),
        })?;
    }
    Ok(live)
}
