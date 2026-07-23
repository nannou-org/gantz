//! Recursive synthdef template derivation for nested-graph instance
//! composition (#295).
//!
//! A flat graph (the output of [`flatten`](crate::flatten::flatten)) may
//! contain [`Flat::Instance`] markers - opaque references to child graphs
//! whose DSP is derived once into shared synthdefs and wired per instance via
//! buses - plus root [`Flat::Inlet`]/[`Flat::Outlet`] markers describing the
//! graph's own interface. This module's [`derive_template`] produces a
//! [`GraphTemplate`]: a topologically ordered list of [`Part`]s (regions and
//! instances) plus the bus wiring connecting them. [`instantiate`] splices a
//! template (and, recursively, its instances' child templates) into a flat
//! [`ResolvedPart`] list with absolute paths - the input the audio driver
//! consumes.
//!
//! # Def reuse
//!
//! One template is derived per [`VariantKey`] - a content-addressed shape
//! combining the child's content address, its connected-inlet widths
//! (unconnected inlets bake mono silence), its consumed-outlet mask and the
//! engine's output channel count. Every instance of the same child at the
//! same shape, in any head, shares one [`GraphTemplate`] (memoised in a
//! [`DefCache`]). Defs are named by structural content
//! ([`content_def_name`]), so the driver's
//! per-name install refcounting installs each shared def once and spawns it
//! per instance, with per-synth wiring set live via `set_control`.
//!
//! # Composition
//!
//! Defs never reference defs. Composition lives entirely in the template's
//! wiring metadata - [`BusKey`]s on region reads/writes and instance inlets -
//! which the driver realises as buses. The child's body derives from the
//! committed child graph with child-local param names, so
//! [`structural_sig`] (and hence the def name) is
//! stable across instances, while [`instantiate`] prefixes every binding path
//! with the instance's absolute path for the driver's param/scope sync.
//!
//! # Staging
//!
//! An instance's def runs as its own synth, so a node feeding an instance
//! must run *before* it and a node reading it *after*. A diamond such as
//! `src -> instance -> mix` plus `src -> mix` therefore cannot fuse `src` and
//! `mix` into one def. Each node is assigned a *stage* - the maximum over its
//! summand sources, bumped when crossing an instance - and regions are the
//! connected components *within a stage*. An edge crossing stages (or
//! otherwise crossing regions) lowers to an implicit bus
//! ([`BusKey::Src`]), costing one bus and no latency (writers run before
//! readers within a block). Cycles *through an instance* have no such order
//! and are rejected as [`DeriveError::BusCycle`] (deliberate feedback is
//! planned to land with `InFeedback` lowering, #293).
//!
//! # Summing
//!
//! Multiple edges into one dsp input sum ([`sum_signals`]), exactly as in
//! [`derive_synthdefs`]. Every summand of a consumed input is classified
//! (a `Feed`) and the consumer sums the resolved signals - in-region wires
//! and bus `In`s alike - after materializing them. A multi-fed instance
//! *inlet* sums inside the child template (the summand widths are part of the
//! [`VariantKey`], and each summand gets its own [`BusKey::IfaceIn`] bus). A
//! multi-fed root *outlet* exports one bus per summand
//! ([`GraphTemplate::outlets`]) and the enclosing reader sums after its
//! per-summand `In`s ([`BusKey::InstOut`]).

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use petgraph::Direction;
use petgraph::visit::EdgeRef;
use plyphon::Rate;
use plyphon::synthdef::{InputRef, SynthDef, UnitSpec};

use gantz_ca::ContentAddr;
use gantz_core::node::graph::{Graph, NodeIx};

use crate::compile::{
    DeriveError, content_def_name, derive_synthdefs, dsp_sinks, merged_pull_order, structural_sig,
};
use crate::dsp::{
    BufferBinding, DspBuilder, Finished, GainRef, ParamBinding, PortShapes, ScopeOutBinding,
    Signal, ToNodeDsp, record_port_shapes, sum_signals,
};
use crate::flatten::Flat;

/// A content-addressed shape identifying one template variant of a child
/// graph.
///
/// Every instance of the same child at the same inlet-width signature, outlet
/// consumption mask and engine width shares one [`GraphTemplate`]. Unconnected
/// inlets bake mono silence (part of the key, preserving derivation's constant
/// folding); the outlet mask records which outlets are consumed downstream.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VariantKey {
    /// The referenced child graph's commit content address.
    pub child: ContentAddr,
    /// One entry per inlet: the width of each summand feeding it, in canonical
    /// order (empty = unconnected, baked silence). The child sums a multi-fed
    /// inlet's summands where the inlet is consumed.
    pub inlets: Vec<Vec<usize>>,
    /// One entry per outlet: `true` if some downstream node consumes it.
    pub outlets: Vec<bool>,
    /// The engine's output channel count (baked into sinks).
    pub out_channels: usize,
}

/// The identity of one bus within a template, on both its write and read
/// sides. The driver allocates exactly one bus per distinct *absolute* key
/// (see [`instantiate`]), so two sides naming the same key share a bus.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BusKey {
    /// An explicit `~bus` boundary, keyed by the *effective* bus node's path
    /// (consecutive `~bus` nodes alias, exactly as in
    /// [`derive_synthdefs`]).
    Bus(Vec<usize>),
    /// An implicit endpoint bus: the output `output` of the node at `path`,
    /// written where a winning edge crosses a region boundary (a stage cut or
    /// an instance-inlet feed).
    Src {
        /// The source node's path.
        path: Vec<usize>,
        /// The source node's output port.
        output: usize,
    },
    /// One summand of an instance's consumed outlet. Never allocated
    /// directly: [`instantiate`] resolves it through the child template's
    /// [`outlets`](GraphTemplate::outlets) table to the bus actually carrying
    /// that summand's signal (falling back to an unwritten - silent - bus for
    /// a sourceless outlet). A reader emits one `In` per summand and sums.
    InstOut {
        /// The instance marker's path.
        path: Vec<usize>,
        /// The consumed outlet's index.
        outlet: usize,
        /// The summand's index within the outlet's exported buses.
        summand: usize,
    },
    /// One summand of the template's own `inlet`-th inlet. Resolved by
    /// [`instantiate`] to the enclosing graph's feeding bus for that summand.
    IfaceIn {
        /// The inlet's interface index.
        inlet: usize,
        /// The summand's index within the inlet's feeds (canonical order,
        /// matching [`VariantKey::inlets`]).
        summand: usize,
    },
}

/// One side of a bus within a region's def: the `In`/`Out` unit and the no-lag
/// bus-index control param the driver sets via `set_control` after spawning.
#[derive(Clone, Debug)]
pub struct TemplateBus {
    /// The bus's identity (template-relative paths).
    pub key: BusKey,
    /// The bus's channel count (the boundary signal's width).
    pub channels: usize,
    /// The index within the def's `units` of the bus `Out` (write side) or
    /// `In` (read side) whose input 0 is the bus-index param.
    pub unit: usize,
    /// The no-lag control param the driver sets to the allocated bus channel.
    pub param: usize,
}

/// One region of a template: a connected component of DSP nodes within a
/// stage, derived as its own synthdef, plus the buses its def writes and
/// reads. Binding paths are template-relative; [`instantiate`] absolutizes
/// them.
pub struct TemplateRegion {
    /// A stable template-relative identity across re-derives: a hash of the
    /// region's sink paths and bus keys. Combined with the instance path
    /// prefix by [`instantiate`] for the driver's keep/replace decision.
    pub key: u64,
    /// The def's [`structural_sig`] (its name is [`content_def_name`] of
    /// this).
    pub sig: u64,
    /// The region's synthdef, shared by every instance of the template.
    pub def: Arc<SynthDef>,
    /// Param bindings (template-relative node paths, def-local indices).
    pub params: Vec<ParamBinding>,
    /// Monitor bindings (template-relative node paths).
    pub monitors: Vec<ScopeOutBinding>,
    /// The def's driver-ramped fade gains.
    pub gains: Vec<GainRef>,
    /// Buffer bindings (template-relative node paths).
    pub buffers: Vec<BufferBinding>,
    /// The buses this region's def writes.
    pub bus_writes: Vec<TemplateBus>,
    /// The buses this region's def reads.
    pub bus_reads: Vec<TemplateBus>,
    /// The width and rate each dsp output port carried (template-relative
    /// node paths), for diagnostics.
    pub shapes: PortShapes,
}

/// One part of a [`GraphTemplate`].
pub enum Part {
    /// A derived region synthdef.
    Region(TemplateRegion),
    /// An instanced nested-graph ref, wiring a shared child variant via buses.
    Instance(InstancePart),
}

/// An instance's identity and wiring within its template. The child's own
/// `In`/`Out` units live in the child template's regions; this records which
/// enclosing bus feeds each connected inlet.
pub struct InstancePart {
    /// The instance marker's template-relative path (its identity for
    /// keep/replace and bus resolution).
    pub path: Vec<usize>,
    /// The variant this instance instantiates (keys the shared template).
    pub variant: VariantKey,
    /// Per inlet: the template-relative key of the bus feeding each summand,
    /// in canonical order (empty = unconnected, baked silence in the variant).
    pub inlet_keys: Vec<Vec<BusKey>>,
}

/// A derived template: its parts in topological order (bus writers before
/// readers) and its own outlet table (for nesting into a parent).
pub struct GraphTemplate {
    /// The parts, in bus-writer-before-reader topological order.
    pub parts: Vec<Part>,
    /// One entry per outlet: the template-relative key + width of the bus
    /// carrying each of the outlet's summands, in canonical order (empty =
    /// unconsumed or sourceless). A parent's read of the instance's outlet
    /// resolves through this table at [`instantiate`] time, so an outlet fed
    /// by a nested instance or passed through from an inlet aliases the
    /// underlying buses with no relay def. A multi-fed outlet exports one bus
    /// per summand and the enclosing reader sums.
    pub outlets: Vec<Vec<(BusKey, usize)>>,
}

/// A memoised cache of derived child templates, keyed by [`VariantKey`].
#[derive(Default)]
pub struct DefCache(HashMap<VariantKey, Arc<GraphTemplate>>);

impl DefCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a cached template by variant key.
    pub fn get(&self, key: &VariantKey) -> Option<Arc<GraphTemplate>> {
        self.0.get(key).cloned()
    }

    /// Insert a derived template (the caller derives it, then caches).
    pub fn insert(&mut self, key: VariantKey, t: Arc<GraphTemplate>) {
        self.0.insert(key, t);
    }

    /// The number of cached variants.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the cache holds no variants.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Resolves a child content address to its committed graph (flattened), for
/// recursive [`derive_template`]. `None` surfaces as
/// [`DeriveError::Unresolved`].
pub type InstanceResolve<'g, N> = dyn Fn(&ContentAddr) -> Option<&'g Graph<Flat<N>>> + 'g;

/// A part with its absolute path resolved - the driver's input.
///
/// Produced by [`instantiate`]: instances dissolve into their child
/// template's regions (spawned per instance from the shared def), so only
/// region-flavoured parts remain, in global bus-writer-before-reader order.
pub struct ResolvedPart {
    /// The part's keep/replace identity: the template region key combined
    /// with the instance path prefix. Stable across re-derives while the
    /// part's sinks and wiring shape stay put.
    pub key: u64,
    /// The def's [`structural_sig`].
    pub sig: u64,
    /// The shared, content-named synthdef.
    pub def: Arc<SynthDef>,
    /// Param bindings with ABSOLUTE node paths (def-local indices).
    pub params: Vec<ParamBinding>,
    /// Monitor bindings with absolute node paths.
    pub monitors: Vec<ScopeOutBinding>,
    /// The def's driver-ramped fade gains.
    pub gains: Vec<GainRef>,
    /// Buffer bindings with absolute node paths.
    pub buffers: Vec<BufferBinding>,
    /// The buses this part's synth writes (absolute keys).
    pub bus_writes: Vec<ResolvedBus>,
    /// The buses this part's synth reads (absolute keys).
    pub bus_reads: Vec<ResolvedBus>,
    /// The width and rate each dsp output port carried (absolute node paths),
    /// for diagnostics.
    pub shapes: PortShapes,
}

/// A [`TemplateBus`] with its key made absolute.
#[derive(Clone, Debug)]
pub struct ResolvedBus {
    /// The bus's absolute identity - the driver's allocation key.
    pub key: BusKey,
    /// The bus's channel count.
    pub channels: usize,
    /// The def-local index of the bus `In`/`Out` unit.
    pub unit: usize,
    /// The def-local no-lag control param carrying the bus index.
    pub param: usize,
}

/// How a flat vertex participates in derivation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A DSP node that joins a region.
    Plain,
    /// A `~bus` boundary.
    Boundary,
    /// An instance marker (`n_inlets`, `n_outlets`).
    Instance(usize, usize),
    /// A root inlet marker (its interface index).
    Inlet(usize),
    /// A root outlet marker (its interface index).
    Outlet(usize),
}

/// Where a consumed input's signal comes from.
enum Feed {
    /// A same-region source: wire it directly.
    Wire(NodeIx, usize),
    /// An external bus: an `In` keyed by `key`, whose write (if any) is owned
    /// by `writer`.
    Read {
        key: BusKey,
        writer: Option<PartId>,
        /// For `Src`/`Bus` keys: the plain node + port whose output the
        /// writer region must emit to the bus.
        demand: Option<(NodeIx, usize)>,
        /// The param path + label of the read-side bus-index param.
        param_at: (Vec<usize>, String),
    },
    /// No source: mono silence.
    Silence,
}

/// A part of the template DAG.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum PartId {
    Region(usize),
    Instance(usize),
}

/// Per (instance path, outlet): the number of summand buses the derived child
/// template exports for that outlet ([`GraphTemplate::outlets`]). Populated in
/// part order (writers precede readers), so a reader's `InstOut` expansion
/// sees its instance's counts.
type OutSummands = HashMap<(Vec<usize>, usize), usize>;

/// Derive a [`GraphTemplate`] for `graph`, recursing into instance children
/// per `resolve` (memoised in `cache`).
///
/// The head graph is the degenerate case: its interface (any root
/// inlet/outlet markers) is all-unconnected, so stray boundaries lower to
/// silence exactly as they always have. When the graph contains no markers at
/// all, this delegates to [`derive_synthdefs`] (handling `~bus` boundaries
/// exactly as today) and re-shapes its regions, renamed to content def names.
pub fn derive_template<N>(
    graph: &Graph<Flat<N>>,
    out_channels: usize,
    resolve: &InstanceResolve<'_, N>,
    cache: &mut DefCache,
) -> Result<GraphTemplate, DeriveError>
where
    N: ToNodeDsp,
{
    let (n_inlets, n_outlets) = iface_arity(graph);
    let inlets = vec![Vec::new(); n_inlets];
    let outlets = vec![false; n_outlets];
    let mut deriving = Vec::new();
    derive_parts(
        graph,
        out_channels,
        &inlets,
        &outlets,
        resolve,
        cache,
        &mut deriving,
    )
}

/// The number of root inlet/outlet markers in a flat graph.
fn iface_arity<N>(graph: &Graph<Flat<N>>) -> (usize, usize) {
    let mut n_in = 0;
    let mut n_out = 0;
    for n in graph.node_indices() {
        match &graph[n] {
            Flat::Inlet { .. } => n_in += 1,
            Flat::Outlet { .. } => n_out += 1,
            _ => {}
        }
    }
    (n_in, n_out)
}

/// Whether the child committed at `ca` (transitively) contains a DSP sink -
/// an instance of it is then itself a sink (it produces audio through the
/// child's own `~out`/`~scopeout`, no parent wiring needed).
fn child_has_sink<N>(
    ca: &ContentAddr,
    resolve: &InstanceResolve<'_, N>,
    memo: &mut HashMap<ContentAddr, bool>,
    stack: &mut Vec<ContentAddr>,
) -> Result<bool, DeriveError>
where
    N: ToNodeDsp,
{
    if let Some(&known) = memo.get(ca) {
        return Ok(known);
    }
    if stack.contains(ca) {
        // A ref cycle cannot introduce a sink its members do not already
        // contain; the cycle itself errors if such an instance ever derives.
        return Ok(false);
    }
    let graph = resolve(ca).ok_or(DeriveError::Unresolved(*ca))?;
    stack.push(*ca);
    let mut has = !dsp_sinks(graph).is_empty();
    if !has {
        for n in graph.node_indices() {
            if let Flat::Instance { child_ca, .. } = &graph[n] {
                if child_has_sink(child_ca, resolve, memo, stack)? {
                    has = true;
                    break;
                }
            }
        }
    }
    stack.pop();
    memo.insert(*ca, has);
    Ok(has)
}

/// The unified template derivation: classify, reach, stage, cut into
/// per-stage regions, order the part DAG and derive each part with its bus
/// wiring. `inlets`/`outlets_consumed` describe the interface shape the
/// enclosing graph observes (all-unconnected/unconsumed for the head).
#[allow(clippy::too_many_lines)]
fn derive_parts<N>(
    graph: &Graph<Flat<N>>,
    out_channels: usize,
    inlets: &[Vec<usize>],
    outlets_consumed: &[bool],
    resolve: &InstanceResolve<'_, N>,
    cache: &mut DefCache,
    deriving: &mut Vec<ContentAddr>,
) -> Result<GraphTemplate, DeriveError>
where
    N: ToNodeDsp,
{
    // Fast path: no markers at all is exactly `derive_synthdefs` (the
    // pre-instancing pipeline), re-shaped with content def names.
    let any_marker = graph
        .node_indices()
        .any(|n| !matches!(&graph[n], Flat::Node { .. }));
    if !any_marker {
        let regions = derive_synthdefs(graph, out_channels, "gantz")?;
        let parts = regions
            .into_iter()
            .map(|r| {
                let mut def = r.derived.def;
                let sig = structural_sig(&def);
                def.name = content_def_name(sig);
                let to_template = |b: crate::compile::BusBinding| TemplateBus {
                    key: match b.output {
                        None => BusKey::Bus(b.node_path),
                        Some(output) => BusKey::Src {
                            path: b.node_path,
                            output,
                        },
                    },
                    channels: b.channels,
                    unit: b.unit,
                    param: b.param,
                };
                Part::Region(TemplateRegion {
                    key: r.key,
                    sig,
                    def: Arc::new(def),
                    params: r.derived.params,
                    monitors: r.derived.monitors,
                    gains: r.derived.gains,
                    buffers: r.derived.buffers,
                    bus_writes: r.bus_writes.into_iter().map(to_template).collect(),
                    bus_reads: r.bus_reads.into_iter().map(to_template).collect(),
                    shapes: r.derived.shapes,
                })
            })
            .collect();
        return Ok(GraphTemplate {
            parts,
            outlets: Vec::new(),
        });
    }

    // Classify every derivation-relevant vertex.
    let mut kind: HashMap<NodeIx, Kind> = HashMap::new();
    let mut instance_ixs: Vec<NodeIx> = Vec::new();
    let mut inlet_paths: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut outlet_ixs: HashMap<usize, NodeIx> = HashMap::new();
    for n in graph.node_indices() {
        match &graph[n] {
            Flat::Node { node, .. } => {
                if let Some(dsp) = node.to_node_dsp() {
                    let k = if dsp.is_boundary() {
                        Kind::Boundary
                    } else {
                        Kind::Plain
                    };
                    kind.insert(n, k);
                }
            }
            Flat::Instance {
                n_inlets,
                n_outlets,
                ..
            } => {
                kind.insert(n, Kind::Instance(*n_inlets, *n_outlets));
                instance_ixs.push(n);
            }
            Flat::Inlet { index, path } => {
                kind.insert(n, Kind::Inlet(*index));
                inlet_paths.insert(*index, path.clone());
            }
            Flat::Outlet { index, .. } => {
                kind.insert(n, Kind::Outlet(*index));
                outlet_ixs.insert(*index, n);
            }
        }
    }
    let n_dsp_in = |n: NodeIx| -> usize {
        match kind.get(&n) {
            Some(Kind::Plain) | Some(Kind::Boundary) => match &graph[n] {
                Flat::Node { node, .. } => node.to_node_dsp().map_or(0, |d| d.n_dsp_inputs()),
                _ => 0,
            },
            Some(Kind::Instance(n_in, _)) => *n_in,
            Some(Kind::Outlet(_)) => 1,
            Some(Kind::Inlet(_)) | None => 0,
        }
    };
    // Whether a vertex can source a signal (outlets never do).
    let is_source_kind = |n: NodeIx| -> bool {
        matches!(
            kind.get(&n),
            Some(Kind::Plain)
                | Some(Kind::Boundary)
                | Some(Kind::Instance(..))
                | Some(Kind::Inlet(_))
        )
    };

    // Seeds: plain sinks, instances whose child transitively holds a sink,
    // and consumed root outlets.
    let mut seeds: Vec<NodeIx> = dsp_sinks(graph);
    {
        let mut memo = HashMap::new();
        let mut stack = Vec::new();
        for &inst in &instance_ixs {
            if let Flat::Instance { child_ca, .. } = &graph[inst] {
                if child_has_sink(child_ca, resolve, &mut memo, &mut stack)? {
                    seeds.push(inst);
                }
            }
        }
    }
    for (j, &consumed) in outlets_consumed.iter().enumerate() {
        if consumed {
            if let Some(&o) = outlet_ixs.get(&j) {
                seeds.push(o);
            }
        }
    }
    if seeds.is_empty() {
        return Err(DeriveError::NoSink);
    }

    // Reach: backward from the seeds through dsp inputs.
    let mut reach: HashSet<NodeIx> = seeds.iter().copied().collect();
    let mut stack: Vec<NodeIx> = seeds.clone();
    while let Some(n) = stack.pop() {
        let n_in = n_dsp_in(n);
        for e in graph.edges_directed(n, Direction::Incoming) {
            if (e.weight().input.0 as usize) < n_in
                && is_source_kind(e.source())
                && reach.insert(e.source())
            {
                stack.push(e.source());
            }
        }
    }

    // Summand sources per dsp input, reach-filtered: every edge into an input
    // is a summand (empty = unconnected), canonically ordered (sorted by
    // source path + output port) so derivation is independent of edge
    // insertion order.
    let srcs: HashMap<NodeIx, Vec<Vec<(NodeIx, usize)>>> = reach
        .iter()
        .map(|&n| {
            let n_in = n_dsp_in(n);
            let mut inputs: Vec<Vec<(NodeIx, usize)>> = vec![Vec::new(); n_in];
            for e in graph.edges_directed(n, Direction::Incoming) {
                let input_ix = e.weight().input.0 as usize;
                let s = e.source();
                if input_ix < n_in && reach.contains(&s) && is_source_kind(s) {
                    inputs[input_ix].push((s, e.weight().output.0 as usize));
                }
            }
            for summands in &mut inputs {
                summands.sort_by_cached_key(|&(s, port)| (graph[s].path().to_vec(), port));
            }
            (n, inputs)
        })
        .collect();

    // Stages, via the SCC condensation of the winning-edge graph so cycles
    // that were previously legal (pure `~bus` cycles, in-region node cycles)
    // stay legal: an SCC's members share a stage, and only an SCC containing
    // an instance - a cycle through an instance - is an error.
    let stage = {
        let mut temp = petgraph::graph::DiGraph::<NodeIx, (), usize>::default();
        let mut to_temp: HashMap<NodeIx, petgraph::graph::NodeIndex<usize>> = HashMap::new();
        let mut ordered: Vec<NodeIx> = reach.iter().copied().collect();
        ordered.sort();
        for &n in &ordered {
            to_temp.insert(n, temp.add_node(n));
        }
        for &n in &ordered {
            for &(s, _) in srcs[&n].iter().flatten() {
                temp.add_edge(to_temp[&s], to_temp[&n], ());
            }
        }
        let sccs = petgraph::algo::tarjan_scc(&temp);
        let mut scc_of: HashMap<NodeIx, usize> = HashMap::new();
        for (i, scc) in sccs.iter().enumerate() {
            for &t in scc {
                scc_of.insert(temp[t], i);
            }
        }
        let mut stage: HashMap<NodeIx, usize> = HashMap::new();
        let mut scc_stage: HashMap<usize, usize> = HashMap::new();
        // `tarjan_scc` returns SCCs in reverse topological order.
        for (i, scc) in sccs.iter().enumerate().rev() {
            let cyclic = scc.len() > 1
                || scc
                    .iter()
                    .any(|&t| srcs[&temp[t]].iter().flatten().any(|&(s, _)| s == temp[t]));
            if cyclic
                && scc
                    .iter()
                    .any(|&t| matches!(kind.get(&temp[t]), Some(Kind::Instance(..))))
            {
                return Err(DeriveError::BusCycle);
            }
            let mut s = 0;
            for &t in scc {
                let n = temp[t];
                for &(src, _) in srcs[&n].iter().flatten() {
                    let src_scc = scc_of[&src];
                    if src_scc != i {
                        let bump = usize::from(matches!(kind.get(&src), Some(Kind::Instance(..))));
                        s = s.max(scc_stage[&src_scc] + bump);
                    }
                }
            }
            scc_stage.insert(i, s);
            for &t in scc {
                stage.insert(temp[t], s);
            }
        }
        stage
    };

    // Regions: connected components of reachable Plain vertices over their
    // winning dsp edges, within a stage (a cross-stage edge never joins).
    let mut comp: HashMap<NodeIx, usize> = HashMap::new();
    let mut n_comps = 0;
    let plain = |n: NodeIx| kind.get(&n) == Some(&Kind::Plain);
    for start in graph.node_indices() {
        if !reach.contains(&start) || !plain(start) || comp.contains_key(&start) {
            continue;
        }
        let id = n_comps;
        n_comps += 1;
        comp.insert(start, id);
        let mut stack = vec![start];
        while let Some(n) = stack.pop() {
            for &(s, _) in srcs[&n].iter().flatten() {
                if plain(s) && stage[&s] == stage[&n] && comp.insert(s, id).is_none() {
                    stack.push(s);
                }
            }
            for e in graph.edges_directed(n, Direction::Outgoing) {
                let t = e.target();
                if !reach.contains(&t)
                    || !plain(t)
                    || comp.contains_key(&t)
                    || stage[&t] != stage[&n]
                {
                    continue;
                }
                let input_ix = e.weight().input.0 as usize;
                let among = srcs[&t]
                    .get(input_ix)
                    .is_some_and(|ss| ss.iter().any(|&(s, _)| s == n));
                if among {
                    comp.insert(t, id);
                    stack.push(t);
                }
            }
        }
    }
    let inst_part: HashMap<NodeIx, usize> = instance_ixs
        .iter()
        .filter(|n| reach.contains(n))
        .enumerate()
        .map(|(i, &n)| (n, i))
        .collect();
    let reachable_instances: Vec<NodeIx> = {
        let mut v: Vec<NodeIx> = inst_part.keys().copied().collect();
        v.sort_by_key(|n| inst_part[n]);
        v
    };

    // `~bus` aliasing + sourcing, exactly as `derive_synthdefs`: a *pure*
    // single-summand chain of boundaries keeps the classic effective-bus
    // identity, while a fanned-out chain lowers to its transitive endpoints
    // (each a summand the consumer classifies as though wired directly).
    let boundary = |n: NodeIx| kind.get(&n) == Some(&Kind::Boundary);
    let effective = |b: NodeIx| -> NodeIx {
        let mut cur = b;
        let mut visited = HashSet::new();
        while let Some(&[(s, _)]) = srcs[&cur].first().map(|v| v.as_slice()) {
            if !boundary(s) || !visited.insert(cur) {
                break;
            }
            cur = s;
        }
        cur
    };
    // The classic case: the effective bus's lone summand is a non-boundary
    // source (every hop of the chain had exactly one). `None` = the chain
    // fans out somewhere, is unsourced, or is a pure bus cycle.
    let classic_source = |b: NodeIx| -> Option<(NodeIx, usize)> {
        match srcs[&effective(b)].first().map(|v| v.as_slice()) {
            Some(&[(s, port)]) if !boundary(s) => Some((s, port)),
            _ => None,
        }
    };
    // Every transitive non-boundary endpoint feeding `b`, canonical order,
    // duplicates kept (each is a summand). Empty = unsourced (silence).
    let bus_endpoints = |b: NodeIx| -> Vec<(NodeIx, usize)> {
        let mut endpoints = Vec::new();
        let mut visited = HashSet::new();
        let mut stack = vec![b];
        while let Some(cur) = stack.pop() {
            if !visited.insert(cur) {
                continue;
            }
            for &(s, port) in srcs[&cur].iter().flatten() {
                match boundary(s) {
                    true => stack.push(s),
                    false => endpoints.push((s, port)),
                }
            }
        }
        endpoints.sort_by_cached_key(|&(s, port)| (graph[s].path().to_vec(), port));
        endpoints
    };

    // Classify one *direct* (non-boundary) source feeding a consumer. `at` is
    // the consuming part (None for a root outlet or instance inlet, which
    // belong to no part). An inlet source expands to one read per enclosing
    // summand; an instance-outlet source to one read per exported summand
    // (`out_summands` - a lone silent read before the child is derived or for
    // a sourceless outlet, preserving the In-on-silent-bus shape).
    let direct_feeds = |s: NodeIx,
                        port: usize,
                        at: Option<PartId>,
                        out_summands: &HashMap<(Vec<usize>, usize), usize>|
     -> Vec<Feed> {
        match kind.get(&s) {
            Some(Kind::Plain) => {
                let writer = PartId::Region(comp[&s]);
                if at == Some(writer) {
                    vec![Feed::Wire(s, port)]
                } else {
                    vec![Feed::Read {
                        key: BusKey::Src {
                            path: graph[s].path().to_vec(),
                            output: port,
                        },
                        writer: Some(writer),
                        demand: Some((s, port)),
                        param_at: (graph[s].path().to_vec(), format!("bus{port}")),
                    }]
                }
            }
            Some(Kind::Instance(..)) => {
                let path = graph[s].path().to_vec();
                let n = out_summands
                    .get(&(path.clone(), port))
                    .copied()
                    .unwrap_or(1)
                    .max(1);
                (0..n)
                    .map(|summand| Feed::Read {
                        key: BusKey::InstOut {
                            path: path.clone(),
                            outlet: port,
                            summand,
                        },
                        writer: inst_part.get(&s).map(|&i| PartId::Instance(i)),
                        demand: None,
                        param_at: (
                            path.clone(),
                            summand_label(&format!("out{port}-bus"), summand),
                        ),
                    })
                    .collect()
            }
            Some(Kind::Inlet(i)) => {
                let summands = inlets.get(*i).map(Vec::as_slice).unwrap_or(&[]);
                (0..summands.len())
                    .map(|summand| Feed::Read {
                        key: BusKey::IfaceIn { inlet: *i, summand },
                        writer: None,
                        demand: None,
                        param_at: (
                            inlet_paths.get(i).cloned().unwrap_or_default(),
                            summand_label("bus", summand),
                        ),
                    })
                    .collect()
            }
            _ => vec![Feed::Silence],
        }
    };
    // Classify every read a summand source lowers to. A boundary is
    // transparent: a pure single-summand chain to a cross-region plain source
    // keys the *bus* (its path is the user-facing identity); any other chain
    // lowers to its endpoints, each classified as though wired directly.
    let feeds = |src: (NodeIx, usize),
                 at: Option<PartId>,
                 out_summands: &HashMap<(Vec<usize>, usize), usize>|
     -> Vec<Feed> {
        let (s, port) = src;
        if !boundary(s) {
            return direct_feeds(s, port, at, out_summands);
        }
        let eff = effective(s);
        match classic_source(eff) {
            Some((src, sport)) if kind.get(&src) == Some(&Kind::Plain) => {
                let writer = PartId::Region(comp[&src]);
                if at == Some(writer) {
                    vec![Feed::Wire(src, sport)]
                } else {
                    vec![Feed::Read {
                        key: BusKey::Bus(graph[eff].path().to_vec()),
                        writer: Some(writer),
                        demand: Some((src, sport)),
                        param_at: (graph[eff].path().to_vec(), "bus".to_string()),
                    }]
                }
            }
            _ => bus_endpoints(s)
                .into_iter()
                .flat_map(|(e, eport)| direct_feeds(e, eport, at, out_summands))
                .collect(),
        }
    };

    // Relations: per-part external reads (for the DAG + needed set) and
    // per-region write demands. Reads are gathered from every reachable
    // consumer: region nodes, instance inlets and consumed outlets.
    let mut reads: Vec<(Option<PartId>, BusKey, Option<PartId>)> = Vec::new();
    let mut demands: HashMap<usize, Vec<(BusKey, (NodeIx, usize))>> = HashMap::new();
    let mut demand = |writer: Option<PartId>, key: &BusKey, d: Option<(NodeIx, usize)>| {
        if let (Some(PartId::Region(c)), Some(d)) = (writer, d) {
            let list = demands.entry(c).or_default();
            if !list.iter().any(|(k, _)| k == key) {
                list.push((key.clone(), d));
            }
        }
    };
    // Summand counts are unknown before derivation, so the DAG pass expands
    // `InstOut` reads against an empty map (a single read - enough for the
    // reader/writer relations, which are summand-agnostic).
    let no_summands = OutSummands::new();
    for (&n, inputs) in &srcs {
        let at = match kind.get(&n) {
            Some(Kind::Plain) => Some(PartId::Region(comp[&n])),
            Some(Kind::Instance(..)) => inst_part.get(&n).map(|&i| PartId::Instance(i)),
            Some(Kind::Outlet(j)) => {
                if !outlets_consumed.get(*j).copied().unwrap_or(false) {
                    continue;
                }
                None
            }
            // Boundaries are transparent (resolved through their endpoints by
            // their consumers); inlets consume nothing.
            _ => continue,
        };
        for summands in inputs.iter() {
            for &src in summands {
                for f in feeds(src, at, &no_summands) {
                    if let Feed::Read {
                        key,
                        writer,
                        demand: d,
                        ..
                    } = f
                    {
                        demand(writer, &key, d);
                        reads.push((at, key, writer));
                    }
                }
            }
        }
    }

    // Needed parts: those holding seeds, grown through reads (a needed
    // reader's writer is needed). A root outlet's reader is the enclosing
    // graph (always needed, represented as `None`).
    let mut needed: HashSet<PartId> = HashSet::new();
    for &s in &seeds {
        match kind.get(&s) {
            Some(Kind::Plain) => {
                needed.insert(PartId::Region(comp[&s]));
            }
            Some(Kind::Instance(..)) => {
                if let Some(&i) = inst_part.get(&s) {
                    needed.insert(PartId::Instance(i));
                }
            }
            _ => {}
        }
    }
    loop {
        let mut grew = false;
        for (at, _, writer) in &reads {
            let reader_needed = match at {
                None => true,
                Some(p) => needed.contains(p),
            };
            if reader_needed {
                if let Some(w) = writer {
                    grew |= needed.insert(*w);
                }
            }
        }
        if !grew {
            break;
        }
    }

    // The part DAG (reader -> writers) over needed parts; Kahn's algorithm
    // yields the derivation (and spawn) order, or reports a cycle through an
    // instance boundary.
    let mut deps: HashMap<PartId, HashSet<PartId>> = HashMap::new();
    for (at, _, writer) in &reads {
        if let (Some(r), Some(w)) = (at, writer) {
            if r != w && needed.contains(r) && needed.contains(w) {
                deps.entry(*r).or_default().insert(*w);
            }
        }
    }
    let all_parts: Vec<PartId> = (0..n_comps)
        .map(PartId::Region)
        .chain((0..reachable_instances.len()).map(PartId::Instance))
        .filter(|p| needed.contains(p))
        .collect();
    let mut topo: Vec<PartId> = Vec::with_capacity(all_parts.len());
    let mut placed: HashSet<PartId> = HashSet::new();
    while topo.len() < all_parts.len() {
        let next = all_parts.iter().find(|p| {
            !placed.contains(p)
                && deps
                    .get(p)
                    .is_none_or(|ws| ws.iter().all(|w| placed.contains(w)))
        });
        match next {
            Some(&p) => {
                placed.insert(p);
                topo.push(p);
            }
            None => return Err(DeriveError::BusCycle),
        }
    }

    // Derive each part in topo order, flowing bus widths (and outlet summand
    // counts) forward.
    let mut port_width: HashMap<BusKey, usize> = HashMap::new();
    let mut out_summands = OutSummands::new();
    for (i, ws) in inlets.iter().enumerate() {
        for (summand, &w) in ws.iter().enumerate() {
            port_width.insert(BusKey::IfaceIn { inlet: i, summand }, w);
        }
    }
    let mut parts: Vec<Part> = Vec::with_capacity(topo.len());
    for &p in &topo {
        match p {
            PartId::Region(c) => {
                let region = derive_region(
                    graph,
                    out_channels,
                    c,
                    &comp,
                    &srcs,
                    &feeds,
                    demands.get(&c).map(Vec::as_slice).unwrap_or(&[]),
                    &mut port_width,
                    &out_summands,
                )?;
                parts.push(Part::Region(region));
            }
            PartId::Instance(i) => {
                let inst = reachable_instances[i];
                let part = derive_instance(
                    graph,
                    inst,
                    out_channels,
                    &srcs,
                    &feeds,
                    resolve,
                    cache,
                    deriving,
                    &mut port_width,
                    &mut out_summands,
                )?;
                parts.push(Part::Instance(part));
            }
        }
    }

    // The template's own outlet table: the key + width of the bus carrying
    // each consumed outlet summand's signal.
    let outlets = (0..outlets_consumed.len())
        .map(|j| {
            if !outlets_consumed.get(j).copied().unwrap_or(false) {
                return Vec::new();
            }
            let Some(o) = outlet_ixs.get(&j) else {
                return Vec::new();
            };
            let summands = srcs
                .get(o)
                .and_then(|inputs| inputs.first())
                .cloned()
                .unwrap_or_default();
            summands
                .into_iter()
                .flat_map(|src| feeds(src, None, &out_summands))
                .filter_map(|f| match f {
                    Feed::Read { key, .. } => {
                        let w = port_width.get(&key).copied().unwrap_or(1);
                        Some((key, w))
                    }
                    // An outlet has no part of its own, so a `Wire` feed is
                    // unreachable (its source is always external to `None`).
                    Feed::Wire(..) | Feed::Silence => None,
                })
                .collect()
        })
        .collect();

    Ok(GraphTemplate { parts, outlets })
}

/// Derive one region's synthdef: its nodes in dsp pull order, `In` units for
/// its external reads (each multi-summand input summed after materializing
/// its wires and reads) and fade-gained `Out` units for its demanded writes.
#[allow(clippy::too_many_arguments)]
fn derive_region<N>(
    graph: &Graph<Flat<N>>,
    out_channels: usize,
    c: usize,
    comp: &HashMap<NodeIx, usize>,
    srcs: &HashMap<NodeIx, Vec<Vec<(NodeIx, usize)>>>,
    feeds: &impl Fn((NodeIx, usize), Option<PartId>, &OutSummands) -> Vec<Feed>,
    writes: &[(BusKey, (NodeIx, usize))],
    port_width: &mut HashMap<BusKey, usize>,
    out_summands: &OutSummands,
) -> Result<TemplateRegion, DeriveError>
where
    N: ToNodeDsp,
{
    let at = Some(PartId::Region(c));
    let region_sinks: Vec<NodeIx> = graph
        .node_indices()
        .filter(|n| comp.get(n) == Some(&c))
        .filter(|&n| match &graph[n] {
            Flat::Node { node, .. } => node
                .to_node_dsp()
                .is_some_and(|d| d.is_output() || d.is_monitor()),
            _ => false,
        })
        .collect();
    let seeds: Vec<NodeIx> = region_sinks
        .iter()
        .copied()
        .chain(writes.iter().map(|&(_, (s, _))| s))
        .collect();
    let order = merged_pull_order(graph, &seeds, |n| comp.get(&n) == Some(&c));

    let mut builder = DspBuilder::new(out_channels);
    let mut outputs: HashMap<NodeIx, Vec<Signal>> = HashMap::new();
    let mut shapes = PortShapes::new();
    let mut bus_reads: Vec<TemplateBus> = Vec::new();
    // One `In` per read key, shared by every consumer in the region.
    let mut in_signals: HashMap<BusKey, Signal> = HashMap::new();

    for n in order {
        let node = match &graph[n] {
            Flat::Node { node, .. } => node,
            _ => continue,
        };
        let Some(dsp) = node.to_node_dsp() else {
            continue;
        };
        let path = graph[n].path();
        // Each input sums its summands' resolved signals.
        let mut inputs: Vec<Option<Signal>> = Vec::with_capacity(srcs[&n].len());
        for summands in &srcs[&n] {
            let mut sigs: Vec<Signal> = Vec::new();
            for &src in summands {
                for f in feeds(src, at, out_summands) {
                    let sig = match f {
                        // A dangling port materializes nothing.
                        Feed::Wire(s, port) => {
                            let Some(sig) = outputs.get(&s).and_then(|o| o.get(port)) else {
                                continue;
                            };
                            sig.clone()
                        }
                        Feed::Read { key, param_at, .. } => in_signals
                            .entry(key.clone())
                            .or_insert_with(|| {
                                let channels = port_width.get(&key).copied().unwrap_or(1);
                                let (p_path, p_label) = &param_at;
                                let bus_param = builder.push_control_param(p_path, p_label);
                                let unit = builder.push_unit(UnitSpec::new(
                                    "In",
                                    Rate::Audio,
                                    vec![InputRef::Param(bus_param)],
                                    channels,
                                ));
                                bus_reads.push(TemplateBus {
                                    key: key.clone(),
                                    channels,
                                    unit: unit as usize,
                                    param: bus_param as usize,
                                });
                                (0..channels as u32)
                                    .map(|output| InputRef::Unit { unit, output })
                                    .collect()
                            })
                            .clone(),
                        Feed::Silence => Signal::silent(1),
                    };
                    sigs.push(sig);
                }
            }
            // `None` iff no summand materialized a signal (e.g. an unconnected
            // inlet), so hybrid inputs fall back to their param exactly when
            // the Steel side keeps it driven.
            inputs.push((!sigs.is_empty()).then(|| sum_signals(&mut builder, &sigs)));
        }
        let outs = dsp.ugens(path, &inputs, &mut builder);
        debug_assert_eq!(
            outs.len(),
            dsp.n_dsp_outputs(),
            "a node must return one Signal per dsp output port",
        );
        record_port_shapes(&mut shapes, &builder, path, &outs);
        outputs.insert(n, outs);
    }

    // Emit the demanded bus writes: lift each channel to audio, apply a fade
    // gain (one per source path) and write to a bus-index param.
    let mut fades: HashMap<Vec<usize>, u32> = HashMap::new();
    let mut bus_writes = Vec::with_capacity(writes.len());
    for (key, (src, port)) in writes {
        let sig = outputs
            .get(src)
            .and_then(|o| o.get(*port))
            .cloned()
            .unwrap_or_else(|| Signal::silent(1));
        let (fade_path, param_at) = match key {
            BusKey::Bus(bus_path) => (bus_path.clone(), (bus_path.clone(), "bus".to_string())),
            BusKey::Src { path, output } => (path.clone(), (path.clone(), format!("bus{output}"))),
            // Only `Bus`/`Src` writes are ever demanded of a region.
            BusKey::InstOut { .. } | BusKey::IfaceIn { .. } => continue,
        };
        let fade = *fades
            .entry(fade_path.clone())
            .or_insert_with(|| builder.push_fade_gain(&fade_path));
        let (p_path, p_label) = &param_at;
        let bus_param = builder.push_control_param(p_path, p_label);
        let mut out_inputs = vec![InputRef::Param(bus_param)];
        for ch in sig.channels() {
            let ch = builder.ensure_audio(ch);
            let mul = builder.push_unit(UnitSpec {
                name: "BinaryOpUGen".to_string(),
                rate: Rate::Audio,
                inputs: vec![ch, InputRef::Param(fade)],
                num_outputs: 1,
                special_index: 2,
            });
            out_inputs.push(InputRef::Unit {
                unit: mul,
                output: 0,
            });
        }
        let unit = builder.push_unit(UnitSpec::new("Out", Rate::Audio, out_inputs, 0));
        bus_writes.push(TemplateBus {
            key: key.clone(),
            channels: sig.width(),
            unit: unit as usize,
            param: bus_param as usize,
        });
        port_width.insert(key.clone(), sig.width());
    }

    // A stable region identity: its sink paths and bus keys.
    let mut h = DefaultHasher::new();
    for s in &region_sinks {
        (0u8, graph[*s].path()).hash(&mut h);
    }
    for w in &bus_writes {
        (1u8, &w.key).hash(&mut h);
    }
    for r in &bus_reads {
        (2u8, &r.key).hash(&mut h);
    }
    let key = h.finish();

    let Finished {
        mut def,
        params,
        monitors,
        gains,
        buffers,
    } = builder.finish(String::new());
    let sig = structural_sig(&def);
    def.name = content_def_name(sig);
    Ok(TemplateRegion {
        key,
        sig,
        def: Arc::new(def),
        params,
        monitors,
        gains,
        buffers,
        bus_writes,
        bus_reads,
        shapes,
    })
}

/// Derive one instance's part: build its [`VariantKey`] from the widths its
/// inlet feeds carry and the outlets its consumers demand, recursively derive
/// the child template (cached by variant) and record the inlet wiring.
#[allow(clippy::too_many_arguments)]
fn derive_instance<N>(
    graph: &Graph<Flat<N>>,
    inst: NodeIx,
    out_channels: usize,
    srcs: &HashMap<NodeIx, Vec<Vec<(NodeIx, usize)>>>,
    feeds: &impl Fn((NodeIx, usize), Option<PartId>, &OutSummands) -> Vec<Feed>,
    resolve: &InstanceResolve<'_, N>,
    cache: &mut DefCache,
    deriving: &mut Vec<ContentAddr>,
    port_width: &mut HashMap<BusKey, usize>,
    out_summands: &mut OutSummands,
) -> Result<InstancePart, DeriveError>
where
    N: ToNodeDsp,
{
    let Flat::Instance {
        path,
        child_ca,
        n_inlets,
        n_outlets,
    } = &graph[inst]
    else {
        unreachable!("derive_instance called on a non-instance");
    };
    let path = path.clone();
    let child_ca = *child_ca;

    // Inlet feeds: the key of the bus feeding each inlet summand, and the
    // width that bus carries (its writer derived earlier in topo order).
    let mut inlet_keys: Vec<Vec<BusKey>> = vec![Vec::new(); *n_inlets];
    let mut inlet_widths: Vec<Vec<usize>> = vec![Vec::new(); *n_inlets];
    for (i, summands) in srcs[&inst].iter().enumerate() {
        // A marker's arity is its `n_inlets`, so `i` is always in range.
        for &src in summands {
            for f in feeds(src, None, out_summands) {
                match f {
                    Feed::Read { key, .. } => {
                        inlet_widths[i].push(port_width.get(&key).copied().unwrap_or(1));
                        inlet_keys[i].push(key);
                    }
                    Feed::Wire(..) | Feed::Silence => {}
                }
            }
        }
    }

    // Consumed outlets: any reachable consumer whose winning source is this
    // instance's outlet.
    let mut outlets = vec![false; *n_outlets];
    for inputs in srcs.values() {
        for &(s, port) in inputs.iter().flatten() {
            if s == inst {
                if let Some(o) = outlets.get_mut(port) {
                    *o = true;
                }
            }
        }
    }

    let variant = VariantKey {
        child: child_ca,
        inlets: inlet_widths,
        outlets,
        out_channels,
    };

    // Recursively derive the child template (cached by variant), guarding
    // against a graph transitively instancing itself.
    let child = if let Some(t) = cache.get(&variant) {
        t
    } else {
        if deriving.contains(&child_ca) {
            return Err(DeriveError::RefCycle(child_ca));
        }
        let child_graph = resolve(&child_ca).ok_or(DeriveError::Unresolved(child_ca))?;
        deriving.push(child_ca);
        let result = derive_parts(
            child_graph,
            out_channels,
            &variant.inlets,
            &variant.outlets,
            resolve,
            cache,
            deriving,
        );
        deriving.pop();
        let t = Arc::new(result?);
        cache.insert(variant.clone(), Arc::clone(&t));
        t
    };

    // Continue the width flow: each consumed outlet summand's width and the
    // outlet's summand count, from the child's outlet table.
    for (j, out) in child.outlets.iter().enumerate() {
        out_summands.insert((path.clone(), j), out.len());
        for (summand, &(_, w)) in out.iter().enumerate() {
            port_width.insert(
                BusKey::InstOut {
                    path: path.clone(),
                    outlet: j,
                    summand,
                },
                w,
            );
        }
    }

    Ok(InstancePart {
        path,
        variant,
        inlet_keys,
    })
}

/// Splice `template` (and, recursively, its instances' cached child
/// templates) into a flat [`ResolvedPart`] list in global topological order,
/// with absolute binding paths and bus keys.
///
/// Instances dissolve: each contributes its child template's regions at the
/// instance's path prefix, so N instances of one child yield N spawns of the
/// same shared defs with per-instance wiring. `cache` must be the cache the
/// template derived against.
pub fn instantiate(template: &GraphTemplate, cache: &DefCache) -> Vec<ResolvedPart> {
    let mut out = Vec::new();
    instantiate_into(template, cache, &[], &[], &mut out);
    out
}

/// The recursive splice: returns this template's outlet table with keys made
/// absolute (for the parent's `InstOut` resolution).
fn instantiate_into(
    template: &GraphTemplate,
    cache: &DefCache,
    prefix: &[usize],
    inlet_keys: &[Vec<BusKey>],
    out: &mut Vec<ResolvedPart>,
) -> Vec<Vec<(BusKey, usize)>> {
    // Per template-local instance path: its child's absolutized outlet table.
    let mut inst_outlets: HashMap<Vec<usize>, Vec<Vec<(BusKey, usize)>>> = HashMap::new();

    let prefixed = |p: &[usize]| -> Vec<usize> {
        let mut abs = prefix.to_vec();
        abs.extend_from_slice(p);
        abs
    };
    // Absolutize a template-local key. `InstOut` resolves through the named
    // instance's child outlet table (writers precede readers in part order,
    // so the entry exists by the time a reader needs it); a sourceless outlet
    // falls back to a dedicated unwritten - silent - bus key.
    let abs_key =
        |key: &BusKey, inst_outlets: &HashMap<Vec<usize>, Vec<Vec<(BusKey, usize)>>>| -> BusKey {
            match key {
                BusKey::Bus(p) => BusKey::Bus(prefixed(p)),
                BusKey::Src { path, output } => BusKey::Src {
                    path: prefixed(path),
                    output: *output,
                },
                BusKey::InstOut {
                    path,
                    outlet,
                    summand,
                } => inst_outlets
                    .get(path)
                    .and_then(|outs| outs.get(*outlet))
                    .and_then(|ss| ss.get(*summand).cloned())
                    .map(|(k, _)| k)
                    .unwrap_or_else(|| BusKey::InstOut {
                        path: prefixed(path),
                        outlet: *outlet,
                        summand: *summand,
                    }),
                BusKey::IfaceIn { inlet, summand } => inlet_keys
                    .get(*inlet)
                    .and_then(|ks| ks.get(*summand))
                    .cloned()
                    .expect("an IfaceIn read implies a connected inlet summand in the variant"),
            }
        };

    for part in &template.parts {
        match part {
            Part::Region(r) => {
                let mut h = DefaultHasher::new();
                (prefix, r.key).hash(&mut h);
                let key = h.finish();
                let abs_bus = |b: &TemplateBus| ResolvedBus {
                    key: abs_key(&b.key, &inst_outlets),
                    channels: b.channels,
                    unit: b.unit,
                    param: b.param,
                };
                out.push(ResolvedPart {
                    key,
                    sig: r.sig,
                    def: Arc::clone(&r.def),
                    params: r
                        .params
                        .iter()
                        .map(|b| ParamBinding {
                            node_path: prefixed(&b.node_path),
                            key: b.key.clone(),
                            index: b.index,
                        })
                        .collect(),
                    monitors: r
                        .monitors
                        .iter()
                        .map(|m| ScopeOutBinding {
                            node_path: prefixed(&m.node_path),
                            ..m.clone()
                        })
                        .collect(),
                    gains: r.gains.clone(),
                    buffers: r
                        .buffers
                        .iter()
                        .map(|b| BufferBinding {
                            node_path: prefixed(&b.node_path),
                            ..b.clone()
                        })
                        .collect(),
                    bus_writes: r.bus_writes.iter().map(abs_bus).collect(),
                    bus_reads: r.bus_reads.iter().map(abs_bus).collect(),
                    shapes: r
                        .shapes
                        .iter()
                        .map(|((path, port), shape)| ((prefixed(path), *port), *shape))
                        .collect(),
                });
            }
            Part::Instance(ip) => {
                let child = cache
                    .get(&ip.variant)
                    .expect("derive_template populates the cache for every instance part");
                let child_prefix = prefixed(&ip.path);
                let child_inlets: Vec<Vec<BusKey>> = ip
                    .inlet_keys
                    .iter()
                    .map(|ks| ks.iter().map(|k| abs_key(k, &inst_outlets)).collect())
                    .collect();
                let outs = instantiate_into(&child, cache, &child_prefix, &child_inlets, out);
                inst_outlets.insert(ip.path.clone(), outs);
            }
        }
    }

    template
        .outlets
        .iter()
        .map(|ss| {
            ss.iter()
                .map(|(k, w)| (abs_key(k, &inst_outlets), *w))
                .collect()
        })
        .collect()
}

/// A per-summand bus param label: summand 0 keeps the bare `base` (def-sig
/// stability for pre-summing single-summand shapes), later summands suffix
/// their index.
fn summand_label(base: &str, summand: usize) -> String {
    match summand {
        0 => base.to_string(),
        j => format!("{base}{j}"),
    }
}
