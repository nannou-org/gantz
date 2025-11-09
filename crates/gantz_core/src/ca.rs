//! The gantz content-address implementation for graphs.

use crate::compile::Edges;
#[doc(inline)]
pub use gantz_ca::*;
use petgraph::visit::{
    Data, EdgeRef, IntoEdgeReferences, IntoNodeReferences, NodeIndexable, NodeRef,
};
use std::{collections::HashMap, hash::Hash};

/// Calculate the content address of a graph.
///
/// Note that some graphs with the same structure may result in different CAs.
/// This is because indices are (currently) meaningful, as they're provided to
/// nodes when registering state and generating expressions.
///
/// ## Approach
///
/// 1. For each node:
///     - hash its index (u64, big-endian).
///     - hash the content address of the node.
/// 2. Collect all edges (src-node-ix, dst-node-ix, src-output, dst-input).
/// 3. Sort the edges.
/// 4. For each edge:
///     - hash the source node index (u64, big-endian).
///     - hash the target node index (u64, big-endian).
///     - hash the source node's output (u16, big-endian).
///     - hash the target node's input (u16, big-endian).
pub fn graph<G>(g: G) -> ContentAddr
where
    G: Data + IntoEdgeReferences + IntoNodeReferences + NodeIndexable,
    G::NodeId: Eq + Hash + Ord,
    G::EdgeWeight: Edges,
    G::NodeWeight: CaHash,
{
    let mut hasher = Hasher::new();
    hash_graph(g, &mut hasher);
    ContentAddr(hasher.finalize().into())
}

/// A more efficient alternative to [`graph`] for when the node content
/// addresses are already known.
pub fn graph_with_nodes<G>(g: G, nodes: &HashMap<G::NodeId, ContentAddr>) -> ContentAddr
where
    G: Data + IntoEdgeReferences + IntoNodeReferences + NodeIndexable,
    G::NodeId: Hash + Ord,
    G::EdgeWeight: Edges,
{
    let mut hasher = Hasher::new();
    hash_graph_with_nodes(g, nodes, &mut hasher);
    ContentAddr(hasher.finalize().into())
}

/// The implementation of [`graph`] with hasher provided.
pub fn hash_graph<G>(g: G, hasher: &mut Hasher)
where
    G: Data + IntoEdgeReferences + IntoNodeReferences + NodeIndexable,
    G::NodeId: Eq + Hash + Ord,
    G::EdgeWeight: Edges,
    G::NodeWeight: CaHash,
{
    let nodes = nodes(g);
    hash_graph_with_nodes(g, &nodes, hasher);
}

/// The implementation of [`graph_with_nodes`] with hasher provided.
pub fn hash_graph_with_nodes<G>(g: G, nodes: &HashMap<G::NodeId, ContentAddr>, hasher: &mut Hasher)
where
    G: Data + IntoEdgeReferences + IntoNodeReferences + NodeIndexable,
    G::NodeId: Hash + Ord,
    G::EdgeWeight: Edges,
{
    const OUT_OF_RANGE: &str = "graph node index exceeds u64::MAX";

    // Hash all nodes in index order (indices are meaningful).
    for n_ref in g.node_references() {
        let id = n_ref.id();
        let node_ca = &nodes[&id];
        let ix: u64 = g.to_index(id).try_into().expect(OUT_OF_RANGE);
        CaHash::hash(&ix, hasher);
        CaHash::hash(&**node_ca, hasher);
    }

    // Collect and sort edges by (source, target, edge_data).
    // Since edge indices don't matter, we can put them in an
    // edge-index-agnostic deterministic order.
    let mut edges = vec![];
    for e_ref in g.edge_references() {
        let src: u64 = g.to_index(e_ref.source()).try_into().expect(OUT_OF_RANGE);
        let dst: u64 = g.to_index(e_ref.target()).try_into().expect(OUT_OF_RANGE);
        for edge in e_ref.weight().edges() {
            edges.push((src, dst, edge));
        }
    }
    edges.sort();

    // Hash all edges as (src, dst, src-output, dst-input).
    for (src, dst, edge) in edges {
        CaHash::hash(&src, hasher);
        CaHash::hash(&dst, hasher);
        CaHash::hash(&edge.output.0, hasher);
        CaHash::hash(&edge.input.0, hasher);
    }
}

/// Hash all the nodes and return a map from node IDs to their content addresses.
pub fn nodes<G>(g: G) -> HashMap<G::NodeId, ContentAddr>
where
    G: Data + IntoNodeReferences + NodeIndexable,
    G::NodeId: Eq + std::hash::Hash,
    G::NodeWeight: CaHash,
{
    g.node_references()
        .map(|n_ref| {
            let id = n_ref.id();
            let ca = content_addr(n_ref.weight());
            (id, ca)
        })
        .collect()
}
