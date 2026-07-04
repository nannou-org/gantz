//! A pre-derivation flattening pass for nested graphs.
//!
//! Nesting (committing a subgraph with `Inlet`/`Outlet` boundaries and reusing
//! it as a single ref node) is gantz's primary abstraction. The control (Steel)
//! compiler supports it call-based: each nested level becomes a function the
//! parent calls. A synthdef is a flat unit list with no notion of calling a
//! sub-synthdef, so the DSP compiler instead *inlines*: [`flatten`] resolves
//! each graph ref, splices the referenced graph's nodes into one flat graph and
//! dissolves the `Inlet`/`Outlet` boundary nodes into the surrounding edges.
//! The result derives via [`derive_synthdef`](crate::derive_synthdef) or
//! [`derive_synthdefs`](crate::derive_synthdefs) unchanged.
//!
//! Every spliced node carries its original path within the nested structure
//! (a [`Flat`] weight, surfaced through [`ToNodeDsp::node_path`]). Paths are
//! load-bearing: params are named by path, the audio driver bridges param and
//! scope state to the VM by path, buses are allocated by path and region keys
//! hash paths. Keeping original paths means a node's identity survives
//! re-derivation regardless of where it lands in the flat graph.
//!
//! # Edge bridging
//!
//! An edge into a ref's input `i` belongs to every consumer of the referenced
//! graph's `i`-th inlet, and an edge from a ref's output `j` re-sources from
//! the single edge feeding the referenced graph's `j`-th outlet (inlets and
//! outlets map positionally by ascending node index, the same "input i to
//! inlet i" contract the control compiler uses). Bridging resolves through
//! arbitrarily deep chains of boundaries (a pure `inlet -> outlet` wire
//! dissolves entirely). At each hop the *oldest* edge whose chain resolves
//! wins, matching derivation's oldest-edge-wins input resolution, and an
//! unresolvable chain (an unconnected inlet or outlet along the way) produces
//! no edge, so the consumer's input falls back to derivation's usual mono
//! silence.

use std::collections::HashMap;

use petgraph::Direction;
use petgraph::visit::EdgeRef;

use gantz_ca::ContentAddr;
use gantz_core::Edge;
use gantz_core::node::graph::{Graph, NodeIx};
use gantz_core::node::{GetNode, MetaCtx};

use crate::dsp::{NodeDsp, ToNodeDsp};

pub use gantz_egui::node::NamedRef;
pub use gantz_egui::sync::AsNamedRef;

/// A node spliced out of a nested structure into a flat graph, carrying its
/// original path (e.g. `[3, 2]` for the node at index 2 within the graph
/// referenced by the root node at index 3).
#[derive(Clone, Debug)]
pub struct Flat<N> {
    /// The node's original path within the nested structure.
    pub path: Vec<usize>,
    /// The node itself.
    pub node: N,
}

/// An error flattening a nested graph.
///
/// Both cases are defensive: the editor refuses to create ref cycles and the
/// registry holds a committed graph for every ref it hands out, so neither
/// should be reachable through the application.
#[derive(Debug)]
pub enum FlattenError {
    /// A graph ref (transitively) resolves through itself.
    RefCycle(ContentAddr),
    /// A graph ref whose target graph could not be found.
    Unresolved(ContentAddr),
}

/// Resolves a node to the committed graph it references, if any.
///
/// - `None`: not a graph ref. The node is copied into the flat graph as-is.
/// - `Some((ca, Some(graph)))`: a graph ref. The referenced graph is spliced
///   in place of the node, with `ca` keying the ref-cycle guard.
/// - `Some((ca, None))`: a graph ref whose target is missing, an
///   [`FlattenError::Unresolved`] error.
pub type Resolve<'g, N> = dyn Fn(&N) -> Option<(ContentAddr, Option<&'g Graph<N>>)> + 'g;

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
        self.node.to_node_dsp()
    }

    fn node_path(&self, _ix: usize) -> Vec<usize> {
        self.path.clone()
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
) -> Result<Graph<Flat<N>>, FlattenError>
where
    N: gantz_core::Node + Clone,
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

/// [`flatten`] resolving [`AsNamedRef`] nodes through the content-addressed
/// registry (a `NamedRef`'s content address is the referenced graph's commit
/// address).
pub fn flatten_from_registry<'g, N>(
    graph: &'g Graph<N>,
    registry: &'g gantz_ca::Registry<Graph<N>>,
) -> Result<Graph<Flat<N>>, FlattenError>
where
    N: gantz_core::Node + AsNamedRef + Clone,
{
    let resolve = |n: &N| {
        n.as_named_ref().map(|nr| {
            let ca = nr.content_addr();
            (ca, registry.commit_graph_ref(&ca.into()))
        })
    };
    let get_node = |ca: &ContentAddr| {
        registry
            .commit_graph_ref(&(*ca).into())
            .map(|g| g as &dyn gantz_core::Node)
    };
    flatten(&get_node, graph, &resolve)
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
    out: &mut Graph<Flat<N>>,
    ca_stack: &mut Vec<ContentAddr>,
) -> Result<usize, FlattenError>
where
    N: gantz_core::Node + Clone,
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
        if let Some((ca, child_graph)) = resolve(node) {
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
        } else if node.inlet(ctx) {
            levels[id].inlets.push(ix);
        } else if node.outlet(ctx) {
            levels[id].outlets.push(ix);
        } else {
            let flat = out.add_node(Flat {
                path: path(),
                node: node.clone(),
            });
            levels[id].kept.insert(ix, flat);
        }
    }
    Ok(id)
}

/// Phase 2: emit the flat edges. Only edges whose target was kept are
/// considered (each concrete input's feeds are enumerated exactly once, at
/// the level where the target lives), with each source resolved through the
/// boundary chain via [`resolve_src`]. Levels are visited in splice order and
/// edges in creation (age) order, so a kept input's flat edges keep their
/// original relative age and derivation's oldest-edge-wins resolution behaves
/// as on an equivalent hand-flattened graph.
fn bridge<N>(levels: &[Level<'_, N>], out: &mut Graph<Flat<N>>) {
    for (id, level) in levels.iter().enumerate() {
        for e in level.graph.edge_references() {
            let Some(&flat_t) = level.kept.get(&e.target()) else {
                continue;
            };
            let mut stack = Vec::new();
            let src = e.weight().output.0 as usize;
            if let Some((flat_s, port)) = resolve_src(levels, id, e.source(), src, &mut stack) {
                let edge = Edge::new((port as u16).into(), e.weight().input);
                out.add_edge(flat_s, flat_t, edge);
            }
        }
    }
}

/// Resolve the source endpoint `(s, sp)` at level `lvl` to a kept flat node's
/// output, following ref outputs down into their child's outlet and inlet
/// outputs up into the parent's edges. `None` when the chain dead-ends
/// (an unconnected boundary) or revisits an endpoint on `stack` (a pure
/// boundary wiring cycle).
fn resolve_src<N>(
    levels: &[Level<'_, N>],
    lvl: usize,
    s: NodeIx,
    sp: usize,
    stack: &mut Vec<SrcKey>,
) -> Option<(NodeIx, usize)> {
    let key = (lvl, s, sp);
    if stack.contains(&key) {
        return None;
    }
    stack.push(key);
    let level = &levels[lvl];
    let resolved = if let Some(&flat) = level.kept.get(&s) {
        Some((flat, sp))
    } else if let Some(&child) = level.child.get(&s) {
        // A ref's output `sp` reads the child's `sp`-th outlet's one input.
        levels[child]
            .outlets
            .get(sp)
            .copied()
            .and_then(|outlet| resolve_via_input(levels, child, outlet, 0, stack))
    } else if let Some(i) = level.inlets.iter().position(|&n| n == s) {
        // An inlet's output reads the parent ref's input `i`. A root-level
        // inlet has no parent to read, so it dissolves unconnected.
        level
            .parent
            .and_then(|(p, r)| resolve_via_input(levels, p, r, i, stack))
    } else {
        // An outlet as a source (outlets have no outputs) or a node dropped
        // by an earlier error path: nothing to wire.
        None
    };
    stack.pop();
    resolved
}

/// Resolve the winning source feeding `node`'s `input` at level `lvl`: the
/// oldest edge whose chain resolves (`edges_directed` iterates newest-first,
/// hence the reversal), matching derivation's input resolution.
fn resolve_via_input<N>(
    levels: &[Level<'_, N>],
    lvl: usize,
    node: NodeIx,
    input: usize,
    stack: &mut Vec<SrcKey>,
) -> Option<(NodeIx, usize)> {
    let edges: Vec<_> = levels[lvl]
        .graph
        .edges_directed(node, Direction::Incoming)
        .filter(|e| e.weight().input.0 as usize == input)
        .map(|e| (e.source(), e.weight().output.0 as usize))
        .collect();
    edges
        .into_iter()
        .rev()
        .find_map(|(s, sp)| resolve_src(levels, lvl, s, sp, stack))
}
