//! The codec between typed nodes and the registry's erased [`NodeData`] and
//! [`DataGraph`] representation, plus the reified-graph cache.
//!
//! The registry stores graphs as plain data. Typed nodes cross that boundary
//! here, one concrete type at a time. [`erase_node_typed`] erases a node
//! under its own [`NodeTag`](gantz_nodetag::NodeTag), and
//! [`reify_node_concrete`] decodes stored fields back at a concrete type. A
//! node set picks the type by [`NodeData::tag`] and passes that dispatch as
//! the `reify_node` fn of [`reify_with`] and [`ReifiedGraphs`].
//! [`edit_node_typed`] edits a stored node in place and keeps the data that
//! this build does not recognise.

use crate::node::graph::Graph;
use crate::node::{self, Node};
use crate::visit;
use gantz_ca::{
    ContentAddr, DataGraph, Datum, DatumError, GraphAddr, NodeData, Registry, SectionId, datum,
};
use petgraph::visit::EdgeRef;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

/// An append-only cache of reified registry graphs, keyed by graph address.
///
/// Content addressing makes entries immutable. An address names exactly one
/// graph forever, so the cache never invalidates. For the same reason a graph
/// that fails to reify into the cache's node set fails for good, so
/// [`ReifiedGraphs::ensure_all_with`] records it and does not retry it.
/// [`ReifiedGraphs::retain_live`] may drop entries to bound memory after a
/// prune.
///
/// Intended use is two-phase. First [`ReifiedGraphs::ensure_with`] everything a
/// pass can reach, which requires `&mut self`. Then serve the whole pass
/// immutably through [`ReifiedGraphs::get`] borrows, for example behind a
/// `GetNode` closure.
#[derive(Debug)]
pub struct ReifiedGraphs<N> {
    graphs: HashMap<GraphAddr, Graph<N>>,
    /// Graphs that failed to reify. See [`ReifiedGraphs::ensure_all_with`].
    failed: HashSet<GraphAddr>,
}

/// Failure to erase a node. Its serde errored, or produced no map of fields.
#[derive(Clone, Debug, thiserror::Error)]
pub enum EraseNodeError {
    /// The node's own serde failed.
    #[error("node serde error: {0}")]
    Datum(#[from] DatumError),
    /// The node's serde produced a value that is neither a map nor null.
    #[error("node serde produced a value that is not a map of fields")]
    NotAMap,
}

/// Failure to reify a typed node from its data form. The tag is unknown to
/// the node set, or the fields fail the node's own deserialization.
#[derive(Clone, Debug, thiserror::Error)]
#[error("node type `{tag}`: {source}")]
pub struct ReifyNodeError {
    /// The wire tag of the node that failed to decode.
    pub tag: String,
    /// The decode failure.
    #[source]
    pub source: DatumError,
}

/// Failure to reify one of a graph's nodes.
#[derive(Clone, Debug, thiserror::Error)]
#[error("node {node_ix}: {source}")]
pub struct ReifyError {
    /// The graph index of the node that failed to decode.
    pub node_ix: usize,
    /// The node-level failure.
    #[source]
    pub source: ReifyNodeError,
}

/// Failure to reify a registry graph while filling the cache.
#[derive(Clone, Debug, thiserror::Error)]
#[error("graph {graph}: {source}")]
pub struct EnsureError {
    /// The address of the registry graph that failed to reify.
    pub graph: GraphAddr,
    /// The graph-level failure.
    #[source]
    pub source: ReifyError,
}

impl<N> ReifiedGraphs<N> {
    /// An empty cache.
    pub fn new() -> Self {
        Self {
            graphs: HashMap::new(),
            failed: HashSet::new(),
        }
    }

    /// The reified graph at the given address, if it has been ensured.
    pub fn get(&self, addr: &GraphAddr) -> Option<&Graph<N>> {
        self.graphs.get(addr)
    }

    /// Whether the given address has been reified.
    pub fn contains(&self, addr: &GraphAddr) -> bool {
        self.graphs.contains_key(addr)
    }

    /// Drop entries outside the given live set to bound memory after a prune.
    pub fn retain_live(&mut self, live: &gantz_ca::LiveSet) {
        self.graphs.retain(|addr, _| live.graphs.contains(addr));
        self.failed.retain(|addr| live.graphs.contains(addr));
    }
}

impl<N> ReifiedGraphs<N> {
    /// Reify the given seed addresses and every graph they transitively
    /// reference, decoding each node weight through `reify_node`.
    ///
    /// References are resolved through the stored graphs' [`NodeData::refs`]
    /// columns. This is a pure data walk. Nothing is decoded to find the set.
    /// Addresses that do not resolve to registry graphs are ignored, for
    /// example builtin node addresses in a node's refs. Already-cached graphs
    /// are ignored too.
    pub fn ensure_with(
        &mut self,
        reg: &Registry,
        seeds: impl IntoIterator<Item = ContentAddr>,
        reify_node: impl Fn(&NodeData) -> Result<N, ReifyNodeError>,
    ) -> Result<(), EnsureError> {
        let mut queue: VecDeque<GraphAddr> = seeds.into_iter().map(GraphAddr::from).collect();
        while let Some(addr) = queue.pop_front() {
            if self.graphs.contains_key(&addr) {
                continue;
            }
            let Some(dg) = reg.graph(&addr) else { continue };
            queue.extend(
                dg.node_weights()
                    .flat_map(|n| n.refs.iter().copied().map(GraphAddr::from)),
            );
            let g = reify_with(dg, &reify_node).map_err(|source| EnsureError {
                graph: addr,
                source,
            })?;
            self.graphs.insert(addr, g);
        }
        Ok(())
    }

    /// Reify every graph in the registry's column, best effort, decoding each
    /// node weight through `reify_node`.
    ///
    /// Graphs that fail to reify are skipped and reported once. An unknown
    /// tag from a domain not compiled in is one example. They remain cache
    /// misses, and lookups degrade over them the same way as over any
    /// missing node. Later calls do not retry them.
    pub fn ensure_all_with(
        &mut self,
        reg: &Registry,
        reify_node: impl Fn(&NodeData) -> Result<N, ReifyNodeError>,
    ) -> Vec<EnsureError> {
        let mut errs = vec![];
        for (addr, dg) in reg.graphs() {
            if self.graphs.contains_key(addr) || self.failed.contains(addr) {
                continue;
            }
            match reify_with(dg, &reify_node) {
                Ok(g) => {
                    self.graphs.insert(*addr, g);
                }
                Err(source) => {
                    self.failed.insert(*addr);
                    errs.push(EnsureError {
                        graph: *addr,
                        source,
                    });
                }
            }
        }
        errs
    }
}

impl<N> Default for ReifiedGraphs<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Erase a typed node to data under an externally supplied wire tag.
///
/// Runs the node's own serde to a [`Datum`]. It also extracts the node's
/// direct outgoing references from [`Node::required_addrs`] and
/// [`Node::required_blobs`], including physically nested nodes. The result is
/// canonical, so its [`NodeData::content_addr`] is the node's one
/// network-wide address.
///
/// The caller is usually [`erase_node_typed`], which passes the node's
/// [`NodeTag`](gantz_nodetag::NodeTag). The node's serde must produce a map,
/// else the erasure fails as [`EraseNodeError::NotAMap`]. A unit-struct
/// node's `Null` counts as the empty map. A serde that embeds its own
/// `"type"` entry has it stripped, so [`NodeData::tag`] stays the single
/// source of the tag.
pub fn erase_node_tagged<N>(tag: &str, node: &N) -> Result<NodeData, EraseNodeError>
where
    N: Serialize + Node,
{
    let mut fields = match datum::to_datum(node)? {
        Datum::Map(fields) => fields,
        Datum::Null => vec![],
        _ => return Err(EraseNodeError::NotAMap),
    };
    fields.retain(|(k, _)| k != "type");
    let (refs, blobs) = node_out_refs(node);
    let mut node_data = NodeData {
        tag: tag.to_string(),
        data: Datum::Map(fields),
        refs,
        blobs,
    };
    node_data.canonicalize();
    Ok(node_data)
}

/// Erase a typed node to data under its own declared
/// [`NodeTag`](gantz_nodetag::NodeTag).
///
/// See [`erase_node_tagged`].
pub fn erase_node_typed<T>(node: &T) -> Result<NodeData, EraseNodeError>
where
    T: gantz_nodetag::NodeTag + Serialize + Node,
{
    erase_node_tagged(T::TAG, node)
}

/// Reify one node at its concrete type. Run the type's own serde over the
/// stored fields.
///
/// No `"type"` tag is prepended, since a concrete type's serde must never
/// see one. Tag dispatch belongs to the caller. For example, it matches
/// [`NodeData::tag`] against each candidate type's
/// [`NodeTag`](gantz_nodetag::NodeTag).
pub fn reify_node_concrete<T>(node_data: &NodeData) -> Result<T, ReifyNodeError>
where
    T: DeserializeOwned,
{
    let err = |source| ReifyNodeError {
        tag: node_data.tag.clone(),
        source,
    };
    let Datum::Map(_) = node_data.data else {
        return Err(err(serde::de::Error::custom("node data is not a map")));
    };
    datum::from_datum(node_data.data.clone()).map_err(err)
}

/// Failure to edit a stored node through its type. See [`edit_node_typed`].
#[derive(Clone, Debug, thiserror::Error)]
pub enum EditNodeError {
    #[error(transparent)]
    Reify(#[from] ReifyNodeError),
    #[error(transparent)]
    Erase(#[from] EraseNodeError),
    /// The edit changes fields that hold data this build does not recognise,
    /// so writing them would lose that data. Holds the lost paths. See
    /// [`NodeData::lost_in`].
    #[error("the edit changes fields that hold unrecognised data: {}", .0.join(", "))]
    Lossy(Vec<String>),
}

/// Edit a stored node through its concrete type `T`, keeping the data that
/// `T` does not recognise. Returns whether `f` changed the node.
///
/// A plain [`reify_node_concrete`] and [`erase_node_typed`] round trip drops
/// such data, for example a field that a newer gantz wrote. This writes
/// back only the top-level fields that `f` changed, and keeps every other
/// stored field as it is. The `refs` and `blobs` columns keep the stored
/// entries that `T` does not report, plus those that the edited node does.
/// An edit to a field that itself holds unrecognised data fails with
/// [`EditNodeError::Lossy`] and leaves `node_data` as it is.
pub fn edit_node_typed<T>(
    node_data: &mut NodeData,
    f: impl FnOnce(&mut T) -> bool,
) -> Result<bool, EditNodeError>
where
    T: gantz_nodetag::NodeTag + Serialize + DeserializeOwned + Node,
{
    let mut node: T = reify_node_concrete(node_data)?;
    let before = erase_node_typed(&node)?;
    if !f(&mut node) {
        return Ok(false);
    }
    let after = erase_node_typed(&node)?;
    let lost = node_data.lost_in(&before);
    if lost.is_empty() {
        *node_data = after;
        return Ok(true);
    }
    let (Datum::Map(stored), Datum::Map(old), Datum::Map(new)) =
        (&mut node_data.data, &before.data, &after.data)
    else {
        return Err(EditNodeError::Lossy(lost));
    };
    let field = |fields: &'_ [(String, Datum)], key: &str| {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let keys: BTreeSet<&String> = old.iter().chain(new).map(|(k, _)| k).collect();
    let changed: Vec<&String> = keys
        .into_iter()
        .filter(|k| field(old, k) != field(new, k))
        .collect();
    let clash: Vec<String> = lost
        .into_iter()
        .filter(|path| changed.iter().any(|k| is_under(path, k)))
        .collect();
    if !clash.is_empty() {
        return Err(EditNodeError::Lossy(clash));
    }
    for key in changed {
        stored.retain(|(k, _)| k != key);
        stored.extend(field(new, key).map(|value| (key.clone(), value)));
    }
    node_data.refs.retain(|r| !before.refs.contains(r));
    node_data.refs.extend(after.refs);
    node_data.blobs.retain(|b| !before.blobs.contains(b));
    node_data.blobs.extend(after.blobs);
    node_data.canonicalize();
    Ok(true)
}

/// Whether the data `path` names is the field `key` or lies within it.
fn is_under(path: &str, key: &str) -> bool {
    path.strip_prefix(key)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
}

/// Reify a typed graph from its stored data form, decoding each node weight
/// through `reify_node`. Indices and edges are preserved verbatim.
pub fn reify_with<N>(
    g: &DataGraph,
    reify_node: impl Fn(&NodeData) -> Result<N, ReifyNodeError>,
) -> Result<Graph<N>, ReifyError> {
    let mut out = Graph::with_capacity(g.node_count(), g.edge_count());
    for (node_ix, node_data) in g.node_weights().enumerate() {
        let node = reify_node(node_data).map_err(|source| ReifyError { node_ix, source })?;
        out.add_node(node);
    }
    for e in g.edge_references() {
        out.add_edge(e.source(), e.target(), *e.weight());
    }
    Ok(out)
}

/// A node's direct outgoing references. Its own reporting plus that of its
/// physically nested nodes.
///
/// The absent node lookup stops reference nodes from following their target
/// into other graphs. This keeps the result direct. The reachability walk
/// owns the transitive closure.
fn node_out_refs<N: Node>(node: &N) -> (Vec<ContentAddr>, Vec<(SectionId, ContentAddr)>) {
    fn no_node(_: &ContentAddr) -> Option<&'static dyn Node> {
        None
    }
    let mut addrs = HashSet::new();
    let mut blobs = HashSet::new();
    node::visit(
        visit::Ctx::new(&no_node, &[], &[]),
        node,
        &mut visit::RequiredAddrs { addrs: &mut addrs },
    );
    node::visit(
        visit::Ctx::new(&no_node, &[], &[]),
        node,
        &mut visit::RequiredBlobs { blobs: &mut blobs },
    );
    let mut refs: Vec<_> = addrs.into_iter().collect();
    refs.sort();
    let mut blobs: Vec<_> = blobs.into_iter().collect();
    blobs.sort();
    (refs, blobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::ExprResult;
    use gantz_nodetag::NodeTag;

    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, NodeTag)]
    struct Num {
        v: i64,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, NodeTag)]
    struct Link {
        addr: ContentAddr,
    }

    impl Node for Num {
        fn expr(&self, _: node::ExprCtx) -> ExprResult {
            unimplemented!("not compiled in these tests")
        }
    }

    impl Node for Link {
        fn expr(&self, _: node::ExprCtx) -> ExprResult {
            unimplemented!("not compiled in these tests")
        }

        fn required_addrs(&self) -> Vec<ContentAddr> {
            vec![self.addr]
        }
    }

    /// A minimal node set, with a codec that dispatches on the stored tag.
    #[derive(Clone, Debug, PartialEq)]
    enum TestNode {
        Num(Num),
        Link(Link),
    }

    fn num(v: i64) -> TestNode {
        TestNode::Num(Num { v })
    }

    fn link(addr: ContentAddr) -> TestNode {
        TestNode::Link(Link { addr })
    }

    fn erase(node: &TestNode) -> NodeData {
        match node {
            TestNode::Num(n) => erase_node_typed(n),
            TestNode::Link(l) => erase_node_typed(l),
        }
        .unwrap()
    }

    fn reify(node_data: &NodeData) -> Result<TestNode, ReifyNodeError> {
        match node_data.tag.as_str() {
            Num::TAG => reify_node_concrete(node_data).map(TestNode::Num),
            Link::TAG => reify_node_concrete(node_data).map(TestNode::Link),
            tag => Err(ReifyNodeError {
                tag: tag.to_string(),
                source: serde::de::Error::custom("unknown node type"),
            }),
        }
    }

    fn erase_graph(g: &Graph<TestNode>) -> DataGraph {
        g.map(|_, n| erase(n), |_, e| *e)
    }

    fn graph(nodes: impl IntoIterator<Item = TestNode>) -> Graph<TestNode> {
        let mut g = Graph::default();
        let ixs: Vec<_> = nodes.into_iter().map(|n| g.add_node(n)).collect();
        for w in ixs.windows(2) {
            g.add_edge(w[0], w[1], gantz_ca::Edge::from((0, 0)));
        }
        g
    }

    #[test]
    fn erase_node_typed_takes_the_tag_and_extracts_refs() {
        let nd = erase(&num(42));
        assert_eq!(nd.tag, "Num");
        assert_eq!(nd.data, Datum::Map(vec![("v".into(), Datum::I64(42))]));
        assert!(nd.refs.is_empty() && nd.blobs.is_empty());
        assert!(nd.is_canonical());

        let target = ContentAddr([7; 32]);
        let nd = erase(&link(target));
        assert_eq!(nd.tag, "Link");
        assert_eq!(nd.refs, vec![target]);
    }

    /// A serde that embeds its own `"type"` entry erases to the same data as
    /// the plain type, so the stored tag is the only tag.
    #[test]
    fn erase_node_tagged_strips_an_embedded_type() {
        #[derive(serde::Serialize)]
        #[serde(tag = "type")]
        enum Embedded {
            Num { v: i64 },
        }

        impl Node for Embedded {
            fn expr(&self, _: node::ExprCtx) -> ExprResult {
                unimplemented!("not compiled in these tests")
            }
        }

        let embedded = erase_node_tagged("Num", &Embedded::Num { v: 42 }).unwrap();
        assert_eq!(embedded, erase(&num(42)));
    }

    /// Concrete typed nodes round-trip without ever seeing a `"type"` field.
    /// A fields struct and a unit struct both erase via their [`NodeTag`] and
    /// reify back at their concrete type. The unit struct's typed serde
    /// yields `Null`, erased as the empty map.
    #[test]
    fn concrete_erase_reify_round_trips() {
        #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize, NodeTag)]
        struct Unit;

        impl Node for Unit {
            fn expr(&self, _: node::ExprCtx) -> ExprResult {
                unimplemented!("not compiled in these tests")
            }
        }

        let nd = erase_node_typed(&Num { v: 7 }).unwrap();
        assert_eq!(reify_node_concrete::<Num>(&nd).unwrap(), Num { v: 7 });

        let nd = erase_node_typed(&Unit).unwrap();
        assert_eq!(nd.tag, "Unit");
        assert_eq!(nd.data, Datum::Map(vec![]));
        assert_eq!(reify_node_concrete::<Unit>(&nd).unwrap(), Unit);
    }

    #[test]
    fn graph_round_trips_preserving_structure() {
        let mut g = graph([num(1), num(2), num(3)]);
        // A parallel edge and a distinct socket pairing survive.
        g.add_edge(0.into(), 2.into(), gantz_ca::Edge::from((1, 1)));
        let back = reify_with(&erase_graph(&g), reify).unwrap();
        let weights: Vec<_> = back.node_weights().cloned().collect();
        assert_eq!(weights, vec![num(1), num(2), num(3)]);
        let edges: Vec<_> = back
            .edge_references()
            .map(|e| (e.source().index(), e.target().index(), *e.weight()))
            .collect();
        let expected: Vec<_> = g
            .edge_references()
            .map(|e| (e.source().index(), e.target().index(), *e.weight()))
            .collect();
        assert_eq!(edges, expected);
    }

    #[test]
    fn reify_unknown_tag_names_node_and_tag() {
        let mut dg = erase_graph(&graph([num(1)]));
        dg.node_weights_mut().for_each(|n| n.tag = "Mystery".into());
        let err = reify_with(&dg, reify).unwrap_err();
        assert_eq!(err.node_ix, 0);
        assert_eq!(err.source.tag, "Mystery");
        assert!(err.to_string().contains("Mystery"), "{err}");
    }

    #[test]
    fn ensure_reifies_transitive_refs_and_ignores_unresolved() {
        let mut reg = Registry::default();
        let leaf = reg.add_graph(erase_graph(&graph([num(1)])));
        let mid = reg.add_graph(erase_graph(&graph([link(leaf.into()), num(2)])));
        let root = {
            // One resolvable ref and one dangling builtin-style addr.
            let mut g = graph([link(mid.into())]);
            g.add_node(link(ContentAddr([9; 32])));
            reg.add_graph(erase_graph(&g))
        };

        let mut cache = ReifiedGraphs::<TestNode>::new();
        cache.ensure_with(&reg, [root.into()], reify).unwrap();
        assert!(cache.contains(&root) && cache.contains(&mid) && cache.contains(&leaf));
        assert!(!cache.contains(&GraphAddr::from(ContentAddr([9; 32]))));
        assert_eq!(cache.get(&leaf).unwrap().node_count(), 1);

        // Ensuring again is a no-op walk over cached entries.
        cache.ensure_with(&reg, [root.into()], reify).unwrap();
    }

    /// A node with a nested field, for `edit_node_typed`.
    #[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Styled {
        name: String,
        style: Style,
    }

    #[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Style {
        color: i64,
    }

    impl gantz_nodetag::NodeTag for Styled {
        const TAG: &'static str = "Styled";
    }

    impl Node for Styled {
        fn expr(&self, _: node::ExprCtx) -> ExprResult {
            unimplemented!("not compiled in these tests")
        }
    }

    /// `Styled` as stored by a newer gantz, with an unknown top-level field
    /// and an unknown field in its style.
    fn newer_styled() -> NodeData {
        let mut node_data = erase_node_typed(&Styled::default()).unwrap();
        let Datum::Map(fields) = &mut node_data.data else {
            panic!("a map");
        };
        fields.push(("future".to_string(), Datum::Bool(true)));
        for (key, value) in fields.iter_mut() {
            if let ("style", Datum::Map(style)) = (key.as_str(), value) {
                style.push(("glow".to_string(), Datum::I64(2)));
            }
        }
        node_data.canonicalize();
        node_data
    }

    #[test]
    fn edit_node_typed_keeps_data_the_edit_leaves_alone() {
        let mut node_data = newer_styled();
        let renamed = edit_node_typed(&mut node_data, |s: &mut Styled| {
            s.name = "renamed".to_string();
            true
        });
        assert!(renamed.unwrap());
        let named: Styled = reify_node_concrete(&node_data).unwrap();
        assert_eq!(named.name, "renamed");
        let roundtrip = erase_node_typed(&named).unwrap();
        assert_eq!(node_data.lost_in(&roundtrip), ["future", "style.glow"]);
    }

    #[test]
    fn edit_node_typed_refuses_to_rewrite_unrecognised_data() {
        let mut node_data = newer_styled();
        let before = node_data.clone();
        let recoloured = edit_node_typed(&mut node_data, |s: &mut Styled| {
            s.style.color = 3;
            true
        });
        let Err(EditNodeError::Lossy(lost)) = recoloured else {
            panic!("expected a lossy edit, got {recoloured:?}");
        };
        assert_eq!(lost, ["style.glow"]);
        assert_eq!(node_data, before);
    }

    /// A graph that fails to reify is reported once, then not retried.
    #[test]
    fn ensure_all_with_reports_a_failed_graph_once() {
        let mut reg = Registry::default();
        for nd in [erase(&num(1)), NodeData::new("Bad", Datum::Map(vec![]))] {
            let mut dg = DataGraph::default();
            dg.add_node(nd);
            let addr = gantz_ca::graph_addr(&dg);
            reg.commit_graph(std::time::Duration::ZERO, None, addr, || dg);
        }
        let bad_attempts = std::cell::Cell::new(0);
        let reify = |nd: &NodeData| match nd.tag.as_str() {
            "Bad" => {
                bad_attempts.set(bad_attempts.get() + 1);
                let source = serde::de::Error::custom("unknown node type");
                let tag = nd.tag.clone();
                Err(ReifyNodeError { tag, source })
            }
            _ => Ok(nd.tag.clone()),
        };
        let mut reified = ReifiedGraphs::new();
        assert_eq!(reified.ensure_all_with(&reg, reify).len(), 1);
        assert_eq!(reified.ensure_all_with(&reg, reify).len(), 0);
        assert_eq!(bad_attempts.get(), 1);
        assert_eq!(reified.graphs.len(), 1);
    }
}
