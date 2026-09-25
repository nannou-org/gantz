//! Deriving a [`plyphon::SynthDef`] from a connected subgraph of [`NodeDsp`]
//! nodes.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use petgraph::Direction;
use petgraph::visit::EdgeRef;
use plyphon::Rate;
use plyphon::synthdef::{InputRef, Param, SynthDef, UnitSpec};

use gantz_core::compile::pull_eval_order;
use gantz_core::node::Conns;
use gantz_core::node::graph::{Graph, NodeIx};

use crate::dsp::{
    BufferBinding, DspBuilder, FadeSink, Finished, GainRef, NodeDsp, ParamBinding, PortShapes,
    ScopeOutBinding, Signal, ToNodeDsp, record_port_shapes, sum_signals,
};

/// An error deriving a synthdef from a graph.
#[derive(Debug, thiserror::Error)]
pub enum DeriveError {
    /// The graph has no dsp sink, neither an `~out` output nor a `~scopeout`
    /// monitor, so there is nothing to root a synthdef at.
    #[error("no dsp sink (no `~out` output and no `~scopeout` monitor)")]
    NoSink,
    /// The `~bus` or instance boundaries form a cycle between parts, so there
    /// is no writer-before-reader order to derive or run them in. See the
    /// [`instance`](crate::instance) module docs.
    #[error("`~bus`/instance boundaries form a cycle between parts")]
    BusCycle,
    /// An instanced reference's target graph could not be resolved.
    #[error("unresolved instanced reference: {0}")]
    Unresolved(gantz_ca::ContentAddr),
    /// Instanced references form a cycle, a graph transitively instancing
    /// itself, so there is no finite template to derive.
    #[error("instanced references form a cycle through {0}")]
    RefCycle(gantz_ca::ContentAddr),
}

/// One side of a `~bus` boundary within a region's def. The bus unit's input
/// 0, the bus channel index, is a no-lag control param the driver sets to a
/// driver-allocated private bus via `set_control` after spawning. No def
/// mutation is involved, so [`structural_sig`] stays stable across
/// allocations.
#[derive(Clone, Debug)]
pub struct BusBinding {
    /// The `~bus` node's path, the driver's bus-allocation key. Consecutive
    /// buses alias, so a `~bus` fed directly by another `~bus` shares the
    /// upstream bus and reads name the effective upstream node's path.
    ///
    /// For an implicit endpoint bus this is the endpoint source node's path
    /// instead. See [`output`](Self::output).
    pub node_path: Vec<usize>,
    /// The bus's channel count, the boundary signal's width.
    pub channels: usize,
    /// The index within the def's `units` of the bus `Out` on the write side
    /// or `In` on the read side. Its input 0 is the bus-index param.
    pub unit: usize,
    /// The no-lag control param the driver sets to the allocated bus channel
    /// via `set_control` after spawning.
    pub param: usize,
    /// `None` for a classic single-writer `~bus`, keyed by the effective bus
    /// node. `Some(port)` for an implicit endpoint bus. When several summands
    /// feed a boundary, the `~bus` keeps only its cut role. Each transitive
    /// endpoint gets its own single-writer bus, keyed by the endpoint source's
    /// path and output port. Readers emit one `In` per endpoint and sum them.
    pub output: Option<usize>,
}

/// A cross-region bus identity within [`derive_synthdefs`]. A classic
/// single-writer `~bus` chain is keyed by the effective bus node. An implicit
/// per-endpoint bus is keyed by the endpoint source node and output port
/// where the chain fans out. See [`BusBinding::output`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RegionBus {
    Bus(NodeIx),
    Src(NodeIx, usize),
}

/// One region of a boundary-cut graph. It holds the derived synthdef with its
/// bindings, plus the buses its def writes and reads. [`derive_synthdefs`]
/// produces regions in region-DAG topological order, bus writers before their
/// readers. Their synths must take the same order in the node tree.
pub struct RegionDerived {
    /// A stable identity across re-derives, hashed from the region's sink and
    /// boundary node paths. The driver matches old and new regions by key for
    /// its per-region keep/replace decision.
    pub key: u64,
    /// The region's synthdef and its bindings.
    pub derived: Derived,
    /// The buses this region's def writes through patchable `Out` units.
    pub bus_writes: Vec<BusBinding>,
    /// The buses this region's def reads through patchable `In` units.
    pub bus_reads: Vec<BusBinding>,
}

/// The output of [`derive_synthdef`]. It holds the synthdef plus the bindings
/// the audio driver uses to bridge dsp node state and the running synth. A
/// [`ParamBinding`] pushes a dsp node's live value to a synth param via
/// `set_control`. A [`ScopeOutBinding`] routes a monitor's samples back into
/// node state.
pub struct Derived {
    /// The compiled synth definition.
    pub def: SynthDef,
    /// One binding per control param, in param-index order.
    pub params: Vec<ParamBinding>,
    /// One binding per `~scopeout` monitor.
    pub monitors: Vec<ScopeOutBinding>,
    /// The fade gains that gate the def's whole output. The driver ramps them
    /// on a crossfaded replacement.
    pub gains: Vec<GainRef>,
    /// One binding per buffer source emitted in the def. The driver installs
    /// each buffer and wires the source's `bufnum` param.
    pub buffers: Vec<BufferBinding>,
    /// The width and rate each dsp output port carried, for diagnostics.
    pub shapes: PortShapes,
}

/// Derive a [`SynthDef`] named `name` from a graph's DSP subgraph, fanning the
/// output across `out_channels` channels.
///
/// A dsp port carries a whole channel group, a [`Signal`]. An edge delivers
/// its source port's full group to the destination input, so channel width
/// flows forward through the derivation. Nodes see their input widths and
/// size their output groups accordingly.
///
/// A graph's dsp sinks are its `~out` outputs and its `~scopeout` monitors,
/// see [`is_output`](crate::NodeDsp::is_output) and
/// [`is_monitor`](crate::NodeDsp::is_monitor). A graph may have several of
/// each. Every sink seeds a pull over its dsp inputs in gantz_core's
/// [`pull_eval_order`], the same order Steel uses. The per-sink orders merge,
/// first occurrence wins. The merge preserves a valid topological order of
/// the whole DSP subgraph. Each node then emits its UGens via
/// [`NodeDsp::ugens`] once, threading its outputs into
/// its consumers' inputs. A signal feeding both `~out` and a `~scopeout`
/// therefore compiles into one shared unit chain.
///
/// Each sink's pull is seeded with its `n_dsp_inputs`, not `n_inputs`. A
/// control edge at a higher input index, such as `~out`'s gain, falls outside
/// the traversal. It is a Steel/state concern, not part of the dsp signal
/// graph. The same rule applies at interior nodes. Only nodes that feed a
/// sink transitively through dsp inputs contribute units, so a dsp chain
/// wired into a control input emits nothing. Dead units would add params the
/// driver drives and would churn [`structural_sig`]. A hybrid dsp input, see
/// [`NodeDsp::n_dsp_inputs`], is part of the
/// traversal. A dsp chain wired into it emits units and drives the input
/// directly. Its fallback param is only baked while no dsp source is
/// connected.
///
/// Nested graphs derive through a pre-derivation pass.
/// [`flatten`](crate::flatten()) resolves graph refs and splices their nodes
/// into a single flat graph. Each node carries its original nested path via
/// [`ToNodeDsp::node_path`].
///
/// Multiple edges into one dsp input sum. The input's value is the unity-gain
/// mix of every incoming edge via [`sum_signals`]. The result is as wide as
/// the widest summand, a mono summand broadcasts across every channel and a
/// narrower one contributes silence past its own width. Summands sort by
/// source node path and output port, so the derived def is independent of
/// edge insertion order. A single edge passes through unit-free.
///
/// Feedback cycles are not supported. See the [`instance`](crate::instance)
/// module docs.
pub fn derive_synthdef<N>(
    graph: &Graph<N>,
    out_channels: usize,
    name: impl Into<String>,
) -> Result<Derived, DeriveError>
where
    N: ToNodeDsp,
{
    let sinks = dsp_sinks(graph);
    if sinks.is_empty() {
        return Err(DeriveError::NoSink);
    }
    let reachable = dsp_reachable(graph, &sinks);
    let sources = resolved_sources(graph, &reachable);

    // Merge each sink's dsp-only pull-eval order into one topological order,
    // keeping only dsp-reachable nodes and the first occurrence of each.
    let order = merged_pull_order(graph, &sinks, |n| reachable.contains(&n));

    let mut builder = DspBuilder::new(out_channels);
    // Each processed node's per-port output signals, for its consumers to
    // reference. A whole channel group flows across an edge.
    let mut outputs: HashMap<NodeIx, Vec<Signal>> = HashMap::new();
    let mut shapes = PortShapes::new();

    for n in order {
        let Some(dsp) = graph[n].to_node_dsp() else {
            continue;
        };
        let mut inputs: Vec<Option<Signal>> = Vec::with_capacity(sources[&n].len());
        for (input_ix, summands) in sources[&n].iter().enumerate() {
            if dsp.is_buffer_input(input_ix) {
                let feed = buffer_feed(graph, n, input_ix);
                inputs.push(feed.and_then(|src| {
                    emit_local(graph, src, &mut builder, &mut outputs, &mut shapes)
                }));
                continue;
            }
            let sigs: Vec<Signal> = summands
                .iter()
                .filter_map(|&(s, port)| outputs.get(&s).and_then(|o| o.get(port)).cloned())
                .collect();
            // `None` iff no summand materialized a signal, for example an
            // unconnected input or a dangling `~unpack` port. Hybrid inputs
            // then fall back to their param, exactly when the Steel side
            // keeps it driven.
            inputs.push((!sigs.is_empty()).then(|| sum_signals(&mut builder, &sigs)));
        }
        let path = graph[n].node_path(n.index());
        let outs = dsp.ugens(&path, &inputs, &mut builder);
        debug_assert_eq!(
            outs.len(),
            dsp.n_dsp_outputs(),
            "a node must return one Signal per dsp output port",
        );
        record_port_shapes(&mut shapes, &builder, &path, &outs);
        outputs.insert(n, outs);
    }

    let Finished {
        def,
        params,
        monitors,
        gains,
        buffers,
    } = builder.finish(name);
    Ok(Derived {
        def,
        params,
        monitors,
        gains,
        buffers,
        shapes,
    })
}

/// Whether `d` roots a synthdef pull: an output, a monitor or a buffer writer.
pub(crate) fn is_sink(d: &dyn NodeDsp) -> bool {
    d.is_output() || d.is_monitor() || d.is_writer()
}

/// Every dsp sink of `graph`, that is every `~out` output, `~scopeout`
/// monitor and buffer writer. Writers come first, so that in one def a
/// buffer write runs before the reads that follow it.
pub(crate) fn dsp_sinks<N: ToNodeDsp>(graph: &Graph<N>) -> Vec<NodeIx> {
    let (mut writers, others): (Vec<NodeIx>, Vec<NodeIx>) = graph
        .node_indices()
        .filter(|&n| graph[n].to_node_dsp().is_some_and(is_sink))
        .partition(|&n| graph[n].to_node_dsp().is_some_and(|d| d.is_writer()));
    writers.extend(others);
    writers
}

/// Whether dsp input `input` of `n` takes a bufnum wire. See
/// [`NodeDsp::is_buffer_input`].
pub(crate) fn is_buffer_input<N: ToNodeDsp>(graph: &Graph<N>, n: NodeIx, input: usize) -> bool {
    graph[n]
        .to_node_dsp()
        .is_some_and(|d| d.is_buffer_input(input))
}

/// Whether `n` is a local buffer source. See [`NodeDsp::is_buffer_source`].
pub(crate) fn is_buffer_source<N: ToNodeDsp>(graph: &Graph<N>, n: NodeIx) -> bool {
    graph[n].to_node_dsp().is_some_and(|d| d.is_buffer_source())
}

/// The buffer source port that feeds buffer input `input` of `n`. The feed
/// can pass through a chain of `~bus` nodes. A buffer wire never becomes a
/// bus, so the source is emitted on the reading side instead. `None` unless
/// exactly one edge feeds each step of the chain and the chain ends at a
/// buffer source.
pub(crate) fn buffer_feed<N: ToNodeDsp>(
    graph: &Graph<N>,
    n: NodeIx,
    input: usize,
) -> Option<(NodeIx, usize)> {
    let mut target = (n, input);
    let mut visited = HashSet::new();
    loop {
        let mut edges = graph
            .edges_directed(target.0, Direction::Incoming)
            .filter(|e| e.weight().input.0 as usize == target.1);
        let e = edges.next()?;
        if edges.next().is_some() {
            return None;
        }
        let s = e.source();
        let dsp = graph[s].to_node_dsp()?;
        if dsp.is_buffer_source() {
            return Some((s, e.weight().output.0 as usize));
        }
        if !dsp.is_boundary() || !visited.insert(s) {
            return None;
        }
        target = (s, 0);
    }
}

/// The signal at output `port` of buffer source `s` in the def under
/// construction. The source is emitted on its first use in a def and its
/// outputs are cached in `outputs` for later uses.
pub(crate) fn emit_local<N: ToNodeDsp>(
    graph: &Graph<N>,
    (s, port): (NodeIx, usize),
    builder: &mut DspBuilder,
    outputs: &mut HashMap<NodeIx, Vec<Signal>>,
    shapes: &mut PortShapes,
) -> Option<Signal> {
    if !outputs.contains_key(&s) {
        let dsp = graph[s].to_node_dsp()?;
        let path = graph[s].node_path(s.index());
        let outs = dsp.ugens(&path, &[], builder);
        record_port_shapes(shapes, builder, &path, &outs);
        outputs.insert(s, outs);
    }
    outputs.get(&s).and_then(|o| o.get(port)).cloned()
}

/// The dsp-reachable set, the dsp nodes that feed a sink transitively through
/// dsp inputs only. `pull_eval_order` masks only the seed's inputs and
/// traverses interior nodes over every incoming edge. Derivation intersects
/// its merged orders with this set to keep control-input feeds out of the
/// defs.
///
/// Buffer inputs and buffer sources are left out. A buffer source is emitted
/// on demand per def instead, see [`emit_local`], so it never joins a region.
fn dsp_reachable<N: ToNodeDsp>(graph: &Graph<N>, sinks: &[NodeIx]) -> HashSet<NodeIx> {
    let mut reachable: HashSet<NodeIx> = sinks.iter().copied().collect();
    let mut stack: Vec<NodeIx> = sinks.to_vec();
    while let Some(n) = stack.pop() {
        let n_dsp_in = graph[n].to_node_dsp().map_or(0, |d| d.n_dsp_inputs());
        for e in graph.edges_directed(n, Direction::Incoming) {
            let input_ix = e.weight().input.0 as usize;
            let s = e.source();
            if input_ix < n_dsp_in
                && !is_buffer_input(graph, n, input_ix)
                && graph[s].to_node_dsp().is_some()
                && !is_buffer_source(graph, s)
                && reachable.insert(s)
            {
                stack.push(s);
            }
        }
    }
    reachable
}

/// The summand `(source node, output port)`s per dsp input of every reachable
/// node. Only reachable dsp sources contribute. Every edge into an input is a
/// summand and an empty list is an unconnected input. Summands sort by source
/// node path and output port with duplicates kept, so the derived def is
/// independent of edge insertion order. A buffer input always has an empty
/// list. Its feed resolves through [`buffer_feed`].
#[allow(clippy::type_complexity)]
fn resolved_sources<N: ToNodeDsp>(
    graph: &Graph<N>,
    reachable: &HashSet<NodeIx>,
) -> HashMap<NodeIx, Vec<Vec<(NodeIx, usize)>>> {
    reachable
        .iter()
        .map(|&n| {
            let n_dsp_in = graph[n].to_node_dsp().map_or(0, |d| d.n_dsp_inputs());
            let mut inputs: Vec<Vec<(NodeIx, usize)>> = vec![Vec::new(); n_dsp_in];
            for e in graph.edges_directed(n, Direction::Incoming) {
                let input_ix = e.weight().input.0 as usize;
                let s = e.source();
                if input_ix < n_dsp_in
                    && !is_buffer_input(graph, n, input_ix)
                    && reachable.contains(&s)
                    && graph[s].to_node_dsp().is_some()
                {
                    inputs[input_ix].push((s, e.weight().output.0 as usize));
                }
            }
            for summands in &mut inputs {
                summands.sort_by_cached_key(|&(s, port)| (graph[s].node_path(s.index()), port));
            }
            (n, inputs)
        })
        .collect()
}

/// Merge each sink's dsp-only pull-eval order into one topological order over
/// the nodes selected by `keep`. First occurrence wins. A filtered subsequence
/// of a topological order remains topological for the kept subgraph.
pub(crate) fn merged_pull_order<N: ToNodeDsp>(
    graph: &Graph<N>,
    seeds: &[NodeIx],
    keep: impl Fn(NodeIx) -> bool,
) -> Vec<NodeIx> {
    let mut order: Vec<NodeIx> = Vec::new();
    let mut seen: HashSet<NodeIx> = HashSet::new();
    for &seed in seeds {
        let n_dsp_in = graph[seed].to_node_dsp().map_or(0, |d| d.n_dsp_inputs());
        let conns = Conns::connected(n_dsp_in).expect("n_dsp_inputs within Conns::MAX");
        for n in pull_eval_order(graph, seed, conns) {
            if keep(n) && seen.insert(n) {
                order.push(n);
            }
        }
    }
    order
}

/// Derive one [`SynthDef`] per boundary-cut region of the graph's DSP
/// subgraph, in region-DAG topological order, bus writers before readers.
///
/// [`derive_synthdef`] fuses the whole DSP subgraph into a single def and
/// lowers boundary nodes as plain wires. This splits it at every cutting
/// `~bus` instead, see [`is_boundary`](crate::NodeDsp::is_boundary). Regions
/// are the connected components of the dsp-reachable subgraph over
/// non-boundary edges. A boundary between two regions lowers to an `Out` to
/// a private bus in the writer's def and an `In` in each reader's. Both carry
/// a no-lag bus-index control param the driver sets via `set_control` after
/// spawning, see [`BusBinding`]. Each region carries its own
/// [`structural_sig`], so an edit respawns only its own region's synth and
/// every other region's unit state survives untouched.
///
/// A boundary whose two sides share a region lowers to a plain wire. A
/// boundary fed directly by another boundary aliases it with no relay def
/// and no extra latency. An unconnected boundary contributes no summand. A
/// boundary fed by several summands keeps only its cut role. Each transitive
/// endpoint writes its own implicit single-writer bus. See
/// [`BusBinding::output`]. Every reader emits one `In` per endpoint and sums
/// them via [`sum_signals`], so mono broadcast reconciles on materialized
/// signals. A region is derived only if it feeds a sink transitively. Bus
/// writes are lifted to audio rate via [`DspBuilder::ensure_audio`] and
/// fade-gained via [`DspBuilder::push_fade_gain`]. Widths flow forward across
/// boundaries, hence the topological derivation order. Defs are named
/// `<name_prefix>-<region key>`.
pub fn derive_synthdefs<N>(
    graph: &Graph<N>,
    out_channels: usize,
    name_prefix: &str,
) -> Result<Vec<RegionDerived>, DeriveError>
where
    N: ToNodeDsp,
{
    let sinks = dsp_sinks(graph);
    if sinks.is_empty() {
        return Err(DeriveError::NoSink);
    }
    let reachable = dsp_reachable(graph, &sinks);
    let sources = resolved_sources(graph, &reachable);
    let is_boundary =
        |n: NodeIx| -> bool { graph[n].to_node_dsp().is_some_and(|d| d.is_boundary()) };

    // Regions are connected components of the reachable non-boundary nodes
    // over their dsp edges. Edges into or out of a boundary never join.
    let mut comp: HashMap<NodeIx, usize> = HashMap::new();
    let mut n_comps = 0;
    for start in graph.node_indices() {
        if !reachable.contains(&start) || is_boundary(start) || comp.contains_key(&start) {
            continue;
        }
        let id = n_comps;
        n_comps += 1;
        comp.insert(start, id);
        let mut stack = vec![start];
        while let Some(n) = stack.pop() {
            // Upstream, this node's summand sources.
            for &(s, _) in sources[&n].iter().flatten() {
                if !is_boundary(s) && comp.insert(s, id).is_none() {
                    stack.push(s);
                }
            }
            // Downstream, reachable non-boundary consumers with this node
            // among that input's summands.
            for e in graph.edges_directed(n, Direction::Outgoing) {
                let t = e.target();
                if !reachable.contains(&t) || is_boundary(t) || comp.contains_key(&t) {
                    continue;
                }
                let input_ix = e.weight().input.0 as usize;
                let among = sources[&t]
                    .get(input_ix)
                    .is_some_and(|ss| ss.iter().any(|&(s, _)| s == n));
                if among {
                    comp.insert(t, id);
                    stack.push(t);
                }
            }
        }
    }

    // Boundary lowering. A pure single-summand chain of boundaries keeps the
    // classic single-writer bus identity. Consecutive buses alias, keyed by
    // the effective, top-most bus node. A boundary whose chain fans out keeps
    // only its cut role. Each transitive non-boundary endpoint gets its own
    // implicit single-writer bus and readers sum after their `In`s. Width
    // reconciliation needs locally materialized signals, since a writer
    // cannot know the sum's final width at its own derive time. A pure
    // boundary cycle degrades to an unsourced, silent bus.
    let boundaries: Vec<NodeIx> = graph
        .node_indices()
        .filter(|&n| reachable.contains(&n) && is_boundary(n))
        .collect();
    let effective = |b: NodeIx| -> NodeIx {
        let mut cur = b;
        let mut visited = HashSet::new();
        while let Some(&[(s, _)]) = sources[&cur].first().map(|v| v.as_slice()) {
            if !is_boundary(s) || !visited.insert(cur) {
                break;
            }
            cur = s;
        }
        cur
    };
    // The classic case. The effective bus's lone summand is a non-boundary
    // source and every hop of the chain had exactly one. `None` means the
    // chain fans out somewhere, is unsourced, or is a pure bus cycle.
    let classic_source = |b: NodeIx| -> Option<(NodeIx, usize)> {
        match sources[&effective(b)].first().map(|v| v.as_slice()) {
            Some(&[(s, port)]) if !is_boundary(s) => Some((s, port)),
            _ => None,
        }
    };
    // Every transitive non-boundary endpoint feeding `b`, in canonical order.
    // Duplicates are kept since each is a summand. Empty means unsourced.
    let bus_endpoints = |b: NodeIx| -> Vec<(NodeIx, usize)> {
        let mut endpoints = Vec::new();
        let mut visited = HashSet::new();
        let mut stack = vec![b];
        while let Some(cur) = stack.pop() {
            if !visited.insert(cur) {
                continue;
            }
            for &(s, port) in sources[&cur].iter().flatten() {
                match is_boundary(s) {
                    true => stack.push(s),
                    false => endpoints.push((s, port)),
                }
            }
        }
        endpoints.sort_by_cached_key(|&(s, port)| (graph[s].node_path(s.index()), port));
        endpoints
    };
    // The buses a boundary lowers to, each with its writing source.
    let region_buses = |b: NodeIx| -> Vec<(RegionBus, (NodeIx, usize))> {
        match classic_source(b) {
            Some(src) => vec![(RegionBus::Bus(effective(b)), src)],
            None => bus_endpoints(b)
                .into_iter()
                .map(|(s, port)| (RegionBus::Src(s, port), (s, port)))
                .collect(),
        }
    };

    // Cross-region reads as (reader component, bus) pairs, from every boundary
    // summand whose bus originates in another component, plus each bus's
    // writing source.
    let mut cross_reads: HashSet<(usize, RegionBus)> = HashSet::new();
    let mut bus_writer: HashMap<RegionBus, (NodeIx, usize)> = HashMap::new();
    for (&n, srcs) in &sources {
        if is_boundary(n) {
            continue;
        }
        for &(s, _) in srcs.iter().flatten() {
            if !is_boundary(s) {
                continue;
            }
            for (bus, (src, port)) in region_buses(s) {
                bus_writer.insert(bus, (src, port));
                if comp[&src] != comp[&n] {
                    cross_reads.insert((comp[&n], bus));
                }
            }
        }
    }

    // Needed components are those holding sinks, plus transitively the writers
    // of every bus a needed component reads.
    let mut needed: HashSet<usize> = sinks.iter().map(|s| comp[s]).collect();
    loop {
        let mut grew = false;
        for &(reader, bus) in &cross_reads {
            if needed.contains(&reader) {
                let (src, _) = bus_writer[&bus];
                grew |= needed.insert(comp[&src]);
            }
        }
        if !grew {
            break;
        }
    }

    // The region DAG from writers to readers over needed components. Kahn's
    // algorithm yields the derivation and node-tree order, or reports a bus
    // cycle.
    let mut deps: HashMap<usize, HashSet<usize>> = HashMap::new(); // readers to writers
    for &(reader, bus) in &cross_reads {
        if !needed.contains(&reader) {
            continue;
        }
        let (src, _) = bus_writer[&bus];
        let writer = comp[&src];
        if writer != reader {
            deps.entry(reader).or_default().insert(writer);
        }
    }
    let mut topo: Vec<usize> = Vec::with_capacity(needed.len());
    let mut placed: HashSet<usize> = HashSet::new();
    // Component ids were assigned in node-index order, so iterating them in
    // order keeps the result deterministic.
    while topo.len() < needed.len() {
        let next = (0..n_comps).find(|c| {
            needed.contains(c)
                && !placed.contains(c)
                && deps
                    .get(c)
                    .is_none_or(|ws| ws.iter().all(|w| placed.contains(w)))
        });
        match next {
            Some(c) => {
                placed.insert(c);
                topo.push(c);
            }
            None => return Err(DeriveError::BusCycle),
        }
    }

    // Derive each region in topo order. Widths flow forward via `bus_width`.
    let mut regions = Vec::with_capacity(topo.len());
    let mut bus_width: HashMap<RegionBus, usize> = HashMap::new();
    for c in topo {
        // The region's roots are its sinks plus the sources of the buses it
        // writes, that is buses sourced here and read from another needed
        // component. Boundary node-index order keeps the write order
        // deterministic.
        let mut writes: Vec<(RegionBus, (NodeIx, usize))> = Vec::new();
        for &b in &boundaries {
            for (bus, src) in region_buses(b) {
                if comp[&src.0] == c
                    && cross_reads
                        .iter()
                        .any(|&(r, rb)| rb == bus && needed.contains(&r))
                    && !writes.iter().any(|&(wb, _)| wb == bus)
                {
                    writes.push((bus, src));
                }
            }
        }
        let region_sinks: Vec<NodeIx> = sinks.iter().copied().filter(|s| comp[s] == c).collect();

        let seeds: Vec<NodeIx> = region_sinks
            .iter()
            .copied()
            .chain(writes.iter().map(|&(_, (s, _))| s))
            .collect();
        let order = merged_pull_order(graph, &seeds, |n| comp.get(&n) == Some(&c));

        let mut builder = DspBuilder::new(out_channels);
        let mut outputs: HashMap<NodeIx, Vec<Signal>> = HashMap::new();
        let mut shapes = PortShapes::new();
        let mut bus_reads: Vec<BusBinding> = Vec::new();
        // One `In` per bus read, shared by every consumer in the region.
        let mut in_signals: HashMap<RegionBus, Signal> = HashMap::new();

        for n in order {
            let Some(dsp) = graph[n].to_node_dsp() else {
                continue;
            };
            // Each input sums its summands. A plain summand wires directly. A
            // boundary summand lowers to its buses, in-region wires or `In`s.
            let mut inputs: Vec<Option<Signal>> = Vec::with_capacity(sources[&n].len());
            for (input_ix, summands) in sources[&n].iter().enumerate() {
                if dsp.is_buffer_input(input_ix) {
                    let feed = buffer_feed(graph, n, input_ix);
                    inputs.push(feed.and_then(|src| {
                        emit_local(graph, src, &mut builder, &mut outputs, &mut shapes)
                    }));
                    continue;
                }
                let mut sigs: Vec<Signal> = Vec::new();
                for &(s, port) in summands {
                    let lowered: Vec<(Option<RegionBus>, (NodeIx, usize))> = match is_boundary(s) {
                        true => region_buses(s)
                            .into_iter()
                            .map(|(bus, src)| (Some(bus), src))
                            .collect(),
                        false => vec![(None, (s, port))],
                    };
                    for (bus, (src, sport)) in lowered {
                        match bus {
                            Some(bus) if comp[&src] != c => {
                                let sig = in_signals
                                    .entry(bus)
                                    .or_insert_with(|| {
                                        let channels = bus_width.get(&bus).copied().unwrap_or(1);
                                        let (path, label, output) = bus_param_at(graph, bus);
                                        let bus_param = builder.push_control_param(&path, &label);
                                        let unit = builder.push_unit(UnitSpec::new(
                                            "In",
                                            Rate::Audio,
                                            vec![InputRef::Param(bus_param)],
                                            channels,
                                        ));
                                        bus_reads.push(BusBinding {
                                            node_path: path,
                                            channels,
                                            unit: unit as usize,
                                            param: bus_param as usize,
                                            output,
                                        });
                                        (0..channels as u32)
                                            .map(|output| InputRef::Unit { unit, output })
                                            .collect()
                                    })
                                    .clone();
                                sigs.push(sig);
                            }
                            // A dangling port materializes nothing.
                            _ => sigs.extend(outputs.get(&src).and_then(|o| o.get(sport)).cloned()),
                        }
                    }
                }
                // `None` iff no summand materialized a signal, whether
                // unconnected, an unsourced boundary or a dangling port. Hybrid
                // inputs then fall back to their param, exactly when the Steel
                // side keeps it driven.
                inputs.push((!sigs.is_empty()).then(|| sum_signals(&mut builder, &sigs)));
            }
            let path = graph[n].node_path(n.index());
            let outs = dsp.ugens(&path, &inputs, &mut builder);
            debug_assert_eq!(
                outs.len(),
                dsp.n_dsp_outputs(),
                "a node must return one Signal per dsp output port",
            );
            record_port_shapes(&mut shapes, &builder, &path, &outs);
            outputs.insert(n, outs);
        }

        // Emit the region's bus writes. Lift each channel to audio, apply one
        // driver fade gain per param path and write to a bus-index param.
        let mut fades: HashMap<Vec<usize>, u32> = HashMap::new();
        let mut bus_writes = Vec::with_capacity(writes.len());
        for (bus, (src, port)) in writes {
            let sig = outputs
                .get(&src)
                .and_then(|o| o.get(port))
                .cloned()
                .unwrap_or_else(|| Signal::silent(1));
            let (path, label, output) = bus_param_at(graph, bus);
            let fade = *fades
                .entry(path.clone())
                .or_insert_with(|| builder.push_fade_gain(&path, FadeSink::Bus));
            let bus_param = builder.push_control_param(&path, &label);
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
            bus_writes.push(BusBinding {
                node_path: path,
                channels: sig.width(),
                unit: unit as usize,
                param: bus_param as usize,
                output,
            });
            bus_width.insert(bus, sig.width());
        }

        // A stable region identity from its sink and boundary roles and node
        // paths. An endpoint bus also hashes its output port. A classic bus
        // hashes nothing extra, so its region key is stable.
        let mut h = DefaultHasher::new();
        for s in &region_sinks {
            (0u8, graph[*s].node_path(s.index())).hash(&mut h);
        }
        for w in &bus_writes {
            (1u8, &w.node_path).hash(&mut h);
            if let Some(o) = w.output {
                o.hash(&mut h);
            }
        }
        for r in &bus_reads {
            (2u8, &r.node_path).hash(&mut h);
            if let Some(o) = r.output {
                o.hash(&mut h);
            }
        }
        let key = h.finish();

        let name = format!("{name_prefix}-{key:016x}");
        let Finished {
            def,
            params,
            monitors,
            gains,
            buffers,
        } = builder.finish(name);
        regions.push(RegionDerived {
            key,
            derived: Derived {
                def,
                params,
                monitors,
                gains,
                buffers,
                shapes,
            },
            bus_writes,
            bus_reads,
        });
    }
    Ok(regions)
}

/// The param path, label and endpoint port of a region bus. A classic bus is
/// keyed by the effective `~bus` node with the plain `"bus"` label. An
/// endpoint bus is keyed by its source node with a port-suffixed `"bus{port}"`
/// label, matching the instancing pipeline's `Src` convention.
fn bus_param_at<N: ToNodeDsp>(
    graph: &Graph<N>,
    bus: RegionBus,
) -> (Vec<usize>, String, Option<usize>) {
    match bus {
        RegionBus::Bus(b) => (graph[b].node_path(b.index()), "bus".to_string(), None),
        RegionBus::Src(s, port) => (
            graph[s].node_path(s.index()),
            format!("bus{port}"),
            Some(port),
        ),
    }
}

/// The content-addressed name for a def with the given [`structural_sig`].
///
/// It is purely a function of the def's structure, with no head or region
/// prefix. Structurally identical defs derived from different heads, or from
/// many instances of one child graph, collide by design. The audio driver's
/// per-name install refcounting then shares one installed def between them.
/// Names change exactly when the structure does.
pub fn content_def_name(sig: u64) -> String {
    format!("gantz-def-{sig:016x}")
}

/// A hash of a synthdef's structure, everything except parameter values.
///
/// Two synthdefs that differ only in their settable [`Param`] defaults share
/// a signature. The audio driver can then `set_control` those values on the
/// running synth rather than respawn it, which preserves phase. A change to
/// the unit graph, the wiring, a baked constant, or a param's name, rate or
/// lag changes the signature and forces a respawn.
pub fn structural_sig(def: &SynthDef) -> u64 {
    let mut h = DefaultHasher::new();
    def.units.len().hash(&mut h);
    for u in &def.units {
        hash_unit(&mut h, u);
    }
    def.params.len().hash(&mut h);
    for p in &def.params {
        hash_param_struct(&mut h, p);
    }
    h.finish()
}

fn hash_unit(h: &mut DefaultHasher, u: &UnitSpec) {
    u.name.hash(h);
    rate_tag(u.rate).hash(h);
    u.num_outputs.hash(h);
    u.special_index.hash(h);
    u.inputs.len().hash(h);
    for i in &u.inputs {
        hash_input(h, i);
    }
}

fn hash_input(h: &mut DefaultHasher, i: &InputRef) {
    match i {
        InputRef::Constant(c) => {
            0u8.hash(h);
            c.to_bits().hash(h);
        }
        InputRef::Param(p) => {
            1u8.hash(h);
            p.hash(h);
        }
        InputRef::Unit { unit, output } => {
            2u8.hash(h);
            unit.hash(h);
            output.hash(h);
        }
    }
}

/// Hash a param's structure, that is its name, rate, trigger flag and lag.
/// The default value is excluded since the driver sets it live via
/// `set_control`.
fn hash_param_struct(h: &mut DefaultHasher, p: &Param) {
    p.name.hash(h);
    rate_tag(p.rate).hash(h);
    p.is_trig.hash(h);
    p.lag.map(f32::to_bits).hash(h);
}

fn rate_tag(rate: Rate) -> u8 {
    match rate {
        Rate::Scalar => 0,
        Rate::Control => 1,
        Rate::Audio => 2,
        Rate::Demand => 3,
    }
}
