//! Provides [`visit()`] and [`register()`] fns for generic gantz graphs.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
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

/// Extract a subgraph containing only the selected nodes and edges between them.
///
/// Nodes are visited in index order for determinism. Callers that need to
/// correlate old/new indices can iterate selected nodes in the same sorted
/// order alongside `subgraph.node_indices()`.
pub fn extract_subgraph<N: Clone>(
    graph: &node::graph::Graph<N>,
    selected: &HashSet<node::graph::NodeIx>,
) -> node::graph::Graph<N> {
    let mut subgraph = node::graph::Graph::default();
    let sorted: BTreeSet<_> = selected.iter().copied().collect();
    let mut ix_map = HashMap::new();
    for old_ix in &sorted {
        let weight = graph[*old_ix].clone();
        let new_ix = subgraph.add_node(weight);
        ix_map.insert(*old_ix, new_ix);
    }
    for e in graph.edge_references() {
        let src = e.source();
        let tgt = e.target();
        if let (Some(&new_src), Some(&new_tgt)) = (ix_map.get(&src), ix_map.get(&tgt)) {
            subgraph.add_edge(new_src, new_tgt, e.weight().clone());
        }
    }
    subgraph
}

/// Add all nodes and edges from `subgraph` into `target`.
///
/// Returns the new node indices in `target` in the same order as
/// `subgraph.node_indices()`, so callers can correlate them with positions.
pub fn add_subgraph<N: Clone>(
    target: &mut node::graph::Graph<N>,
    subgraph: &node::graph::Graph<N>,
) -> Vec<node::graph::NodeIx> {
    let mut ix_map = HashMap::new();
    let mut new_indices = Vec::new();
    for n in subgraph.node_indices() {
        let weight = subgraph[n].clone();
        let new_ix = target.add_node(weight);
        ix_map.insert(n, new_ix);
        new_indices.push(new_ix);
    }
    for e in subgraph.edge_references() {
        let new_src = ix_map[&e.source()];
        let new_tgt = ix_map[&e.target()];
        target.add_edge(new_src, new_tgt, e.weight().clone());
    }
    new_indices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Edge;

    /// Build a diamond graph: A -> B, A -> C, B -> D, C -> D.
    fn diamond_graph() -> (node::graph::Graph<&'static str>, [node::graph::NodeIx; 4]) {
        let mut g = node::graph::Graph::default();
        let a = g.add_node("A");
        let b = g.add_node("B");
        let c = g.add_node("C");
        let d = g.add_node("D");
        g.add_edge(a, b, Edge::new(0.into(), 0.into()));
        g.add_edge(a, c, Edge::new(0.into(), 0.into()));
        g.add_edge(b, d, Edge::new(0.into(), 0.into()));
        g.add_edge(c, d, Edge::new(0.into(), 1.into()));
        (g, [a, b, c, d])
    }

    #[test]
    fn extract_subgraph_basic() {
        let (g, [_a, b, c, d]) = diamond_graph();
        let selected: HashSet<_> = [b, c, d].into_iter().collect();
        let sub = extract_subgraph(&g, &selected);
        assert_eq!(sub.node_count(), 3);
        // Only edges where both endpoints are selected: B->D, C->D.
        assert_eq!(sub.edge_count(), 2);
        // Weights preserved.
        let weights: Vec<_> = sub.node_indices().map(|n| sub[n]).collect();
        assert_eq!(weights, vec!["B", "C", "D"]);
    }

    #[test]
    fn extract_subgraph_excludes_external_edges() {
        let (g, [a, _b, _c, d]) = diamond_graph();
        // Select only A and D — no direct edge between them.
        let selected: HashSet<_> = [a, d].into_iter().collect();
        let sub = extract_subgraph(&g, &selected);
        assert_eq!(sub.node_count(), 2);
        assert_eq!(sub.edge_count(), 0);
    }

    #[test]
    fn extract_subgraph_empty_selection() {
        let (g, _) = diamond_graph();
        let sub = extract_subgraph(&g, &HashSet::new());
        assert_eq!(sub.node_count(), 0);
        assert_eq!(sub.edge_count(), 0);
    }

    #[test]
    fn add_subgraph_returns_correct_indices() {
        let mut target = node::graph::Graph::<&str>::default();
        let _existing = target.add_node("X");

        let mut sub = node::graph::Graph::default();
        let sa = sub.add_node("A");
        let sb = sub.add_node("B");
        sub.add_edge(sa, sb, Edge::new(0.into(), 0.into()));

        let new = add_subgraph(&mut target, &sub);
        assert_eq!(new.len(), 2);
        assert_eq!(target.node_count(), 3);
        assert_eq!(target.edge_count(), 1);
        // New indices should be after the existing node.
        assert_eq!(target[new[0]], "A");
        assert_eq!(target[new[1]], "B");
        // Edge should connect the new nodes.
        assert!(target.find_edge(new[0], new[1]).is_some());
    }
}
