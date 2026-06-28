//! The gantz content-address implementation for graphs.

pub use crate::{
    ContentAddr, content_addr,
    hash::{CaHash, Hasher},
};
use petgraph::visit::{Data, EdgeRef, IntoEdgeReferences, IntoNodeReferences, NodeRef};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt, hash::Hash, ops};

/// The content address of a graph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub struct GraphAddr(ContentAddr);

impl ops::Deref for GraphAddr {
    type Target = ContentAddr;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<ContentAddr> for GraphAddr {
    fn from(ca: ContentAddr) -> Self {
        Self(ca)
    }
}

impl From<GraphAddr> for ContentAddr {
    fn from(addr: GraphAddr) -> Self {
        addr.0
    }
}

impl CaHash for GraphAddr {
    fn hash(&self, hasher: &mut Hasher) {
        CaHash::hash(&self.0, hasher);
    }
}

impl fmt::Display for GraphAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Calculate the content address of a graph.
///
/// The address depends on graph *structure*, not on the physical node-index
/// layout: each node is hashed by its canonical rank (its position in
/// ascending-index order) rather than its raw index. For a hole-free graph the
/// rank equals the raw index, so addresses are unchanged; vacant slots left by
/// node removals (`StableGraph` "holes") are compacted away, keeping the address
/// stable across a `gantz_format` round-trip (which cannot reproduce holes).
///
/// Raw indices remain meaningful at *runtime* - they key node state and appear
/// in generated expressions - but they no longer leak into the address.
///
/// ## Approach
///
/// 1. Rank each node by its position in ascending-index order.
/// 2. For each node (in rank order):
///     - hash its rank (u64, big-endian).
///     - hash the content address of the node.
/// 3. Collect all edges (rank-src, rank-dst, edge-weight).
/// 4. Sort the edges.
/// 5. For each edge:
///     - hash the source node rank (u64, big-endian).
///     - hash the target node rank (u64, big-endian).
///     - hash the edge weight (source output + target input).
pub fn addr<G>(g: G) -> GraphAddr
where
    G: Data + IntoEdgeReferences + IntoNodeReferences,
    G::NodeId: Eq + Hash + Ord,
    G::EdgeWeight: CaHash + Ord,
    G::NodeWeight: CaHash,
{
    let mut hasher = Hasher::new();
    hash_graph(g, &mut hasher);
    GraphAddr(ContentAddr(hasher.finalize().into()))
}

/// A more efficient alternative to [`addr`] for when the node content
/// addresses are already known.
pub fn addr_with_nodes<G>(g: G, nodes: &HashMap<G::NodeId, ContentAddr>) -> GraphAddr
where
    G: Data + IntoEdgeReferences + IntoNodeReferences,
    G::NodeId: Hash + Ord,
    G::EdgeWeight: CaHash + Ord,
{
    let mut hasher = Hasher::new();
    hash_graph_with_nodes(g, nodes, &mut hasher);
    GraphAddr(ContentAddr(hasher.finalize().into()))
}

/// The implementation of [`addr`] with hasher provided.
pub fn hash_graph<G>(g: G, hasher: &mut Hasher)
where
    G: Data + IntoEdgeReferences + IntoNodeReferences,
    G::NodeId: Eq + Hash + Ord,
    G::EdgeWeight: CaHash + Ord,
    G::NodeWeight: CaHash,
{
    let nodes = node_addrs(g);
    hash_graph_with_nodes(g, &nodes, hasher);
}

/// The implementation of [`addr_with_nodes`] with hasher provided.
pub fn hash_graph_with_nodes<G>(g: G, nodes: &HashMap<G::NodeId, ContentAddr>, hasher: &mut Hasher)
where
    G: Data + IntoEdgeReferences + IntoNodeReferences,
    G::NodeId: Hash + Ord,
    G::EdgeWeight: CaHash + Ord,
{
    // Assign each node a canonical rank: its position in ascending-index order
    // (the order `node_references` yields for a `StableGraph`). For a hole-free
    // graph the rank equals the raw index, so existing addresses are unchanged;
    // vacant slots left by node removals are compacted away, making the address
    // independent of the physical slot layout and stable across a round-trip.
    let rank: HashMap<G::NodeId, u64> = g
        .node_references()
        .enumerate()
        .map(|(i, n_ref)| (n_ref.id(), i as u64))
        .collect();

    // Hash all nodes in rank order.
    for n_ref in g.node_references() {
        let id = n_ref.id();
        let node_ca = &nodes[&id];
        CaHash::hash(&rank[&id], hasher);
        CaHash::hash(&**node_ca, hasher);
    }

    // Collect and sort edges by (source rank, target rank, edge weight). Since
    // edge indices don't matter, we put them in an edge-index-agnostic
    // deterministic order.
    let mut edges = vec![];
    for e_ref in g.edge_references() {
        let src = rank[&e_ref.source()];
        let dst = rank[&e_ref.target()];
        edges.push((src, dst, e_ref));
    }
    edges.sort_by(|(sa, da, ea), (sb, db, eb)| (sa, da, ea.weight()).cmp(&(sb, db, eb.weight())));

    // Hash all edges as (src rank, dst rank, edge weight).
    for (src, dst, e_ref) in edges {
        CaHash::hash(&src, hasher);
        CaHash::hash(&dst, hasher);
        CaHash::hash(e_ref.weight(), hasher);
    }
}

/// Hash all the nodes and return a map from node IDs to their content addresses.
pub fn node_addrs<G>(g: G) -> HashMap<G::NodeId, ContentAddr>
where
    G: Data + IntoNodeReferences,
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

#[cfg(test)]
mod tests {
    use super::addr;
    use petgraph::{Directed, stable_graph::StableGraph};

    type G = StableGraph<u32, u32, Directed, usize>;

    /// The address must ignore `StableGraph` "holes": an edited graph (whose
    /// node removals leave vacant index slots) and its compacted form (as
    /// produced by a `gantz_format` round-trip, which cannot reproduce holes)
    /// must share an address.
    #[test]
    fn addr_is_stable_across_hole_compaction() {
        // Holey: four nodes wired up, then an interior node removed - leaving a
        // vacant slot at index 1 and surviving nodes at indices 0, 2, 3.
        let mut holey = G::default();
        let h0 = holey.add_node(10);
        let h1 = holey.add_node(20);
        let h2 = holey.add_node(30);
        let h3 = holey.add_node(40);
        holey.add_edge(h0, h2, 0);
        holey.add_edge(h2, h3, 1);
        holey.add_edge(h0, h1, 2); // dropped along with h1
        holey.remove_node(h1);

        // Compacted: the same surviving structure with contiguous indices.
        let mut compact = G::default();
        let c0 = compact.add_node(10);
        let c1 = compact.add_node(30);
        let c2 = compact.add_node(40);
        compact.add_edge(c0, c1, 0);
        compact.add_edge(c1, c2, 1);

        assert_eq!(addr(&holey), addr(&compact));
    }

    /// The address is deterministic and remains sensitive to structure (so the
    /// canonical-rank scheme didn't collapse genuinely distinct graphs).
    #[test]
    fn addr_is_deterministic_and_structure_sensitive() {
        let build = |rev: bool| {
            let mut g = G::default();
            let n0 = g.add_node(1);
            let n1 = g.add_node(2);
            if rev {
                g.add_edge(n1, n0, 0);
            } else {
                g.add_edge(n0, n1, 0);
            }
            g
        };
        assert_eq!(addr(&build(false)), addr(&build(false)));
        assert_ne!(addr(&build(false)), addr(&build(true)));
    }
}
