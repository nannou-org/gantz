//! Tests that `flatten` splices nested graphs into a flat graph derivation
//! understands: boundary bridging, original-path preservation, error cases and
//! an offline render of a nested graph through the real engine.

use std::collections::HashMap;

use gantz_ca::ContentAddr;
use gantz_core::edge::Edge;
use gantz_core::node::graph::{Graph, NodeIx};
use gantz_core::node::{ExprCtx, ExprResult, MetaCtx, parse_expr};
use gantz_plyphon::flatten::{Flat, FlattenError, flatten};
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
            N::Inlet | N::Outlet | N::Ref(_) | N::Other => None,
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
) -> impl Fn(&N) -> Option<(ContentAddr, Option<&'g Graph<N>>)> + 'g {
    move |n| match n {
        N::Ref(ca) => Some((*ca, map.get(ca))),
        _ => None,
    }
}

fn flatten_with(graph: &Graph<N>, map: &HashMap<ContentAddr, Graph<N>>) -> Graph<Flat<N>> {
    try_flatten(graph, map).expect("flatten")
}

fn try_flatten<'g>(
    graph: &'g Graph<N>,
    map: &'g HashMap<ContentAddr, Graph<N>>,
) -> Result<Graph<Flat<N>>, FlattenError> {
    let resolve = resolver(map);
    flatten(&|_| None, graph, &resolve)
}

/// The flat node with the given original path.
fn at<'a>(flat: &'a Graph<Flat<N>>, path: &[usize]) -> NodeIx {
    flat.node_indices()
        .find(|&n| flat[n].path == path)
        .unwrap_or_else(|| panic!("no flat node at path {path:?}"))
}

/// The `(source path, output, input)` of every edge into the node at `path`.
fn edges_into(flat: &Graph<Flat<N>>, path: &[usize]) -> Vec<(Vec<usize>, u16, u16)> {
    let n = at(flat, path);
    let mut edges: Vec<_> = flat
        .edges_directed(n, Direction::Incoming)
        .map(|e| {
            let w = e.weight();
            (flat[e.source()].path.clone(), w.output.0, w.input.0)
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

    let flat = flatten_with(&g, &HashMap::new());
    assert_eq!(flat.node_count(), 3);
    assert_eq!(flat.edge_count(), 2);
    for n in flat.node_indices() {
        assert_eq!(flat[n].path, vec![n.index()], "flat paths are [ix]");
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
fn oldest_edge_wins_across_a_bridge() {
    // Two sources race for the ref's one input: the oldest resolving edge wins
    // at each hop, matching derivation's input resolution.
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
        vec![(vec![0], 0, 0)],
        "the older source feeds the consumer",
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
    let def = derive_synthdef(&flat, 1, "test").expect("derive").def;

    let (mut controller, _nrt, mut world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 1,
        ..Options::default()
    });
    controller.add_synthdef(def);
    controller
        .synth_new("test", ROOT_GROUP_ID, AddAction::Tail)
        .expect("synth_new");

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
