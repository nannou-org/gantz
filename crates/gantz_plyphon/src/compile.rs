//! Deriving a [`plyphon::SynthDef`] from a connected subgraph of [`NodeDsp`](crate::NodeDsp)
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

use crate::dsp::{DspBuilder, GainRef, ParamBinding, ScopeOutBinding, Signal, ToNodeDsp};

/// An error deriving a synthdef from a graph.
#[derive(Debug)]
pub enum DeriveError {
    /// The graph has no dsp *sink* (no `~out` output and no `~scopeout` monitor), so
    /// there is nothing to root a synthdef at.
    NoSink,
    /// The `~bus` boundaries form a cycle between regions, so there is no
    /// writer-before-reader order to derive (or run) them in. Deliberate
    /// cross-region feedback (an `InFeedback`-based bus) is a planned follow-up.
    BusCycle,
}

/// One side of a `~bus` boundary within a region's def: the bus unit whose
/// input 0 (the bus channel index) is a `0.0` placeholder the driver patches to
/// a driver-allocated private bus before installing (the `ScopeOut`-bufnum
/// idiom, so [`structural_sig`] stays stable across allocations).
#[derive(Clone, Debug)]
pub struct BusBinding {
    /// The `~bus` node's path - the driver's bus-allocation key. Consecutive
    /// buses alias (a `~bus` fed directly by another `~bus` shares the upstream
    /// bus), so reads name the *effective* upstream node's path.
    pub node_path: Vec<usize>,
    /// The bus's channel count - the boundary signal's width.
    pub channels: usize,
    /// The index within the def's `units` of the bus `Out` (write side) or `In`
    /// (read side) whose input 0 is the patchable placeholder.
    pub unit: usize,
}

/// One region of a boundary-cut graph: its derived synthdef + bindings, plus
/// the buses its def writes and reads. Produced by [`derive_synthdefs`] in
/// region-DAG topological order (bus writers before their readers - the order
/// their synths must also take in the node tree).
pub struct RegionDerived {
    /// A stable identity across re-derives: a hash of the region's sink and
    /// boundary node paths. The driver matches old and new regions by key for
    /// its per-region keep/replace decision.
    pub key: u64,
    /// The region's synthdef + param/monitor/gain bindings.
    pub derived: Derived,
    /// The buses this region's def writes (`Out` with a patchable bus input).
    pub bus_writes: Vec<BusBinding>,
    /// The buses this region's def reads (`In` with a patchable bus input).
    pub bus_reads: Vec<BusBinding>,
}

/// The output of [`derive_synthdef`]: the synthdef plus the bindings the audio
/// driver uses to bridge dsp node state and the running synth - [`ParamBinding`]s
/// (push each dsp node's live value to a synth param via `set_control`) and
/// [`ScopeOutBinding`]s (route each monitor's `/tr`s back into node state).
pub struct Derived {
    /// The compiled synth definition.
    pub def: SynthDef,
    /// One binding per control param, in param-index order.
    pub params: Vec<ParamBinding>,
    /// One binding per monitor (`~scopeout`), in `SendTrig`-id order.
    pub monitors: Vec<ScopeOutBinding>,
    /// The params that gate the def's whole output (e.g. `~out`'s gain), which
    /// the driver fades through on a crossfaded replacement.
    pub gains: Vec<GainRef>,
}

/// Derive a [`SynthDef`] named `name` from a graph's DSP subgraph, fanning the
/// output across `out_channels` channels.
///
/// A dsp port carries a whole channel group ([`Signal`]): an edge delivers its
/// source port's full group to the destination input, so channel width flows
/// *forward* through the derivation - nodes see their input widths and size
/// their output groups accordingly (no `gantz_core` graph or edge involvement).
///
/// A graph's dsp *sinks* are its `~out` outputs ([`is_output`](crate::NodeDsp::is_output))
/// and its `~scopeout` monitors ([`is_monitor`](crate::NodeDsp::is_monitor)). A graph
/// may have several of each (e.g. an output plus a couple of taps of interior
/// signals). Every sink seeds a pull over its *dsp* inputs in gantz_core's
/// pull-eval order ([`pull_eval_order`]) - the same order Steel uses - and the
/// per-sink orders are merged, first-occurrence wins. That merge preserves a
/// valid topological order of the whole DSP subgraph: within each order a node
/// precedes its dependents, so the earliest occurrence of any source still
/// precedes the earliest occurrence of its consumer. Each node then emits its
/// UGens via [`NodeDsp::ugens`](crate::NodeDsp::ugens) once, threading its outputs
/// into its consumers' inputs, so a signal feeding both `~out` and a `~scopeout`
/// compiles into one shared unit chain.
///
/// Seeding each sink's pull with its `n_dsp_inputs` (not `n_inputs`) means a
/// control edge at a higher input index (e.g. `~out`'s gain, `~scopeout`'s trigger)
/// falls outside the traversal - it is a Steel/state concern, not part of the dsp
/// signal graph. The same rule applies at *interior* nodes: only nodes that feed a
/// sink transitively through dsp inputs contribute units, so a dsp chain wired into
/// a control input (e.g. a `~lag` into `~sinosc`'s freq) emits nothing rather than
/// dead units whose params the driver would drive and whose presence would force
/// spurious respawns (they'd land in [`structural_sig`]).
///
/// Nested graphs are supported via a pre-derivation pass: [`flatten`](crate::flatten)
/// resolves graph refs and splices their nodes into a single flat graph (each
/// carrying its original nested path via [`ToNodeDsp::node_path`]) before
/// derivation runs.
///
/// Phase-1 limitations: a single edge per DSP input (no summing) and acyclic
/// graphs only (no feedback).
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
    // keeping only dsp-reachable nodes and the first occurrence of each (see the
    // fn docs).
    let order = merged_pull_order(graph, &sinks, |n| reachable.contains(&n));

    let mut builder = DspBuilder::new(out_channels);
    // Each processed node's per-port output signals, for its consumers to
    // reference. A whole channel group flows across an edge.
    let mut outputs: HashMap<NodeIx, Vec<Signal>> = HashMap::new();

    for n in order {
        let Some(dsp) = graph[n].to_node_dsp() else {
            continue;
        };
        let inputs: Vec<Signal> = sources[&n]
            .iter()
            .map(|src| {
                src.and_then(|(s, port)| outputs.get(&s).and_then(|o| o.get(port)).cloned())
                    // Unconnected inputs default to mono silence.
                    .unwrap_or_else(|| Signal::silent(1))
            })
            .collect();
        let outs = dsp.ugens(&graph[n].node_path(n.index()), &inputs, &mut builder);
        debug_assert_eq!(
            outs.len(),
            dsp.n_dsp_outputs(),
            "a node must return one Signal per dsp output port",
        );
        outputs.insert(n, outs);
    }

    let (def, params, monitors, gains) = builder.finish(name);
    Ok(Derived {
        def,
        params,
        monitors,
        gains,
    })
}

/// Every dsp sink of `graph`: an audio output (`~out`) or a monitor (`~scopeout`).
fn dsp_sinks<N: ToNodeDsp>(graph: &Graph<N>) -> Vec<NodeIx> {
    graph
        .node_indices()
        .filter(|&n| {
            graph[n]
                .to_node_dsp()
                .is_some_and(|d| d.is_output() || d.is_monitor())
        })
        .collect()
}

/// The dsp-reachable set: dsp nodes that feed a sink transitively through *dsp*
/// inputs only. `pull_eval_order` masks only the seed's inputs - interior nodes
/// are traversed over ALL incoming edges - so derivation intersects its merged
/// orders with this set to keep control-input feeds out of the defs.
fn dsp_reachable<N: ToNodeDsp>(graph: &Graph<N>, sinks: &[NodeIx]) -> HashSet<NodeIx> {
    let mut reachable: HashSet<NodeIx> = sinks.iter().copied().collect();
    let mut stack: Vec<NodeIx> = sinks.to_vec();
    while let Some(n) = stack.pop() {
        let n_dsp_in = graph[n].to_node_dsp().map_or(0, |d| d.n_dsp_inputs());
        for e in graph.edges_directed(n, Direction::Incoming) {
            if (e.weight().input.0 as usize) < n_dsp_in
                && graph[e.source()].to_node_dsp().is_some()
                && reachable.insert(e.source())
            {
                stack.push(e.source());
            }
        }
    }
    reachable
}

/// The winning `(source node, output port)` per dsp input of every reachable
/// node: only reachable dsp sources contribute, and the *first-added* edge to
/// an input wins (no summing of multiple sources yet) - `edges_directed`
/// iterates newest-edge-first, so the reversed pass makes the oldest edge
/// authoritative.
#[allow(clippy::type_complexity)]
fn resolved_sources<N: ToNodeDsp>(
    graph: &Graph<N>,
    reachable: &HashSet<NodeIx>,
) -> HashMap<NodeIx, Vec<Option<(NodeIx, usize)>>> {
    reachable
        .iter()
        .map(|&n| {
            let n_dsp_in = graph[n].to_node_dsp().map_or(0, |d| d.n_dsp_inputs());
            let mut inputs: Vec<Option<(NodeIx, usize)>> = vec![None; n_dsp_in];
            let edges: Vec<_> = graph.edges_directed(n, Direction::Incoming).collect();
            for e in edges.into_iter().rev() {
                let input_ix = e.weight().input.0 as usize;
                let s = e.source();
                if input_ix < n_dsp_in
                    && inputs[input_ix].is_none()
                    && reachable.contains(&s)
                    && graph[s].to_node_dsp().is_some()
                {
                    inputs[input_ix] = Some((s, e.weight().output.0 as usize));
                }
            }
            (n, inputs)
        })
        .collect()
}

/// Merge each sink's dsp-only pull-eval order into one topological order over
/// the nodes selected by `keep`, first occurrence wins (a filtered subsequence
/// of a topological order remains topological for the kept subgraph).
fn merged_pull_order<N: ToNodeDsp>(
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

/// Derive one [`SynthDef`] per boundary-cut *region* of the graph's DSP
/// subgraph, in region-DAG topological order (bus writers before readers).
///
/// Where [`derive_synthdef`] fuses the whole DSP subgraph into a single def
/// (boundary nodes lower as plain wires), this splits it at every cutting
/// `~bus` ([`is_boundary`](crate::NodeDsp::is_boundary)): regions are the
/// connected components of the dsp-reachable subgraph over non-boundary edges,
/// and a boundary between two regions lowers to an `Out` to a private bus in
/// the writer's def and an `In` in each reader's - both with placeholder bus
/// inputs the driver patches post-sig (see [`BusBinding`]). The point: each
/// region carries its own [`structural_sig`], so an edit respawns only its own
/// region's synth and every other region's unit state survives untouched.
///
/// A boundary whose two sides share a region lowers to a plain wire. A
/// boundary fed directly by another boundary *aliases* it (no relay def, no
/// extra latency). An unconnected boundary reads as mono silence. A region is
/// derived only if it feeds a sink transitively. Bus writes are lifted to audio
/// rate ([`DspBuilder::ensure_audio`]) and fade-gained (the crossfade lever,
/// [`DspBuilder::push_fade_gain`]). Widths flow forward across boundaries -
/// hence the topological derivation order. Defs are named
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

    // Regions: connected components of the reachable NON-boundary nodes over
    // their winning dsp edges (edges into or out of a boundary never join).
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
            // Upstream: this node's winning sources.
            for &(s, _) in sources[&n].iter().flatten() {
                if !is_boundary(s) && comp.insert(s, id).is_none() {
                    stack.push(s);
                }
            }
            // Downstream: reachable non-boundary consumers whose winning source
            // for that input is this node.
            for e in graph.edges_directed(n, Direction::Outgoing) {
                let t = e.target();
                if !reachable.contains(&t) || is_boundary(t) || comp.contains_key(&t) {
                    continue;
                }
                let input_ix = e.weight().input.0 as usize;
                if sources[&t].get(input_ix).copied().flatten().map(|(s, _)| s) == Some(n) {
                    comp.insert(t, id);
                    stack.push(t);
                }
            }
        }
    }

    // Each boundary's *effective* bus (consecutive boundaries alias) and that
    // bus's winning source. A pure boundary cycle degrades to an unsourced bus.
    let boundaries: Vec<NodeIx> = graph
        .node_indices()
        .filter(|&n| reachable.contains(&n) && is_boundary(n))
        .collect();
    let effective = |b: NodeIx| -> NodeIx {
        let mut cur = b;
        let mut visited = HashSet::new();
        while let Some(&(s, _)) = sources[&cur].first().and_then(|o| o.as_ref()) {
            if !is_boundary(s) || !visited.insert(cur) {
                break;
            }
            cur = s;
        }
        cur
    };
    // The effective bus's source, unless it is itself a boundary (a cycle).
    let bus_source = |b: NodeIx| -> Option<(NodeIx, usize)> {
        sources[&effective(b)]
            .first()
            .copied()
            .flatten()
            .filter(|&(s, _)| !is_boundary(s))
    };

    // Cross-region reads: (reader component, effective bus), from every winning
    // boundary-sourced input whose bus originates in another component.
    let mut cross_reads: HashSet<(usize, NodeIx)> = HashSet::new();
    for (&n, srcs) in &sources {
        if is_boundary(n) {
            continue;
        }
        for &(s, _) in srcs.iter().flatten() {
            if !is_boundary(s) {
                continue;
            }
            if let Some((src, _)) = bus_source(s) {
                if comp[&src] != comp[&n] {
                    cross_reads.insert((comp[&n], effective(s)));
                }
            }
        }
    }

    // Needed components: those holding sinks, plus (transitively) the writers
    // of every bus a needed component reads.
    let mut needed: HashSet<usize> = sinks.iter().map(|s| comp[s]).collect();
    loop {
        let mut grew = false;
        for &(reader, bus) in &cross_reads {
            if needed.contains(&reader) {
                if let Some((src, _)) = bus_source(bus) {
                    grew |= needed.insert(comp[&src]);
                }
            }
        }
        if !grew {
            break;
        }
    }

    // Region DAG (writer -> reader) over needed components. Kahn's algorithm
    // yields the derivation (and node-tree) order, or reports a bus cycle.
    let mut deps: HashMap<usize, HashSet<usize>> = HashMap::new(); // reader -> writers
    for &(reader, bus) in &cross_reads {
        if !needed.contains(&reader) {
            continue;
        }
        if let Some((src, _)) = bus_source(bus) {
            let writer = comp[&src];
            if writer != reader {
                deps.entry(reader).or_default().insert(writer);
            }
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

    // Derive each region in topo order; widths flow forward via `bus_width`.
    let mut regions = Vec::with_capacity(topo.len());
    let mut bus_width: HashMap<NodeIx, usize> = HashMap::new();
    for c in topo {
        // The region's roots: its sinks, plus the buses it writes (an effective
        // bus sourced here and read from another needed component).
        let region_sinks: Vec<NodeIx> = sinks.iter().copied().filter(|s| comp[s] == c).collect();
        let writes: Vec<NodeIx> = boundaries
            .iter()
            .copied()
            .filter(|&b| effective(b) == b)
            .filter(|&b| bus_source(b).is_some_and(|(s, _)| comp[&s] == c))
            .filter(|&b| {
                cross_reads
                    .iter()
                    .any(|&(r, bus)| bus == b && needed.contains(&r))
            })
            .collect();

        let seeds: Vec<NodeIx> = region_sinks.iter().chain(writes.iter()).copied().collect();
        let order = merged_pull_order(graph, &seeds, |n| comp.get(&n) == Some(&c));

        let mut builder = DspBuilder::new(out_channels);
        let mut outputs: HashMap<NodeIx, Vec<Signal>> = HashMap::new();
        let mut bus_reads: Vec<BusBinding> = Vec::new();
        // One `In` per bus read, shared by every consumer in the region.
        let mut in_signals: HashMap<NodeIx, Signal> = HashMap::new();

        for n in order {
            let Some(dsp) = graph[n].to_node_dsp() else {
                continue;
            };
            let inputs: Vec<Signal> = sources[&n]
                .iter()
                .map(|src| match src {
                    // A boundary source: a wire within the region, an `In` from
                    // another region's bus, or silence when unsourced.
                    Some((s, _)) if is_boundary(*s) => match bus_source(*s) {
                        Some((src, port)) if comp[&src] == c => outputs
                            .get(&src)
                            .and_then(|o| o.get(port))
                            .cloned()
                            .unwrap_or_else(|| Signal::silent(1)),
                        Some(_) => {
                            let bus = effective(*s);
                            in_signals
                                .entry(bus)
                                .or_insert_with(|| {
                                    let channels = bus_width.get(&bus).copied().unwrap_or(1);
                                    let unit = builder.push_unit(UnitSpec::new(
                                        "In",
                                        Rate::Audio,
                                        vec![InputRef::Constant(0.0)],
                                        channels,
                                    ));
                                    bus_reads.push(BusBinding {
                                        node_path: graph[bus].node_path(bus.index()),
                                        channels,
                                        unit: unit as usize,
                                    });
                                    (0..channels as u32)
                                        .map(|output| InputRef::Unit { unit, output })
                                        .collect()
                                })
                                .clone()
                        }
                        None => Signal::silent(1),
                    },
                    Some((s, port)) => outputs
                        .get(s)
                        .and_then(|o| o.get(*port))
                        .cloned()
                        .unwrap_or_else(|| Signal::silent(1)),
                    None => Signal::silent(1),
                })
                .collect();
            let outs = dsp.ugens(&graph[n].node_path(n.index()), &inputs, &mut builder);
            debug_assert_eq!(
                outs.len(),
                dsp.n_dsp_outputs(),
                "a node must return one Signal per dsp output port",
            );
            outputs.insert(n, outs);
        }

        // Emit the region's bus writes: lift each channel to audio, apply the
        // driver's fade gain, and write to a placeholder bus.
        let mut bus_writes = Vec::with_capacity(writes.len());
        for b in writes {
            let (src, port) = bus_source(b).expect("writes are sourced");
            let sig = outputs
                .get(&src)
                .and_then(|o| o.get(port))
                .cloned()
                .unwrap_or_else(|| Signal::silent(1));
            let fade = builder.push_fade_gain(&graph[b].node_path(b.index()));
            let mut out_inputs = vec![InputRef::Constant(0.0)];
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
                node_path: graph[b].node_path(b.index()),
                channels: sig.width(),
                unit: unit as usize,
            });
            bus_width.insert(b, sig.width());
        }

        // A stable region identity: its sink and boundary roles + node paths.
        let mut h = DefaultHasher::new();
        for s in &region_sinks {
            (0u8, graph[*s].node_path(s.index())).hash(&mut h);
        }
        for w in &bus_writes {
            (1u8, &w.node_path).hash(&mut h);
        }
        for r in &bus_reads {
            (2u8, &r.node_path).hash(&mut h);
        }
        let key = h.finish();

        let name = format!("{name_prefix}-{key:016x}");
        let (def, params, monitors, gains) = builder.finish(name);
        regions.push(RegionDerived {
            key,
            derived: Derived {
                def,
                params,
                monitors,
                gains,
            },
            bus_writes,
            bus_reads,
        });
    }
    Ok(regions)
}

/// A hash of a synthdef's *structure* - everything except parameter values.
///
/// Two synthdefs that differ only in their [`Param`] defaults (the settable
/// values) share a signature, so the audio driver can `set_control` those values
/// on the running synth rather than respawning it (preserving phase). A change to
/// the unit graph, the wiring, a baked constant, or a param's name/rate/lag *does*
/// change the signature, forcing a respawn.
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

/// Hash a param's structure - name, rate, trigger flag and lag - but NOT its
/// default value (which the driver sets live via `set_control`).
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
