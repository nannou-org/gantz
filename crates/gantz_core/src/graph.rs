//! Provides [`visit()`] and [`register()`] fns for generic gantz graphs.

use std::{
    collections::HashSet,
    hash::{Hash, Hasher},
};

use crate::{
    Edge,
    node::{self, Node},
    visit,
};
use petgraph::visit::{
    Data, EdgeRef, IntoEdgeReferences, IntoEdgesDirected, IntoNodeReferences, NodeIndexable,
    NodeRef, Topo, Visitable,
};
use steel::steel_vm::engine::Engine;

/// Hash the nodes, edges and their IDs of the given graph.
pub fn hash<G, H>(g: G, h: &mut H)
where
    G: Data + IntoEdgeReferences + IntoNodeReferences,
    G::EdgeId: Hash,
    G::EdgeWeight: Hash,
    G::NodeId: Hash,
    G::NodeWeight: Hash,
    H: Hasher,
{
    for n in g.node_references() {
        n.id().hash(h);
        n.weight().hash(h);
    }
    for e in g.edge_references() {
        e.id().hash(h);
        e.weight().hash(h);
    }
}

/// Visit all nodes in the graph in toposort order, and all nested nodes in
/// depth-first order.
pub fn visit<'a, G>(
    get_node: node::GetNode<'a>,
    g: G,
    path: &[node::Id],
    visitor: &mut dyn node::Visitor,
) where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    let mut path = path.to_vec();
    let mut topo = Topo::new(g);
    while let Some(n) = topo.next(g) {
        let ix = g.to_index(n);
        path.push(ix);
        let inputs: Vec<_> = g
            .edges_directed(n, petgraph::Direction::Incoming)
            .map(|e_ref| (g.to_index(e_ref.source()), e_ref.weight().clone()))
            .collect();
        let ctx = visit::Ctx::new(get_node, &path, &inputs);

        // FIXME: index directly.
        let nref = g.node_references().find(|nref| nref.id() == n).unwrap();

        node::visit(ctx, nref.weight(), visitor);
        path.pop();
    }
}

/// Register the given graph of nodes, including any nested nodes.
pub fn register<'a, G>(get_node: node::GetNode<'a>, g: G, path: &[node::Id], vm: &mut Engine)
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    visit(get_node, g, path, &mut visit::Register(vm));
}

/// Collect all content addresses required by nodes in this graph.
pub fn required_addrs<'a, G>(get_node: node::GetNode<'a>, g: G) -> HashSet<gantz_ca::ContentAddr>
where
    G: Data<EdgeWeight = Edge> + IntoEdgesDirected + IntoNodeReferences + NodeIndexable + Visitable,
    G::NodeWeight: Node,
{
    let mut addrs = HashSet::new();
    visit(
        get_node,
        g,
        &[],
        &mut visit::RequiredAddrs { addrs: &mut addrs },
    );
    addrs
}
