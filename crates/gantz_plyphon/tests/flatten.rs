//! Tests that `flatten` splices nested graphs into a flat graph derivation
//! understands: boundary bridging, original-path preservation, error cases and
//! an offline render of a nested graph through the real engine.

use std::collections::HashMap;

use gantz_ca::ContentAddr;
use gantz_core::edge::Edge;
use gantz_core::node::graph::{Graph, NodeIx};
use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, parse_expr};
use gantz_plyphon::flatten::{Flat, FlattenError, RefKind, flatten};
use gantz_plyphon::{
    AddAction, Bus, Lag, NodeDsp, Out, ROOT_GROUP_ID, ScopeOut, SinOsc, ToNodeDsp, derive_synthdef,
    derive_synthdefs, structural_sig,
};
use petgraph::Direction;
use petgraph::visit::EdgeRef;
use plyphon::{Options, World, engine};

const SR: f32 = 48_000.0;

/// A minimal erased node enum, standing in for the app's `Box<dyn Node>`:
/// the DSP nodes plus the nesting machinery (`Inlet`/`Outlet`/`Ref`) and a
/// non-DSP `Other` stand-in.
#[derive(Clone)]
enum N {
    SinOsc(SinOsc),
    Lag(Lag),
    Out(Out),
    ScopeOut(ScopeOut),
    Bus(Bus),
    Inlet,
    Outlet,
    Ref(ContentAddr),
    /// A DSP-aware ref: the child CA + the `inline` flag.
    DspRef(ContentAddr, bool),
    Other,
}

impl ToNodeDsp for N {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        match self {
            N::SinOsc(s) => Some(s),
            N::Lag(l) => Some(l),
            N::Out(o) => Some(o),
            N::ScopeOut(t) => Some(t),
            N::Bus(b) => Some(b),
            N::Inlet | N::Outlet | N::Ref(_) | N::DspRef(_, _) | N::Other => None,
        }
    }
}

impl gantz_core::Node for N {
    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        parse_expr("'()")
    }

    fn inlet(&self, _ctx: MetaCtx) -> bool {
        matches!(self, N::Inlet)
    }

    fn outlet(&self, _ctx: MetaCtx) -> bool {
        matches!(self, N::Outlet)
    }
}

fn ca(byte: u8) -> ContentAddr {
    ContentAddr([byte; 32])
}

/// A `Resolve` closure over a map of committed graphs: `Ref` nodes resolve
/// (missing entries surface as `Unresolved`), everything else is concrete.
fn resolver<'g>(
    map: &'g HashMap<ContentAddr, Graph<N>>,
) -> impl Fn(&N) -> Option<(ContentAddr, RefKind, Option<&'g Graph<N>>)> + 'g {
    move |n| match n {
        N::Ref(ca) => Some((*ca, RefKind::Inline, map.get(ca))),
        N::DspRef(ca, inline) => {
            let kind = if *inline {
                RefKind::Inline
            } else {
                RefKind::Instance
            };
            Some((*ca, kind, map.get(ca)))
        }
        _ => None,
    }
}

fn flatten_with<'g>(
    graph: &'g Graph<N>,
    map: &'g HashMap<ContentAddr, Graph<N>>,
) -> Graph<Flat<&'g N>> {
    try_flatten(graph, map).expect("flatten")
}

fn try_flatten<'g>(
    graph: &'g Graph<N>,
    map: &'g HashMap<ContentAddr, Graph<N>>,
) -> Result<Graph<Flat<&'g N>>, FlattenError> {
    let resolve = resolver(map);
    flatten(&|_| None, graph, &resolve)
}

/// The flat node with the given original path.
fn at<'a>(flat: &'a Graph<Flat<&N>>, path: &[usize]) -> NodeIx {
    flat.node_indices()
        .find(|&n| flat[n].path() == path)
        .unwrap_or_else(|| panic!("no flat node at path {path:?}"))
}

/// The `(source path, output, input)` of every edge into the node at `path`.
fn edges_into(flat: &Graph<Flat<&N>>, path: &[usize]) -> Vec<(Vec<usize>, u16, u16)> {
    let n = at(flat, path);
    let mut edges: Vec<_> = flat
        .edges_directed(n, Direction::Incoming)
        .map(|e| {
            let w = e.weight();
            (flat[e.source()].path().to_vec(), w.output.0, w.input.0)
        })
        .collect();
    edges.reverse();
    edges
}

/// A child graph `inlet -> ~lag -> outlet`.
fn lag_child() -> Graph<N> {
    let mut g = Graph::<N>::default();
    let i = g.add_node(N::Inlet);
    let l = g.add_node(N::Lag(Lag::default()));
    let o = g.add_node(N::Outlet);
    g.add_edge(i, l, Edge::new(0.into(), 0.into()));
    g.add_edge(l, o, Edge::new(0.into(), 0.into()));
    g
}

/// A child graph `inlet -> outlet` (a pure pass-through wire).
fn wire_child() -> Graph<N> {
    let mut g = Graph::<N>::default();
    let i = g.add_node(N::Inlet);
    let o = g.add_node(N::Outlet);
    g.add_edge(i, o, Edge::new(0.into(), 0.into()));
    g
}

#[test]
fn flat_graph_flattens_to_identity() {
    // No refs: every node is copied with its flat path and every edge kept, and
    // the derived def is structurally identical to deriving the raw graph.
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let other = g.add_node(N::Other);
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, o, Edge::new(0.into(), 0.into()));
    g.add_edge(other, o, Edge::new(0.into(), 1.into()));

    let map = HashMap::new();
    let flat = flatten_with(&g, &map);
    assert_eq!(flat.node_count(), 3);
    assert_eq!(flat.edge_count(), 2);
    for n in flat.node_indices() {
        assert_eq!(flat[n].path(), vec![n.index()], "flat paths are [ix]");
    }

    let raw = derive_synthdef(&g, 1, "t").expect("derive raw");
    let flt = derive_synthdef(&flat, 1, "t").expect("derive flat");
    assert_eq!(structural_sig(&raw.def), structural_sig(&flt.def));
}

#[test]
fn splices_a_nested_child() {
    // parent: sin -> ref -> out, child: inlet -> lag -> outlet. The lag splices
    // in carrying its nested path, boundary edges bridge to direct edges, and
    // its dur param is named and bound by that path.
    let map = HashMap::from([(ca(1), lag_child())]);
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1)));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, o, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    assert_eq!(flat.node_count(), 3, "sin + spliced lag + out");
    assert_eq!(edges_into(&flat, &[1, 1]), vec![(vec![0], 0, 0)]);
    assert_eq!(edges_into(&flat, &[2]), vec![(vec![1, 1], 0, 0)]);

    let derived = derive_synthdef(&flat, 1, "t").expect("derive");
    let dur = derived
        .def
        .params
        .iter()
        .find(|p| p.name.ends_with("/dur"))
        .expect("lag dur param");
    assert_eq!(
        dur.name, "1-1/dur",
        "param named by the original nested path"
    );
    assert!(
        derived.params.iter().any(|b| b.node_path == vec![1, 1]),
        "binding keyed by the original nested path",
    );
}

#[test]
fn inlet_fans_out_to_every_consumer() {
    // child: one inlet feeding two lags. The one parent edge into the ref
    // becomes an edge to each consumer.
    let mut child = Graph::<N>::default();
    let i = child.add_node(N::Inlet);
    let l0 = child.add_node(N::Lag(Lag::default()));
    let l1 = child.add_node(N::Lag(Lag::default()));
    child.add_edge(i, l0, Edge::new(0.into(), 0.into()));
    child.add_edge(i, l1, Edge::new(0.into(), 0.into()));
    let map = HashMap::from([(ca(1), child)]);

    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1)));
    g.add_edge(s, r, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    assert_eq!(edges_into(&flat, &[1, 1]), vec![(vec![0], 0, 0)]);
    assert_eq!(edges_into(&flat, &[1, 2]), vec![(vec![0], 0, 0)]);
}

#[test]
fn pass_through_wire_dissolves() {
    // child: inlet -> outlet. The ref dissolves into a direct parent edge.
    let map = HashMap::from([(ca(1), wire_child())]);
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1)));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, o, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    assert_eq!(flat.node_count(), 2);
    assert_eq!(edges_into(&flat, &[2]), vec![(vec![0], 0, 0)]);
}

#[test]
fn two_levels_splice_and_pass_through() {
    // grandchild: inlet -> lag -> outlet. child: inlet -> ref(grandchild) ->
    // outlet. The lag carries its two-level path and the double boundary
    // bridges to direct edges.
    let mut child = Graph::<N>::default();
    let i = child.add_node(N::Inlet);
    let r = child.add_node(N::Ref(ca(1)));
    let o = child.add_node(N::Outlet);
    child.add_edge(i, r, Edge::new(0.into(), 0.into()));
    child.add_edge(r, o, Edge::new(0.into(), 0.into()));
    let map = HashMap::from([(ca(1), lag_child()), (ca(2), child)]);

    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(2)));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, o, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    assert_eq!(flat.node_count(), 3, "sin + doubly nested lag + out");
    assert_eq!(edges_into(&flat, &[1, 1, 1]), vec![(vec![0], 0, 0)]);
    assert_eq!(edges_into(&flat, &[2]), vec![(vec![1, 1, 1], 0, 0)]);
}

#[test]
fn unconnected_boundaries_dissolve_to_silence() {
    // The ref's input is unconnected (the child lag's input dissolves to
    // nothing) and a second ref has no outlet to source the out's input from.
    // Both sides resolve to no edge, deferring to derivation's silence.
    let mut no_outlet = Graph::<N>::default();
    no_outlet.add_node(N::Inlet);
    let map = HashMap::from([(ca(1), lag_child()), (ca(2), no_outlet)]);

    let mut g = Graph::<N>::default();
    let r1 = g.add_node(N::Ref(ca(1)));
    let r2 = g.add_node(N::Ref(ca(2)));
    let o = g.add_node(N::Out(Out::default()));
    let o2 = g.add_node(N::Out(Out::default()));
    g.add_edge(r1, o, Edge::new(0.into(), 0.into()));
    g.add_edge(r2, o2, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    assert!(edges_into(&flat, &[0, 1]).is_empty(), "unconnected inlet");
    assert_eq!(edges_into(&flat, &[2]), vec![(vec![0, 1], 0, 0)]);
    assert!(edges_into(&flat, &[3]).is_empty(), "no outlet to source");
    derive_synthdef(&flat, 1, "t").expect("still derives");
}

#[test]
fn multi_instance_refs_are_independent() {
    // The same committed child spliced twice: two lags with distinct paths and
    // independent param bindings.
    let map = HashMap::from([(ca(1), lag_child())]);
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let r1 = g.add_node(N::Ref(ca(1)));
    let r2 = g.add_node(N::Ref(ca(1)));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, r1, Edge::new(0.into(), 0.into()));
    g.add_edge(r1, r2, Edge::new(0.into(), 0.into()));
    g.add_edge(r2, o, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    assert_eq!(edges_into(&flat, &[1, 1]), vec![(vec![0], 0, 0)]);
    assert_eq!(edges_into(&flat, &[2, 1]), vec![(vec![1, 1], 0, 0)]);
    assert_eq!(edges_into(&flat, &[3]), vec![(vec![2, 1], 0, 0)]);

    let derived = derive_synthdef(&flat, 1, "t").expect("derive");
    for path in [vec![1, 1], vec![2, 1]] {
        assert!(
            derived.params.iter().any(|b| b.node_path == path),
            "expected an independent binding at {path:?}",
        );
    }
}

#[test]
fn ref_cycle_and_unresolved_are_errors() {
    // a refs b refs a: a cycle. A ref to an uncommitted address: unresolved.
    let mut a = Graph::<N>::default();
    a.add_node(N::Ref(ca(2)));
    let mut b = Graph::<N>::default();
    b.add_node(N::Ref(ca(1)));
    let map = HashMap::from([(ca(1), a), (ca(2), b)]);

    let mut g = Graph::<N>::default();
    g.add_node(N::Ref(ca(1)));
    assert!(matches!(
        try_flatten(&g, &map),
        Err(FlattenError::RefCycle(_)),
    ));

    let mut g = Graph::<N>::default();
    g.add_node(N::Ref(ca(9)));
    assert!(matches!(
        try_flatten(&g, &HashMap::new()),
        Err(FlattenError::Unresolved(c)) if c == ca(9),
    ));
}

#[test]
fn flatten_error_displays_readably() {
    // Flatten failures surface in the UI, so each variant formats as a
    // readable message rather than a `Debug` dump.
    assert_eq!(
        FlattenError::RefCycle(ca(9)).to_string(),
        format!("graph reference resolves through itself: {}", ca(9)),
    );
    assert_eq!(
        FlattenError::Unresolved(ca(9)).to_string(),
        format!("unresolved graph reference: {}", ca(9)),
    );
}

#[test]
fn boundary_wiring_cycle_dissolves() {
    // Two pass-through refs wired into a loop (sharing one committed child -
    // no *ref* cycle). Resolution terminates and the consumer dissolves
    // unconnected rather than looping.
    let map = HashMap::from([(ca(1), wire_child())]);
    let mut g = Graph::<N>::default();
    let r1 = g.add_node(N::Ref(ca(1)));
    let r2 = g.add_node(N::Ref(ca(1)));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(r1, r2, Edge::new(0.into(), 0.into()));
    g.add_edge(r2, r1, Edge::new(0.into(), 0.into()));
    g.add_edge(r2, o, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    assert_eq!(flat.node_count(), 1, "only the out survives");
    assert!(edges_into(&flat, &[2]).is_empty());
}

#[test]
fn every_edge_bridges_across_a_boundary() {
    // Two sources feed the ref's one input: BOTH chains bridge (oldest first),
    // so derivation sums them - no edge is dropped at the boundary.
    let map = HashMap::from([(ca(1), wire_child())]);
    let mut g = Graph::<N>::default();
    let s_old = g.add_node(N::SinOsc(SinOsc::default()));
    let s_new = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1)));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s_old, r, Edge::new(0.into(), 0.into()));
    g.add_edge(s_new, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, o, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    assert_eq!(
        edges_into(&flat, &[3]),
        vec![(vec![0], 0, 0), (vec![1], 0, 0)],
        "both sources feed the consumer",
    );

    // Derivation sums the bridged summands: one add ties both sines into
    // the out.
    let def = derive_synthdef(&flat, 1, "t").expect("derive").def;
    assert_eq!(def.units.iter().filter(|u| u.name == "SinOsc").count(), 2);
    let adds = def
        .units
        .iter()
        .filter(|u| u.name == "BinaryOpUGen" && u.special_index == 0)
        .count();
    assert_eq!(adds, 1, "the bridged summands sum at the input");
}

#[test]
fn duplicate_chains_bridge_as_two_summands() {
    // A child whose inlet fans to its outlet twice: both chains resolve to
    // the same source, and both bridge - two distinct wires deliberately sum
    // the signal twice (doubling), the honest reading of the patch.
    let mut child = Graph::<N>::default();
    let i = child.add_node(N::Inlet);
    let o = child.add_node(N::Outlet);
    child.add_edge(i, o, Edge::new(0.into(), 0.into()));
    child.add_edge(i, o, Edge::new(0.into(), 0.into()));
    let map = HashMap::from([(ca(1), child)]);

    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1)));
    let out = g.add_node(N::Out(Out::default()));
    g.add_edge(s, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, out, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    assert_eq!(
        edges_into(&flat, &[2]),
        vec![(vec![0], 0, 0), (vec![0], 0, 0)],
        "each chain is a summand, even to the same source",
    );
}

#[test]
fn nested_sinks_and_buses_derive_with_stable_keys() {
    // A nested `~bus` cuts regions exactly as a flat one would, with bindings
    // and keys carrying the full nested path, and an unrelated parent addition
    // keeps every region key (no spurious respawns).
    let mut child = Graph::<N>::default();
    let i = child.add_node(N::Inlet);
    let b = child.add_node(N::Bus(Bus::default()));
    let o = child.add_node(N::Outlet);
    child.add_edge(i, b, Edge::new(0.into(), 0.into()));
    child.add_edge(b, o, Edge::new(0.into(), 0.into()));
    let map = HashMap::from([(ca(1), child)]);

    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1)));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, o, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    let regions = derive_synthdefs(&flat, 1, "t").expect("derive");
    assert_eq!(regions.len(), 2, "the nested bus cuts writer from reader");
    assert_eq!(regions[0].bus_writes[0].node_path, vec![1, 1]);
    assert_eq!(regions[1].bus_reads[0].node_path, vec![1, 1]);

    // An unrelated appended node re-flattens with every path (hence key) intact.
    let keys: Vec<u64> = regions.iter().map(|r| r.key).collect();
    g.add_node(N::Other);
    let flat = flatten_with(&g, &map);
    let regions = derive_synthdefs(&flat, 1, "t").expect("re-derive");
    let keys2: Vec<u64> = regions.iter().map(|r| r.key).collect();
    assert_eq!(keys, keys2, "unrelated additions keep region keys");
}

#[test]
fn nested_scopeout_binds_by_nested_path() {
    // A monitor inside the child roots a synthdef pull and its binding names
    // the nested path (where the driver streams the ring state to).
    let mut child = Graph::<N>::default();
    let i = child.add_node(N::Inlet);
    let t = child.add_node(N::ScopeOut(ScopeOut::default()));
    child.add_edge(i, t, Edge::new(0.into(), 0.into()));
    let map = HashMap::from([(ca(1), child)]);

    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1)));
    g.add_edge(s, r, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    let derived = derive_synthdef(&flat, 1, "t").expect("derive");
    assert_eq!(derived.monitors.len(), 1);
    assert_eq!(derived.monitors[0].node_path, vec![1, 1]);
}

#[test]
fn nested_synth_plays_expected_tone() {
    // The whole pipeline end to end: a sine committed inside a child graph
    // sounds through the parent's `~out` when rendered by the real engine.
    let mut child = Graph::<N>::default();
    let s = child.add_node(N::SinOsc(SinOsc::default()));
    let o = child.add_node(N::Outlet);
    child.add_edge(s, o, Edge::new(0.into(), 0.into()));
    let map = HashMap::from([(ca(1), child)]);

    let mut g = Graph::<N>::default();
    let r = g.add_node(N::Ref(ca(1)));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(r, o, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    let derived = derive_synthdef(&flat, 1, "test").expect("derive");

    let (mut controller, _nrt, mut world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 1,
        ..Options::default()
    });
    controller.add_synthdef(derived.def);
    let node = controller
        .synth_new("test", ROOT_GROUP_ID, AddAction::Tail)
        .expect("synth_new");
    for gain in &derived.gains {
        controller
            .set_control(node, gain.index, 1.0)
            .expect("fade in");
    }

    let a = render(&mut world, SR as usize / 2);
    assert!(a.iter().any(|s| s.abs() > 0.1), "nested synth was silent");
    let (m220, m440) = (goertzel(&a, 220.0), goertzel(&a, 440.0));
    assert!(
        m220 > 5.0 * m440,
        "expected 220 Hz dominant: m220={m220}, m440={m440}",
    );
}

/// Goertzel magnitude estimate at `freq` (Hz) over mono `samples` sampled at [`SR`].
fn goertzel(samples: &[f32], freq: f32) -> f32 {
    let n = samples.len();
    let k = (0.5 + n as f32 * freq / SR).floor();
    let w = 2.0 * std::f32::consts::PI * k / n as f32;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for &x in samples {
        let s = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    power.max(0.0).sqrt() / n as f32
}

/// Render `frames` of mono audio.
fn render(world: &mut World, frames: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames + 512);
    let mut buf = vec![0.0f32; 512];
    while out.len() < frames {
        world.fill(&mut buf, 1);
        out.extend_from_slice(&buf);
    }
    out.truncate(frames);
    out
}

/// The flat vertex at `path`.
fn flat_kind<'a>(flat: &'a Graph<Flat<&'a N>>, path: &[usize]) -> &'a Flat<&'a N> {
    &flat[at(flat, path)]
}

#[test]
fn instanced_ref_stays_an_opaque_marker() {
    // parent: sin -> dsp-ref(child, inline=false) -> out. The ref is NOT
    // spliced: it survives as a single `Flat::Instance` marker carrying the
    // child's CA, with parent edges into/out of it preserved (the child's
    // nodes do not appear in the flat graph).
    let map = HashMap::from([(ca(1), lag_child())]);
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::DspRef(ca(1), false));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, o, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    // sin + the instance marker + out. The child's lag is NOT spliced.
    assert_eq!(flat.node_count(), 3, "the instanced ref is not spliced");
    assert_eq!(edges_into(&flat, &[2]), vec![(vec![1], 0, 0)]);
    assert!(
        matches!(flat_kind(&flat, &[1]), Flat::Instance { child_ca, .. } if *child_ca == ca(1)),
        "the ref lowers as an Instance marker carrying the child CA",
    );
}

#[test]
fn inlined_dsp_ref_splices_as_a_plain_ref() {
    // The same topology with `inline: true` splices the child's lag, matching
    // the plain `Ref` behaviour: the marker is gone and the lag carries its
    // nested path.
    let map = HashMap::from([(ca(1), lag_child())]);
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::DspRef(ca(1), true));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, o, Edge::new(0.into(), 0.into()));

    let flat = flatten_with(&g, &map);
    assert_eq!(flat.node_count(), 3, "sin + spliced lag + out");
    assert!(
        matches!(flat_kind(&flat, &[1, 1]), Flat::Node { .. }),
        "the inlined ref splices the child's nodes",
    );
    assert_eq!(edges_into(&flat, &[2]), vec![(vec![1, 1], 0, 0)]);
}

#[test]
fn instance_marker_preserves_multiport_edges() {
    // An instanced ref with two inlets and two outlets: parent edges into each
    // input and out of each output are kept on the marker (input i / output j
    // positional), so `derive_template` sees a node with the ref's arity.
    let mut child = Graph::<N>::default();
    let i0 = child.add_node(N::Inlet);
    let i1 = child.add_node(N::Inlet);
    let o0 = child.add_node(N::Outlet);
    let o1 = child.add_node(N::Outlet);
    child.add_edge(i0, o0, Edge::new(0.into(), 0.into()));
    child.add_edge(i1, o1, Edge::new(0.into(), 0.into()));
    let map = HashMap::from([(ca(2), child)]);

    let mut g = Graph::<N>::default();
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::DspRef(ca(2), false));
    let pk = g.add_node(N::Out(Out::default()));
    // Two inputs into the ref (input 0 and 1), two outputs out (0 and 1 - the
    // second into the out's gain input to keep it distinct).
    g.add_edge(s0, r, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, r, Edge::new(0.into(), 1.into()));
    g.add_edge(r, pk, Edge::new(0.into(), 0.into()));
    g.add_edge(r, pk, Edge::new(1.into(), 1.into()));

    let flat = flatten_with(&g, &map);
    // The marker is at path [2]; both input edges and both output edges survive.
    let mut ins = edges_into(&flat, &[2]);
    ins.sort();
    assert_eq!(ins, vec![(vec![0], 0, 0), (vec![1], 0, 1)]);
    // Output 0 -> out input 0; output 1 -> out input 1.
    let mut outs: Vec<_> = flat
        .edges_directed(at(&flat, &[2]), Direction::Outgoing)
        .map(|e| (e.weight().output.0, e.weight().input.0))
        .collect();
    outs.sort();
    assert_eq!(outs, vec![(0, 0), (1, 1)]);
}

#[test]
fn root_boundaries_kept_as_markers() {
    // Root-level inlets/outlets are the flat graph's own interface: they stay
    // as `Flat::Inlet`/`Flat::Outlet` markers with positional indices, and
    // their edges survive (inlet feeding a consumer, source feeding an
    // outlet). Nested boundaries keep dissolving (covered elsewhere).
    let mut g = Graph::<N>::default();
    let i0 = g.add_node(N::Inlet);
    let l = g.add_node(N::Lag(Lag::default()));
    let i1 = g.add_node(N::Inlet);
    let o0 = g.add_node(N::Outlet);
    g.add_edge(i0, l, Edge::new(0.into(), 0.into()));
    g.add_edge(l, o0, Edge::new(0.into(), 0.into()));
    let _ = i1;

    let map = HashMap::new();
    let flat = flatten_with(&g, &map);
    assert_eq!(flat.node_count(), 4, "lag + three root boundary markers");
    assert!(
        matches!(flat_kind(&flat, &[0]), Flat::Inlet { index: 0, .. }),
        "first inlet is marker 0",
    );
    assert!(
        matches!(flat_kind(&flat, &[2]), Flat::Inlet { index: 1, .. }),
        "second inlet is marker 1 (ascending node index order)",
    );
    assert!(
        matches!(flat_kind(&flat, &[3]), Flat::Outlet { index: 0, .. }),
        "outlet is marker 0",
    );
    assert_eq!(edges_into(&flat, &[1]), vec![(vec![0], 0, 0)]);
    assert_eq!(edges_into(&flat, &[3]), vec![(vec![1], 0, 0)]);
}
