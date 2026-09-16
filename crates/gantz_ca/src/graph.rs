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
/// The address depends on graph structure, not on the physical node-index
/// layout. Each node is hashed by its canonical rank, its position in
/// ascending-index order, rather than its raw index. For a hole-free graph
/// the rank equals the raw index. Vacant slots left by node removals in a
/// `StableGraph` are compacted away. This keeps the address stable across a
/// `gantz_format` round-trip, which cannot reproduce holes.
///
/// Raw indices remain meaningful at runtime. They key node state and appear
/// in generated expressions. They do not leak into the address.
///
/// Nodes are hashed in rank order as the pair of rank and node address. Edges
/// are then sorted and hashed as source rank, target rank and weight.
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

/// The implementation of [`addr`] with hasher provided.
fn hash_graph<G>(g: G, hasher: &mut Hasher)
where
    G: Data + IntoEdgeReferences + IntoNodeReferences,
    G::NodeId: Eq + Hash + Ord,
    G::EdgeWeight: CaHash + Ord,
    G::NodeWeight: CaHash,
{
    // Domain-separate graph addresses from every other kind. `Commit`'s
    // `CaHash` has a matching prefix.
    hasher.update(b"gantz.graph");
    // Rank each node by its position in ascending-index order, the order
    // `node_references` yields for a `StableGraph`.
    let rank: HashMap<G::NodeId, u64> = g
        .node_references()
        .enumerate()
        .map(|(i, n_ref)| (n_ref.id(), i as u64))
        .collect();

    for n_ref in g.node_references() {
        let id = n_ref.id();
        let node_ca = content_addr(n_ref.weight());
        CaHash::hash(&rank[&id], hasher);
        CaHash::hash(&*node_ca, hasher);
    }

    // Sort edges by source rank, target rank and weight so that edge indices
    // do not affect the address.
    let mut edges = vec![];
    for e_ref in g.edge_references() {
        let src = rank[&e_ref.source()];
        let dst = rank[&e_ref.target()];
        edges.push((src, dst, e_ref));
    }
    edges.sort_by(|(sa, da, ea), (sb, db, eb)| (sa, da, ea.weight()).cmp(&(sb, db, eb.weight())));

    for (src, dst, e_ref) in edges {
        CaHash::hash(&src, hasher);
        CaHash::hash(&dst, hasher);
        CaHash::hash(e_ref.weight(), hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::addr;
    use petgraph::{Directed, stable_graph::StableGraph};

    type G = StableGraph<u32, u32, Directed, usize>;

    /// The address must ignore `StableGraph` holes. An edited graph with
    /// vacant index slots and its compacted form must share an address.
    #[test]
    fn addr_is_stable_across_hole_compaction() {
        // Four nodes wired up, then an interior node removed. This leaves a
        // vacant slot at index 1 and surviving nodes at indices 0, 2 and 3.
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

    /// The address is deterministic and sensitive to structure. The
    /// canonical-rank scheme must not collapse distinct graphs.
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
