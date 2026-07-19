//! A pre-derivation flattening pass for nested graphs.
//!
//! Nesting (committing a subgraph with `Inlet`/`Outlet` boundaries and reusing
//! it as a single ref node) is gantz's primary abstraction. The control (Steel)
//! compiler supports it call-based: each nested level becomes a function the
//! parent calls. A synthdef is a flat unit list with no notion of calling a
//! sub-synthdef, so the DSP compiler lowers a ref one of two ways: *instance*
//! it (the default for DSP-bearing children) - keep an opaque marker that
//! `derive_template` (crate::instance) turns into a shared synthdef spawned
//! per instance - or *inline* it: splice the referenced graph's nodes into
//! one flat graph, dissolving the `Inlet`/`Outlet` boundary nodes into the
//! surrounding edges. The spliced result derives via
//! [`derive_synthdef`](crate::derive_synthdef) or
//! [`derive_synthdefs`](crate::derive_synthdefs) unchanged.
//!
//! Every spliced node carries its original path within the nested structure
//! (a [`Flat`] weight, surfaced through [`ToNodeDsp::node_path`]). Paths are
//! load-bearing: params are named by path, the audio driver bridges param and
//! scope state to the VM by path, buses are allocated by path and region keys
//! hash paths. Keeping original paths means a node's identity survives
//! re-derivation regardless of where it lands in the flat graph.
//!
//! Not every ref splices. A ref resolved as [`RefKind::Instance`] stays an
//! opaque [`Flat::Instance`] marker for
//! `derive_template` (crate::instance) to lower into a
//! shared synthdef spawned per instance. Root-level `Inlet`/`Outlet` nodes are
//! likewise kept as markers ([`Flat::Inlet`]/[`Flat::Outlet`]): they are the
//! flattened graph's own interface, which template derivation lowers to the
//! shared def's bus reads and writes. All markers are non-DSP
//! (`to_node_dsp()` is `None`), so [`derive_synthdef`](crate::derive_synthdef)
//! and [`derive_synthdefs`](crate::derive_synthdefs) ignore them.
//!
//! # Edge bridging
//!
//! An edge into a ref's input `i` belongs to every consumer of the referenced
//! graph's `i`-th inlet, and an edge from a ref's output `j` re-sources from
//! the edges feeding the referenced graph's `j`-th outlet (inlets and
//! outlets map positionally by ascending node index, the same "input i to
//! inlet i" contract the control compiler uses). Bridging resolves through
//! arbitrarily deep chains of boundaries (a pure `inlet -> outlet` wire
//! dissolves entirely). *Every* resolving chain bridges as its own flat edge
//! - derivation sums a multi-fed input, so a boundary fanned in by several
//! sources delivers every summand (two distinct chains reaching the same
//! source deliberately sum it twice). An unresolvable chain (an unconnected
//! inlet or outlet along the way) produces no edge, so the consumer's input
//! falls back to derivation's usual mono silence.

use std::collections::HashMap;

use petgraph::Direction;
use petgraph::visit::EdgeRef;

use gantz_ca::{ContentAddr, GraphAddr};
use gantz_core::Edge;
use gantz_core::node::graph::{Graph, NodeIx};
use gantz_core::node::{GetNode, MetaCtx};

use crate::dsp::{NodeDsp, ToNodeDsp};

pub use gantz_core::node::AsRefNode;

/// A vertex of the flattened graph: a node spliced out of the nested
/// structure, an opaque instanced-reference marker, or a root-level boundary
/// marker. Every variant carries its original path (e.g. `[3, 2]` for the
/// node at index 2 within the graph referenced by the root node at index 3).
#[derive(Clone, Debug)]
pub enum Flat<N> {
    /// A concrete node spliced into the flat graph.
    Node {
        /// The node's original path within the nested structure.
        path: Vec<usize>,
        /// The node itself.
        node: N,
    },
    /// An instanced nested-graph ref: opaque to derivation, resolved by
    /// `derive_template` (crate::instance) into a shared
    /// synthdef variant wired per instance.
    Instance {
        /// The ref node's original path within the nested structure.
        path: Vec<usize>,
        /// The referenced child graph's content address.
        child_ca: ContentAddr,
        /// The ref's inlet count (the child graph's `Inlet` count), for
        /// instance-aware reachability and edge bridging.
        n_inlets: usize,
        /// The ref's outlet count (the child graph's `Outlet` count).
        n_outlets: usize,
    },
    /// A root-level `Inlet`, kept as a marker: it is the flattened graph's own
    /// interface, which template derivation lowers to a shared-def bus read.
    /// (Nested inlets dissolve into the surrounding edges as before.)
    Inlet {
        /// The inlet node's path (`[ix]` - root markers are never nested).
        path: Vec<usize>,
        /// The inlet's position among the root's inlets in ascending node
        /// index order (the "input i to inlet i" contract).
        index: usize,
    },
    /// A root-level `Outlet` marker (see [`Flat::Inlet`]).
    Outlet {
        /// The outlet node's path.
        path: Vec<usize>,
        /// The outlet's position among the root's outlets.
        index: usize,
    },
}

impl<N> Flat<N> {
    /// The vertex's original path within the nested structure.
    pub fn path(&self) -> &[usize] {
        match self {
            Flat::Node { path, .. }
            | Flat::Instance { path, .. }
            | Flat::Inlet { path, .. }
            | Flat::Outlet { path, .. } => path,
        }
    }
}

/// An error flattening a nested graph.
///
/// Both cases are defensive: the editor refuses to create ref cycles and the
/// registry holds a committed graph for every ref it hands out, so neither
/// should be reachable through the application.
#[derive(Debug, thiserror::Error)]
pub enum FlattenError {
    /// A graph ref (transitively) resolves through itself.
    #[error("graph reference resolves through itself: {0}")]
    RefCycle(ContentAddr),
    /// A graph ref whose target graph could not be found.
    #[error("unresolved graph reference: {0}")]
    Unresolved(ContentAddr),
}

/// How a nested-graph ref lowers during flattening.
///
/// `Inline` splices the referenced graph's nodes into the flat graph (the
/// classic behaviour). `Instance` leaves an opaque [`Flat::Instance`] marker,
/// deferring the child's DSP to a shared synthdef derived once and wired per
/// instance (see `derive_template` (crate::instance)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    /// Splice the referenced graph's nodes into the flat graph.
    Inline,
    /// Leave an opaque instance marker; the child's DSP is derived once into a
    /// shared synthdef and wired per instance.
    Instance,
}

/// Resolves a node to the committed graph it references, if any, and how it
/// lowers.
///
/// - `None`: not a graph ref. The node is copied into the flat graph as-is.
/// - `Some((ca, RefKind::Inline, Some(graph)))`: an inlined graph ref. The
///   referenced graph is spliced in place of the node, with `ca` keying the
///   ref-cycle guard.
/// - `Some((ca, RefKind::Instance, _))`: an instanced graph ref. An opaque
///   [`Flat::Instance`] marker carrying `ca` is emitted in place of the node;
///   the child graph is not resolved at flatten time (it may be `None`).
/// - `Some((_, RefKind::Inline, None))`: a graph ref whose target is missing,
///   an [`FlattenError::Unresolved`] error.
pub type Resolve<'g, N> = dyn Fn(&N) -> Option<(ContentAddr, RefKind, Option<&'g Graph<N>>)> + 'g;

/// One level of the nested structure: a graph, where it hangs off its parent,
/// and where each of its nodes went during splicing. Bridging resolves edge
/// sources through this table.
struct Level<'g, N> {
    graph: &'g Graph<N>,
    /// The parent level's index and the ref node there, `None` at the root.
    parent: Option<(usize, NodeIx)>,
    /// Inlet nodes in ascending index order (the "input i to inlet i" contract).
    inlets: Vec<NodeIx>,
    /// Outlet nodes in ascending index order.
    outlets: Vec<NodeIx>,
    /// Copied nodes: original index to flat-graph index.
    kept: HashMap<NodeIx, NodeIx>,
    /// Resolved refs: ref node index to the child's index in the level table.
    child: HashMap<NodeIx, usize>,
}

/// An edge-source endpoint (level, node, output port), tracked on a stack
/// during source resolution to guard against pure boundary wiring cycles.
type SrcKey = (usize, NodeIx, usize);

impl<N: ToNodeDsp> ToNodeDsp for Flat<N> {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        match self {
            Flat::Node { node, .. } => node.to_node_dsp(),
            Flat::Instance { .. } | Flat::Inlet { .. } | Flat::Outlet { .. } => None,
        }
    }

    fn node_path(&self, _ix: usize) -> Vec<usize> {
        self.path().to_vec()
    }
}

/// Flatten `graph`, splicing every nested level (per `resolve`) into one flat
/// graph and dissolving `Inlet`/`Outlet` boundary nodes into the surrounding
/// edges (see the module docs).
///
/// `get_node` backs the [`MetaCtx`] used to identify inlets and outlets via
/// the [`gantz_core::Node::inlet`]/[`outlet`](gantz_core::Node::outlet)
/// predicates (so identification agrees with the control compiler, including
/// through refs). Nodes that are neither graph refs nor boundaries are copied
/// as-is. Non-DSP nodes ride along harmlessly, derivation ignores them.
pub fn flatten<'g, N>(
    get_node: GetNode<'_>,
    graph: &'g Graph<N>,
    resolve: &Resolve<'g, N>,
) -> Result<Graph<Flat<&'g N>>, FlattenError>
where
    N: gantz_core::Node,
{
    let ctx = MetaCtx::new(get_node);
    let mut levels = Vec::new();
    let mut out = Graph::default();
    let mut ca_stack = Vec::new();
    splice(
        ctx,
        resolve,
        graph,
        None,
        Vec::new(),
        &mut levels,
        &mut out,
        &mut ca_stack,
    )?;
    bridge(&levels, &mut out);
    Ok(out)
}

/// [`flatten`] resolving [`AsRefNode`] nodes through the content-addressed
/// registry (a reference's content address is the referenced graph's
/// `GraphAddr`).
///
/// How each ref lowers is decided here: a ref whose child (transitively)
/// contains DSP nodes lowers as [`RefKind::Instance`] by default - its child
/// derives once into shared synthdefs spawned per instance - unless its
/// [`DspRefExt`](crate::ref_ext::DspRefExt) ext datum sets `inline`, which
/// opts back into splicing. Refs to non-DSP children (including pure
/// `inlet -> outlet` wires) always splice: they carry structure, not sound,
/// and must dissolve.
pub fn flatten_from_registry<'g, N>(
    graph: &'g Graph<N>,
    reified: &'g gantz_core::data::ReifiedGraphs<N>,
) -> Result<Graph<Flat<&'g N>>, FlattenError>
where
    N: gantz_core::Node + AsRefNode + ToNodeDsp,
{
    let dsp_memo = std::cell::RefCell::new(HashMap::new());
    let resolve = move |n: &N| {
        n.as_ref_node().map(|r| {
            let ca = r.content_addr();
            let inline = r
                .ext_as::<crate::ref_ext::DspRefExt>(crate::ref_ext::DSP_REF_EXT_KEY)
                .unwrap_or_default()
                .inline;
            let kind = if !inline && is_dsp_graph(reified, ca.into(), &mut dsp_memo.borrow_mut()) {
                RefKind::Instance
            } else {
                RefKind::Inline
            };
            (ca, kind, reified.get(&ca.into()))
        })
    };
    let get_node = |ca: &ContentAddr| {
        reified
            .get(&(*ca).into())
            .map(|g| g as &dyn gantz_core::Node)
    };
    flatten(&get_node, graph, &resolve)
}

/// Flatten every child graph `flat` (transitively) instances, so template
/// derivation's resolver can hand out `&Graph<Flat<N>>` per child content
/// address. Walks [`Flat::Instance`] markers to a fixpoint: only children an
/// instanced ref actually reaches are flattened.
pub fn flatten_instance_children<'g, N>(
    flat: &Graph<Flat<&N>>,
    reified: &'g gantz_core::data::ReifiedGraphs<N>,
) -> Result<HashMap<ContentAddr, Graph<Flat<&'g N>>>, FlattenError>
where
    N: gantz_core::Node + AsRefNode + ToNodeDsp,
{
    fn marker_cas<N>(g: &Graph<Flat<N>>) -> Vec<ContentAddr> {
        g.node_indices()
            .filter_map(|n| match &g[n] {
                Flat::Instance { child_ca, .. } => Some(*child_ca),
                _ => None,
            })
            .collect()
    }
    let mut out: HashMap<ContentAddr, Graph<Flat<&'g N>>> = HashMap::new();
    let mut queue = marker_cas(flat);
    while let Some(ca) = queue.pop() {
        if out.contains_key(&ca) {
            continue;
        }
        let graph = reified
            .get(&ca.into())
            .ok_or(FlattenError::Unresolved(ca))?;
        let child = flatten_from_registry(graph, reified)?;
        queue.extend(marker_cas(&child));
        out.insert(ca, child);
    }
    Ok(out)
}

/// Phase 1: recursively copy `graph`'s concrete nodes into `out` (paths
/// prefixed by `prefix`), recording each level's boundary nodes and resolved
/// refs in `levels` for [`bridge`] to resolve edges through. Returns the
/// level's index within `levels`.
#[allow(clippy::too_many_arguments)]
fn splice<'g, N>(
    ctx: MetaCtx,
    resolve: &Resolve<'g, N>,
    graph: &'g Graph<N>,
    parent: Option<(usize, NodeIx)>,
    prefix: Vec<usize>,
    levels: &mut Vec<Level<'g, N>>,
    out: &mut Graph<Flat<&'g N>>,
    ca_stack: &mut Vec<ContentAddr>,
) -> Result<usize, FlattenError>
where
    N: gantz_core::Node,
{
    let id = levels.len();
    levels.push(Level {
        graph,
        parent,
        inlets: Vec::new(),
        outlets: Vec::new(),
        kept: HashMap::new(),
        child: HashMap::new(),
    });
    for ix in graph.node_indices() {
        let node = &graph[ix];
        let path = || {
            let mut p = prefix.clone();
            p.push(ix.index());
            p
        };
        if let Some((ca, kind, child_graph)) = resolve(node) {
            match kind {
                // An instanced ref: emit an opaque marker carrying the child
                // CA and do NOT splice (no recursion, no cycle check - an
                // instance never resolves its child at flatten time). The
                // marker behaves as a kept node with the ref's inputs/outputs.
                RefKind::Instance => {
                    let n_inlets = node.n_inputs(ctx);
                    let n_outlets = node.n_outputs(ctx);
                    let flat = out.add_node(Flat::Instance {
                        path: path(),
                        child_ca: ca,
                        n_inlets,
                        n_outlets,
                    });
                    levels[id].kept.insert(ix, flat);
                }
                RefKind::Inline => {
                    if ca_stack.contains(&ca) {
                        return Err(FlattenError::RefCycle(ca));
                    }
                    let child_graph = child_graph.ok_or(FlattenError::Unresolved(ca))?;
                    ca_stack.push(ca);
                    let child = splice(
                        ctx,
                        resolve,
                        child_graph,
                        Some((id, ix)),
                        path(),
                        levels,
                        out,
                        ca_stack,
                    )?;
                    ca_stack.pop();
                    levels[id].child.insert(ix, child);
                }
            }
        } else if node.inlet(ctx) {
            levels[id].inlets.push(ix);
            // A root-level inlet is the flat graph's own interface: keep it
            // as a marker for template derivation. Nested inlets dissolve.
            if parent.is_none() {
                let index = levels[id].inlets.len() - 1;
                let flat = out.add_node(Flat::Inlet {
                    path: path(),
                    index,
                });
                levels[id].kept.insert(ix, flat);
            }
        } else if node.outlet(ctx) {
            levels[id].outlets.push(ix);
            if parent.is_none() {
                let index = levels[id].outlets.len() - 1;
                let flat = out.add_node(Flat::Outlet {
                    path: path(),
                    index,
                });
                levels[id].kept.insert(ix, flat);
            }
        } else {
            let flat = out.add_node(Flat::Node { path: path(), node });
            levels[id].kept.insert(ix, flat);
        }
    }
    Ok(id)
}

/// Phase 2: emit the flat edges. Only edges whose target was kept are
/// considered (each concrete input's feeds are enumerated exactly once, at
/// the level where the target lives), with each source resolved through the
/// boundary chain via [`resolve_src`] - one flat edge per resolving chain
/// (derivation sums a multi-fed input). Levels are visited in splice order
/// and edges in creation (age) order, so a kept input's flat edges keep their
/// original relative age and the flat graph matches an equivalent
/// hand-flattened one.
fn bridge<'g, N>(levels: &[Level<'g, N>], out: &mut Graph<Flat<&'g N>>) {
    for (id, level) in levels.iter().enumerate() {
        for e in level.graph.edge_references() {
            let Some(&flat_t) = level.kept.get(&e.target()) else {
                continue;
            };
            let mut stack = Vec::new();
            let src = e.weight().output.0 as usize;
            for (flat_s, port) in resolve_src(levels, id, e.source(), src, &mut stack) {
                let edge = Edge::new((port as u16).into(), e.weight().input);
                out.add_edge(flat_s, flat_t, edge);
            }
        }
    }
}

/// Resolve the source endpoint `(s, sp)` at level `lvl` to every kept flat
/// node output its boundary chains reach, following ref outputs down into
/// their child's outlet and inlet outputs up into the parent's edges. Empty
/// when every chain dead-ends (an unconnected boundary) or revisits an
/// endpoint on `stack` (a pure boundary wiring cycle).
fn resolve_src<N>(
    levels: &[Level<'_, N>],
    lvl: usize,
    s: NodeIx,
    sp: usize,
    stack: &mut Vec<SrcKey>,
) -> Vec<(NodeIx, usize)> {
    let key = (lvl, s, sp);
    if stack.contains(&key) {
        return Vec::new();
    }
    stack.push(key);
    let level = &levels[lvl];
    let resolved = if let Some(&flat) = level.kept.get(&s) {
        vec![(flat, sp)]
    } else if let Some(&child) = level.child.get(&s) {
        // A ref's output `sp` reads the child's `sp`-th outlet's feeds.
        levels[child]
            .outlets
            .get(sp)
            .copied()
            .map(|outlet| resolve_via_input(levels, child, outlet, 0, stack))
            .unwrap_or_default()
    } else if let Some(i) = level.inlets.iter().position(|&n| n == s) {
        // An inlet's output reads the parent ref's input `i`. A root-level
        // inlet has no parent to read, so it dissolves unconnected.
        level
            .parent
            .map(|(p, r)| resolve_via_input(levels, p, r, i, stack))
            .unwrap_or_default()
    } else {
        // An outlet as a source (outlets have no outputs) or a node dropped
        // by an earlier error path: nothing to wire.
        Vec::new()
    };
    stack.pop();
    resolved
}

/// Resolve every source feeding `node`'s `input` at level `lvl`, oldest edge
/// first (`edges_directed` iterates newest-first, hence the reversal) - each
/// resolving chain is a summand of the consumer's input.
fn resolve_via_input<N>(
    levels: &[Level<'_, N>],
    lvl: usize,
    node: NodeIx,
    input: usize,
    stack: &mut Vec<SrcKey>,
) -> Vec<(NodeIx, usize)> {
    let edges: Vec<_> = levels[lvl]
        .graph
        .edges_directed(node, Direction::Incoming)
        .filter(|e| e.weight().input.0 as usize == input)
        .map(|e| (e.source(), e.weight().output.0 as usize))
        .collect();
    edges
        .into_iter()
        .rev()
        .flat_map(|(s, sp)| resolve_src(levels, lvl, s, sp, stack))
        .collect()
}

/// Whether the reified graph at `ga` contains a DSP node, directly or
/// transitively through references: the lowering decision behind
/// [`RefKind::Instance`].
///
/// This is the typed twin of the data-level
/// [`is_dsp_graph`](crate::ref_ext::is_dsp_graph) (which classifies stored
/// registry data by wire tag): only the reified cache is in scope during a
/// flatten, so the probe stays typed - keep the two classifications in step.
/// Memoized in `memo` so repeated probes over one flatten stay linear
/// overall. Graphs missing from the cache classify as non-DSP.
fn is_dsp_graph<N>(
    reified: &gantz_core::data::ReifiedGraphs<N>,
    ga: GraphAddr,
    memo: &mut HashMap<GraphAddr, bool>,
) -> bool
where
    N: ToNodeDsp + AsRefNode,
{
    let mut stack: Vec<GraphAddr> = Vec::new();
    is_dsp(reified, ga, memo, &mut stack)
}

/// The recursive half of [`is_dsp_graph`]: reference cycles are treated as
/// non-DSP at the point of re-entry (a cycle cannot introduce a DSP node
/// that its members do not already contain).
fn is_dsp<N>(
    reified: &gantz_core::data::ReifiedGraphs<N>,
    ga: GraphAddr,
    memo: &mut HashMap<GraphAddr, bool>,
    stack: &mut Vec<GraphAddr>,
) -> bool
where
    N: ToNodeDsp + AsRefNode,
{
    if let Some(&known) = memo.get(&ga) {
        return known;
    }
    if stack.contains(&ga) {
        return false;
    }
    let Some(graph) = reified.get(&ga) else {
        memo.insert(ga, false);
        return false;
    };
    stack.push(ga);
    let dsp = graph
        .node_indices()
        .any(|ix| graph[ix].to_node_dsp().is_some())
        || graph.node_indices().any(|ix| {
            graph[ix]
                .as_ref_node()
                .is_some_and(|r| is_dsp(reified, r.content_addr().into(), memo, stack))
        });
    stack.pop();
    memo.insert(ga, dsp);
    dsp
}
