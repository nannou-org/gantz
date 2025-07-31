//! Items related to generating steel code from a gantz graph, primarily the
//! [`module`] fn.

use crate::{
    Edge,
    node::{self, Node},
};
// FIXME: Make these private, expose easier way to call entry points.
pub(crate) use codegen::eval_stmts;
#[doc(inline)]
pub use codegen::{pull_eval_fn_name, push_eval_fn_name};
#[doc(inline)]
pub use meta::{EdgeKind, Meta};
use petgraph::visit::{
    Data, Dfs, EdgeRef, GraphBase, GraphRef, IntoEdgesDirected, IntoNeighbors, IntoNodeReferences,
    NodeIndexable, Topo, Visitable, Walker,
};
pub(crate) use rosetree::RoseTree;
use std::{
    collections::{BTreeMap, HashSet},
    hash::Hash,
};
use steel::parser::ast::ExprKind;

mod codegen;
mod flow;
mod meta;
mod rosetree;

/// Allows for remaining generic over graph type edges that represent one or
/// more gantz [`Edge`]s.
///
/// This is necessary to support calling the `reachable` fns with both the
/// `Meta` graph and graphs of different types.
pub trait Edges {
    /// Produce an iterator yielding all the [`Edge`]s.
    fn edges(&self) -> impl Iterator<Item = Edge>;
}

/// A representation of how to evaluate a graph.
///
/// Produced via [`eval_plan`].
#[derive(Debug)]
struct EvalPlan<'a> {
    /// The gantz graph `Meta` from which this `EvalPlan` was produced.
    meta: &'a Meta,

    /// Order of evaluation from all inlets to all outlets.
    ///
    /// Empty in the case that the graph has no inlets or outlets (i.e. is not
    /// nested).
    // TODO: Knowing the connectedness of the inlets/outlets would be useful
    // for generating only the necessary node configs.
    nested_steps: Vec<EvalStep>,
    /// The order of node evaluation for each push_eval node.
    push_steps: BTreeMap<node::Id, Vec<EvalStep>>,
    /// The order of node evaluation for each pull_eval node.
    pull_steps: BTreeMap<node::Id, Vec<EvalStep>>,
}

/// An evaluation step ready for translation to code.
///
/// Represents evaluation of a node with some set of the inputs connected.
#[derive(Debug)]
pub(crate) struct EvalStep {
    /// The node to be evaluated.
    pub(crate) node: node::Id,
    /// Arguments to the node's function call.
    ///
    /// The `len` of the outer vec will always be equal to the number of inputs
    /// on `node`.
    pub(crate) inputs: Vec<Option<ExprInput>>,
    /// The set of connected outputs.
    pub(crate) outputs: Vec<bool>,
}

/// An argument to a node's function call.
#[derive(Debug)]
pub(crate) struct ExprInput {
    /// The node from which the value was generated.
    pub(crate) node: node::Id,
    /// The output on the source node associated with the generated value.
    pub(crate) output: node::Output,
}

impl Edges for Edge {
    fn edges(&self) -> impl Iterator<Item = Edge> {
        std::iter::once(*self)
    }
}

impl Edges for Vec<Edge> {
    fn edges(&self) -> impl Iterator<Item = Edge> {
        self.iter().copied()
    }
}

/// Given a graph and an eval src, return the set of direct neighbors that
/// will be included in the initial traversal.
fn eval_neighbors<G>(
    g: G,
    n: G::NodeId,
    conns: &node::Conns,
    src_conn: impl Fn(&Edge) -> usize,
) -> HashSet<G::NodeId>
where
    G: IntoEdgesDirected,
    G::EdgeWeight: Edges,
    G::NodeId: Eq + Hash,
{
    let mut set = HashSet::new();
    for e_ref in g.edges_directed(n, petgraph::Outgoing) {
        for edge in e_ref.weight().edges() {
            let conn_ix = src_conn(&edge);
            let include = conns.get(conn_ix).unwrap();
            if include {
                set.insert(e_ref.target());
            }
        }
    }
    set
}

/// Given a graph and a `EvalConf` src, return the set of direct neighbors that
/// will be included in the initial traversal.
fn push_eval_neighbors<G>(g: G, n: G::NodeId, ev: &node::Conns) -> HashSet<G::NodeId>
where
    G: IntoEdgesDirected,
    G::EdgeWeight: Edges,
    G::NodeId: Eq + Hash,
{
    eval_neighbors(g, n, ev, |edge| edge.output.0 as usize)
}

/// Given a graph and a `EvalConf` src, return the set of direct neighbors that
/// will be included in the initial traversal.
fn pull_eval_neighbors<G>(g: G, n: G::NodeId, ev: &node::Conns) -> HashSet<G::NodeId>
where
    G: IntoEdgesDirected,
    G::EdgeWeight: Edges,
    G::NodeId: Eq + Hash,
{
    let rev_g = petgraph::visit::Reversed(g);
    eval_neighbors(rev_g, n, ev, |edge| edge.input.0 as usize)
}

/// An iterator yielding all nodes reachable via pushing from the given node
/// over the given set of neighbors.
fn reachable<G>(
    g: G,
    src: G::NodeId,
    src_neighbors: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
    G::NodeId: Eq + Hash,
{
    /// A filter around a graph's `IntoNeighbors` implementation that discludes
    /// edges from the root that are not in the given src neighbors set.
    #[derive(Clone, Copy)]
    struct EvalFilter<'a, G: GraphBase> {
        g: G,
        /// The node that is the source of evaluation.
        src: G::NodeId,
        /// The eval src node's neighbors that are included in the eval set.
        src_neighbors: &'a HashSet<G::NodeId>,
    }

    /// The iterator filter applied to the inner graph's neighbors iterator.
    /// If we're iterating over the eval src's neighbors, this will only yield
    /// those specified in the included set.
    struct EvalFilterNeighbors<'a, I: Iterator> {
        neighbors: I,
        // Whether or not `neighbors` are from the eval src.
        is_src: bool,
        /// The eval src node's neighbors that are included in the eval set.
        src_neighbors: &'a HashSet<I::Item>,
    }

    impl<'a, G> GraphBase for EvalFilter<'a, G>
    where
        G: GraphBase,
    {
        type NodeId = G::NodeId;
        type EdgeId = G::EdgeId;
    }

    impl<'a, G: GraphRef> GraphRef for EvalFilter<'a, G> {}

    impl<'a, G> Visitable for EvalFilter<'a, G>
    where
        G: GraphRef + Visitable,
    {
        type Map = G::Map;
        fn visit_map(&self) -> Self::Map {
            self.g.visit_map()
        }
        fn reset_map(&self, map: &mut Self::Map) {
            self.g.reset_map(map);
        }
    }

    impl<'a, I> Iterator for EvalFilterNeighbors<'a, I>
    where
        I: Iterator,
        I::Item: Eq + Hash,
    {
        type Item = I::Item;
        fn next(&mut self) -> Option<Self::Item> {
            while let Some(n) = self.neighbors.next() {
                if !self.is_src || self.src_neighbors.contains(&n) {
                    return Some(n);
                }
            }
            None
        }
    }

    impl<'a, G> IntoNeighbors for EvalFilter<'a, G>
    where
        G: IntoNeighbors,
        G::NodeId: Eq + Hash,
    {
        type Neighbors = EvalFilterNeighbors<'a, G::Neighbors>;
        fn neighbors(self, a: Self::NodeId) -> Self::Neighbors {
            let neighbors = self.g.neighbors(a);
            EvalFilterNeighbors {
                neighbors,
                is_src: self.src == a,
                src_neighbors: &self.src_neighbors,
            }
        }
    }

    // Wrap the graph in a filter to only include src neighbors that appear in
    // the eval set.
    let g = EvalFilter {
        g,
        src,
        src_neighbors,
    };

    Dfs::new(g, src).iter(g)
}

/// An iterator yielding all nodes reachable via pushing from the given node
/// over the given set of direct neighbors.
fn push_reachable<G>(
    g: G,
    n: G::NodeId,
    nbs: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
    G::NodeId: Eq + Hash,
{
    reachable(g, n, nbs)
}

/// An iterator yielding all nodes reachable via pulling from the given node
/// over the given set of direct neighbors.
fn pull_reachable<G>(
    g: G,
    n: G::NodeId,
    nbs: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + Visitable,
    G::NodeId: Eq + Hash,
{
    let rev_g = petgraph::visit::Reversed(g);
    reachable(rev_g, n, nbs)
}

/// Push evaluation from the specified node.
///
/// Evaluation order is equivalent to a topological ordering of the connected
/// component starting from the given node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes
/// implement `Node`. Direction of edges indicate the flow of data through the
/// graph.
fn push_eval_order<G>(
    g: G,
    n: G::NodeId,
    nbs: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + IntoNodeReferences + Visitable,
    G::NodeId: Eq + Hash,
{
    let dfs: HashSet<G::NodeId> = push_reachable(g, n, nbs).collect();
    Topo::new(g).iter(g).filter(move |node| dfs.contains(&node))
}

/// Pull evaluation from the specified node.
///
/// Evaluation order is equivalent to a topological ordering of the connected
/// component that ends at the given node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes
/// implement `Node`. Direction of edges indicate the flow of data through the
/// graph.
fn pull_eval_order<G>(
    g: G,
    n: G::NodeId,
    nbs: &HashSet<G::NodeId>,
) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + IntoNodeReferences + Visitable,
    G::NodeId: Eq + Hash,
{
    let dfs: HashSet<G::NodeId> = pull_reachable(g, n, nbs).collect();
    Topo::new(g).iter(g).filter(move |node| dfs.contains(&node))
}

/// The evaluation order given any number of simultaneously pushing and pulling
/// nodes.
///
/// Evaluation order is equivalent to a topological ordering of the connected
/// components reachable via DFS from each push node and reversed-edge DFS from
/// each pull node.
///
/// Expects any directed graph whose edges are of type `Edge` and whose nodes
/// implement `Node`. Direction of edges indicate the flow of data through the
/// graph.
pub(crate) fn eval_order<G, A, B>(g: G, push: A, pull: B) -> impl Iterator<Item = G::NodeId>
where
    G: IntoEdgesDirected + IntoNodeReferences + Visitable,
    G::EdgeWeight: Edges,
    G::NodeId: Eq + Hash,
    A: IntoIterator<Item = (G::NodeId, node::Conns)>,
    B: IntoIterator<Item = (G::NodeId, node::Conns)>,
{
    let mut reachable = HashSet::new();
    reachable.extend(push.into_iter().flat_map(|(n, conns)| {
        let ps = push_eval_neighbors(g, n, &conns);
        push_reachable(g, n, &ps).collect::<Vec<_>>()
    }));
    reachable.extend(pull.into_iter().flat_map(|(n, conns)| {
        let pl = pull_eval_neighbors(g, n, &conns);
        pull_reachable(g, n, &pl).collect::<Vec<_>>()
    }));
    Topo::new(g).iter(g).filter(move |n| reachable.contains(&n))
}

/// Given a node evaluation order, produce the series of evaluation steps
/// required.
pub(crate) fn eval_steps<I>(meta: &Meta, eval_order: I) -> impl Iterator<Item = EvalStep>
where
    I: IntoIterator<Item = node::Id>,
{
    // Step through each of the nodes.
    let mut visited = HashSet::new();
    eval_order.into_iter().map(move |n| {
        visited.insert(n);

        // Collect the inputs, initialising the set to `None`.
        let n_inputs = meta.inputs.get(&n).copied().unwrap_or(0);
        let mut inputs: Vec<_> = (0..n_inputs).map(|_| None).collect();
        for e_ref in meta.graph.edges_directed(n, petgraph::Incoming) {
            // Only consider edges to nodes that we have already visited.
            if !visited.contains(&e_ref.source()) {
                continue;
            }
            for (edge, _kind) in e_ref.weight() {
                // Assign the expression argument for this input.
                let arg = ExprInput {
                    node: e_ref.source(),
                    output: edge.output,
                };
                inputs[edge.input.0 as usize] = Some(arg);
            }
        }

        // Collect the set of connected outputs.
        let n_outputs = meta.outputs.get(&n).copied().unwrap_or(0);
        let mut outputs: Vec<_> = (0..n_outputs).map(|_| false).collect();
        for e_ref in meta.graph.edges_directed(n, petgraph::Outgoing) {
            for (edge, _kind) in e_ref.weight() {
                outputs[edge.output.0 as usize] |= true;
            }
        }

        EvalStep {
            node: n,
            inputs,
            outputs,
        }
    })
}

/// Create the evaluation plan for the graph associated with the given meta.
fn eval_plan(meta: &Meta) -> EvalPlan {
    let pull_steps = meta
        .pull
        .iter()
        .flat_map(|(&n, confs)| {
            confs.iter().map(move |conns| {
                let nbs = pull_eval_neighbors(&meta.graph, n, conns);
                let order = pull_eval_order(&meta.graph, n, &nbs);
                let steps = eval_steps(meta, order).collect();
                (n, steps)
            })
        })
        .collect();

    let push_steps = meta
        .push
        .iter()
        .flat_map(|(&n, confs)| {
            confs.iter().map(move |conns| {
                let nbs = push_eval_neighbors(&meta.graph, n, conns);
                let order = push_eval_order(&meta.graph, n, &nbs);
                let steps = eval_steps(meta, order).collect();
                (n, steps)
            })
        })
        .collect();

    let nested_steps = {
        let order = eval_order(
            &meta.graph,
            // FIXME: shouldn't hardcode these `Conns` counts...
            meta.inlets
                .iter()
                .map(|&n| (n, node::Conns::connected(1).unwrap())),
            meta.outlets
                .iter()
                .map(|&n| (n, node::Conns::connected(1).unwrap())),
        );
        eval_steps(meta, order).collect()
    };

    EvalPlan {
        meta,
        push_steps,
        pull_steps,
        nested_steps,
    }
}

/// Given a root gantz graph, generate the full module with all the necessary
/// functions for executing it.
///
/// This includes:
///
/// 1. A function for each node (and for each node input configuration).
/// 2. A function for each node requiring push/pull evaluation.
/// 3. The above for all nested graphs.
pub fn module<G>(g: G) -> Vec<ExprKind>
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    // Create a `Meta` for each graph (including nested) in a tree.
    let mut meta_tree = RoseTree::<Meta>::default();
    crate::graph::visit(g, &[], &mut meta_tree);
    let eval_tree = meta_tree.map_ref(&mut eval_plan);

    // Collect node fns.
    let node_confs_tree = codegen::node_confs_tree(&eval_tree);
    let node_fns = codegen::node_fns(g, &node_confs_tree);

    // Collect eval fns.
    let eval_fns = codegen::eval_fns(&eval_tree);

    node_fns.into_iter().chain(eval_fns).collect()
}
