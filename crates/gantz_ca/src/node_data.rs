//! The registry's erased node representation. A self-describing value plus
//! its structural content references.

use crate::edge::Edge;
use crate::{CaHash, ContentAddr, Datum, Hasher, SectionId};
use serde::{Deserialize, Serialize};

/// A graph of erased nodes. This is the registry's stored graph
/// representation.
///
/// It has the same petgraph shape as `gantz_core`'s typed working graph, with
/// node weights erased to [`NodeData`]. The graph is plain, self-describing
/// data.
pub type DataGraph = petgraph::graph::Graph<NodeData, Edge, petgraph::Directed, usize>;

/// One erased node. A self-describing value plus its structural references.
///
/// - `tag` is the node type's wire tag, `gantz_nodetag::NodeTag::TAG`. It
///   identifies how to interpret `data`.
/// - `data` is the node's field datum. This is the tagged map produced by
///   node-set serde minus its `"type"` entry.
/// - `refs` and `blobs` are the node's outgoing content references. They are
///   extracted from `Node::required_addrs` and `Node::required_blobs` when a
///   typed node is erased. They are stored structurally and covered by the
///   node's address. Reachability for liveness, export and sync want-lists
///   is then a pure data walk. Any peer can compute it and re-verify content
///   addresses without the node type compiled in.
///
/// Identity-sensitive contexts such as content addressing and sync staging
/// require the canonical form so that one logical node has exactly one
/// address. See [`NodeData::canonicalize`].
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct NodeData {
    /// The node type's wire tag.
    pub tag: String,
    /// The node's fields as a self-describing value.
    pub data: Datum,
    /// Content addresses of the graphs and nodes this node references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<ContentAddr>,
    /// The blobs this node references, tagged with their blob section.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blobs: Vec<(SectionId, ContentAddr)>,
}

impl NodeData {
    /// A node with the given tag and field datum and no outgoing references.
    pub fn new(tag: impl Into<String>, data: Datum) -> Self {
        Self {
            tag: tag.into(),
            data,
            refs: vec![],
            blobs: vec![],
        }
    }

    /// Canonical form. `data` is canonicalized via [`Datum::canonicalize`].
    /// `refs` and `blobs` are sorted and deduplicated.
    pub fn canonicalize(&mut self) {
        self.data.canonicalize();
        self.refs.sort();
        self.refs.dedup();
        self.blobs.sort();
        self.blobs.dedup();
    }

    /// Whether `self` is already in canonical form.
    pub fn is_canonical(&self) -> bool {
        let mut c = self.clone();
        c.canonicalize();
        *self == c
    }

    /// The node's content address.
    ///
    /// Assumes `self` is canonical. See [`NodeData::canonicalize`].
    /// Non-canonical forms of the same logical node produce different
    /// addresses.
    pub fn content_addr(&self) -> ContentAddr {
        crate::content_addr(self)
    }
}

impl CaHash for NodeData {
    /// Content-address folding. A `gantz.node` domain prefix, the
    /// length-prefixed tag, the self-delimiting [`Datum`] fold, then the
    /// length-prefixed `refs` and `blobs` columns. Length prefixes keep the
    /// variable-size parts from colliding through adjacency.
    fn hash(&self, hasher: &mut Hasher) {
        fn len(hasher: &mut Hasher, n: usize) {
            hasher.update(&(n as u64).to_be_bytes());
        }
        hasher.update(b"gantz.node");
        len(hasher, self.tag.len());
        hasher.update(self.tag.as_bytes());
        self.data.hash(hasher);
        // `Vec<ContentAddr>` folds a length prefix then fixed-size addresses.
        self.refs.hash(hasher);
        len(hasher, self.blobs.len());
        for (section, addr) in &self.blobs {
            len(hasher, section.len());
            hasher.update(section.as_bytes());
            addr.hash(hasher);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_addr;

    fn addr(byte: u8) -> ContentAddr {
        ContentAddr([byte; 32])
    }

    fn node(tag: &str) -> NodeData {
        NodeData::new(tag, Datum::Map(vec![("x".to_string(), Datum::I64(1))]))
    }

    /// Distinct nodes fold to distinct addresses. The length prefixes prevent
    /// tag, data, refs and blobs content from blurring through adjacency.
    #[test]
    fn ca_hash_distinctness() {
        // Tag vs data boundary.
        let a = NodeData::new("ab", Datum::Str("c".into()));
        let b = NodeData::new("a", Datum::Str("bc".into()));
        assert_ne!(content_addr(&a), content_addr(&b));
        // Refs participate in the address.
        let mut with_ref = node("t");
        with_ref.refs.push(addr(1));
        assert_ne!(content_addr(&node("t")), content_addr(&with_ref));
        // Blobs participate, and the section/addr boundary is unambiguous.
        let mut b1 = node("t");
        b1.blobs.push(("ab".to_string(), addr(2)));
        let mut b2 = node("t");
        b2.blobs.push(("a".to_string(), addr(2)));
        assert_ne!(content_addr(&b1), content_addr(&b2));
        assert_ne!(content_addr(&node("t")), content_addr(&b1));
        // A ref and a blob with the same addr are distinct content.
        let mut r = node("t");
        r.refs.push(addr(3));
        let mut bl = node("t");
        bl.blobs.push((String::new(), addr(3)));
        assert_ne!(content_addr(&r), content_addr(&bl));
    }

    /// Canonicalization sorts and dedupes the ref columns and canonicalizes
    /// the datum, and only the canonical form is address-stable.
    #[test]
    fn canonicalize_normalizes() {
        let mut n = NodeData::new(
            "t",
            Datum::Map(vec![
                ("b".to_string(), Datum::Null),
                ("a".to_string(), Datum::Bool(true)),
            ]),
        );
        n.refs = vec![addr(2), addr(1), addr(2)];
        n.blobs = vec![("s".to_string(), addr(9)), ("s".to_string(), addr(9))];
        assert!(!n.is_canonical());
        let non_canonical_addr = content_addr(&n);
        n.canonicalize();
        assert!(n.is_canonical());
        assert_eq!(n.refs, vec![addr(1), addr(2)]);
        assert_eq!(n.blobs, vec![("s".to_string(), addr(9))]);
        assert_ne!(content_addr(&n), non_canonical_addr);
        // Canonicalization is idempotent.
        let once = n.clone();
        n.canonicalize();
        assert_eq!(n, once);
    }

    /// Pin the fold so accidental scheme changes are caught. Node addresses
    /// are part of the wire format.
    #[test]
    fn ca_hash_stability_pin() {
        let mut n = NodeData::new(
            "test",
            Datum::Map(vec![
                ("flag".to_string(), Datum::Bool(true)),
                ("ratio".to_string(), Datum::F64(1.5)),
            ]),
        );
        n.refs = vec![addr(1)];
        n.blobs = vec![("dsp.buffer".to_string(), addr(2))];
        assert!(n.is_canonical());
        assert_eq!(
            n.content_addr().to_string(),
            "e89011c7a8461d3ec787c95641617e7cdcc9265d449ebc4bfecb3c1201721a82",
            "NodeData CaHash scheme changed - this breaks existing node addresses",
        );
    }

    /// `DataGraph` satisfies the structural-hashing bounds. The canonical-rank
    /// graph addressing works over erased nodes.
    #[test]
    fn data_graph_addr_smoke() {
        let mut g = DataGraph::default();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        g.add_edge(a, b, Edge::from((0, 0)));
        let _ = crate::graph_addr(&g);
        let g2 = g.clone();
        assert_eq!(crate::graph_addr(&g), crate::graph_addr(&g2));
    }
}
