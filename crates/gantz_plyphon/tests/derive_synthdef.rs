//! Tests that `derive_synthdef` builds the right plyphon `SynthDef` from a DSP
//! graph, and that the derived def actually produces the expected audio when run
//! through the real engine offline.

use gantz_core::edge::Edge;
use gantz_core::node::graph::Graph;
use gantz_plyphon::{
    Backend, DeriveError, DspBuilder, Embedded, Finished, Lag, NodeDsp, NodeRate, Out, Pack,
    PortShape, ScopeOut, Signal, SinOsc, Sum, ToNodeDsp, Unpack, derive_synthdef, structural_sig,
};
use plyphon::synthdef::{InputRef, SynthDef, UnitSpec};
use plyphon::{AddAction, Options, ROOT_GROUP_ID, Rate, World, engine};

const SR: f32 = 48_000.0;

/// A minimal erased node enum, standing in for the app's `Box<dyn Node>`.
/// `Other` is a non-DSP node (a stand-in for any control-rate node).
enum N {
    SinOsc(SinOsc),
    Lag(Lag),
    Out(Out),
    ScopeOut(ScopeOut),
    Pack(Pack),
    Sum(Sum),
    Unpack(Unpack),
    Other,
}

impl ToNodeDsp for N {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        match self {
            N::SinOsc(s) => Some(s),
            N::Lag(l) => Some(l),
            N::Out(o) => Some(o),
            N::ScopeOut(t) => Some(t),
            N::Pack(p) => Some(p),
            N::Sum(s) => Some(s),
            N::Unpack(u) => Some(u),
            N::Other => None,
        }
    }
}

/// Build a `~sinosc -> ~out` graph (default params).
fn sine_to_out() -> Graph<N> {
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, o, Edge::new(0.into(), 0.into()));
    g
}

#[test]
fn derives_expected_units() {
    let g = sine_to_out();
    let derived = derive_synthdef(&g, 1, "test").expect("derive");
    let def = &derived.def;

    assert_eq!(def.units.len(), 4, "SinOsc + level-mul + channel-mul + Out");

    // Three control params - the sine's freq (0) and the out's gain (1), each
    // carrying the node's *nominal* default (the live value lives in node state
    // and is applied via set_control), plus the out's driver-owned fade (2).
    assert_eq!(def.params.len(), 3);
    assert!(def.params[0].name.ends_with("/freq"));
    assert_eq!(def.params[0].default, SinOsc::DEFAULT_FREQ);
    assert_eq!(def.params[0].lag, None, "freq is unsmoothed by default");
    assert!(def.params[1].name.ends_with("/gain"));
    assert_eq!(def.params[1].default, Out::DEFAULT_GAIN);
    assert_eq!(
        def.params[1].lag,
        Some(0.01),
        "gain has a default de-click lag"
    );
    assert!(def.params[2].name.ends_with("/fade"));
    assert_eq!(
        def.params[2].default, 0.0,
        "fade defaults to silence (driver ramps in)"
    );

    // Bindings map each param back to its dsp node (sine at [0], out at [1]);
    // the fade has NO binding - the driver alone drives it.
    assert_eq!(derived.params.len(), 2);
    assert_eq!(derived.params[0].node_path, vec![0]);
    assert_eq!(derived.params[0].index, 0);
    assert_eq!(derived.params[1].node_path, vec![1]);
    assert_eq!(derived.params[1].index, 1);

    // unit 0: SinOsc.ar(freq-param, 0)
    assert_eq!(def.units[0].name, "SinOsc");
    assert!(matches!(def.units[0].inputs[0], InputRef::Param(0)));

    // unit 1: BinaryOpUGen.kr multiply - the level = gain * fade, once.
    assert_eq!(def.units[1].name, "BinaryOpUGen");
    assert_eq!(def.units[1].special_index, 2, "multiply selector");
    assert!(matches!(def.units[1].rate, Rate::Control));
    assert!(matches!(def.units[1].inputs[0], InputRef::Param(1)));
    assert!(matches!(def.units[1].inputs[1], InputRef::Param(2)));

    // unit 2: BinaryOpUGen.ar multiply (SinOsc * level)
    assert_eq!(def.units[2].name, "BinaryOpUGen");
    assert_eq!(def.units[2].special_index, 2, "multiply selector");
    assert!(matches!(def.units[2].rate, Rate::Audio));
    assert!(matches!(
        def.units[2].inputs[0],
        InputRef::Unit { unit: 0, output: 0 }
    ));
    assert!(matches!(
        def.units[2].inputs[1],
        InputRef::Unit { unit: 1, output: 0 }
    ));

    // unit 3: Out.ar(0, levelled)
    assert_eq!(def.units[3].name, "Out");
    assert_eq!(def.units[3].num_outputs, 0);
    assert!(matches!(def.units[3].inputs[0], InputRef::Constant(b) if b == 0.0));
    assert!(matches!(
        def.units[3].inputs[1],
        InputRef::Unit { unit: 2, output: 0 }
    ));
}

#[test]
fn lag_change_changes_structural_sig() {
    // The param *value* is no longer in the synthdef (it lives in node state), so a
    // value change cannot alter the def. The *lag* is structural, so it does.
    let g = sine_to_out();
    let base = derive_synthdef(&g, 1, "t").expect("derive").def;

    let mut g2 = Graph::<N>::default();
    let mut lagged_sine = SinOsc::default();
    lagged_sine.set_freq_lag(0.5);
    let s = g2.add_node(N::SinOsc(lagged_sine));
    let o = g2.add_node(N::Out(Out::default()));
    g2.add_edge(s, o, Edge::new(0.into(), 0.into()));
    let lagged = derive_synthdef(&g2, 1, "t").expect("derive").def;

    assert_ne!(
        structural_sig(&base),
        structural_sig(&lagged),
        "a freq lag change must change the structural signature",
    );
}

#[test]
fn lag_is_part_of_node_identity() {
    // Node identity is the erased (data-layer) content address.
    let content_addr = |n: &SinOsc| {
        gantz_core::data::erase_node_typed(n)
            .unwrap()
            .content_addr()
    };
    assert_eq!(
        content_addr(&SinOsc::default()),
        content_addr(&SinOsc::default()),
        "identical nodes share a content address",
    );
    let mut lagged = SinOsc::default();
    lagged.set_freq_lag(0.5);
    assert_ne!(
        content_addr(&SinOsc::default()),
        content_addr(&lagged),
        "the freq lag is part of the node's content address",
    );
}

#[test]
fn fans_output_across_channels() {
    let g = sine_to_out();
    let def = derive_synthdef(&g, 2, "test").expect("derive").def;
    // `Out` gets the bus index followed by one signal input per channel.
    assert_eq!(def.units[3].name, "Out");
    assert_eq!(def.units[3].inputs.len(), 1 + 2);
}

#[test]
fn lag_node_wired_into_chain() {
    // `~sinosc -> ~lag -> ~out`: the Lag UGen sits between the SinOsc and the gain
    // mul, smoothing the signal, with its own `dur` control param.
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let l = g.add_node(N::Lag(Lag::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, l, Edge::new(0.into(), 0.into()));
    g.add_edge(l, o, Edge::new(0.into(), 0.into()));
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;

    // Units: SinOsc(0), Lag(1), level-mul(2), channel-mul(3), Out(4).
    assert_eq!(def.units.len(), 5);
    assert_eq!(def.units[1].name, "Lag");
    // Lag input 0 = the SinOsc output; input 1 = the dur param.
    assert!(matches!(
        def.units[1].inputs[0],
        InputRef::Unit { unit: 0, output: 0 }
    ));
    assert!(matches!(def.units[1].inputs[1], InputRef::Param(_)));
    // The channel mul reads the Lag output.
    assert_eq!(def.units[3].name, "BinaryOpUGen");
    assert!(matches!(def.units[3].rate, Rate::Audio));
    assert!(matches!(
        def.units[3].inputs[0],
        InputRef::Unit { unit: 1, output: 0 }
    ));

    // A `dur` param (the lag time) exists, defaulting to 0.1 s.
    let dur = def
        .params
        .iter()
        .find(|p| p.name.ends_with("/dur"))
        .expect("dur param");
    assert_eq!(dur.default, Lag::DEFAULT_DUR);
}

#[test]
fn control_edge_on_root_does_not_panic() {
    // Connecting a non-DSP control source to `~out`'s gain (input index 1, beyond
    // its single dsp input) must not panic the synthdef derivation: the pull is
    // seeded over only the dsp inputs, so the control edge falls outside the eval
    // conns and is simply ignored. Regression for the `eval_neighbors` unwrap.
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    let ctrl = g.add_node(N::Other);
    g.add_edge(s, o, Edge::new(0.into(), 0.into())); // audio -> ~out input 0
    g.add_edge(ctrl, o, Edge::new(0.into(), 1.into())); // control -> ~out gain (input 1)

    let derived = derive_synthdef(&g, 1, "t").expect("derive must not panic");
    // The control source is filtered out; the dsp graph is still SinOsc + muls + Out.
    assert_eq!(
        derived.def.units.len(),
        4,
        "SinOsc + level/channel muls + Out"
    );
    assert_eq!(derived.def.units[0].name, "SinOsc");
    assert_eq!(derived.def.units[3].name, "Out");
}

#[test]
fn dsp_wire_into_freq_drives_fm() {
    // `~lag -> ~sinosc.freq -> ~out`: freq is a *hybrid* dsp input, so the
    // connected chain emits units and the `Lag`'s output wire drives the
    // oscillator's freq input directly. The freq fallback param must NOT be
    // baked - the wire wins, and an undriven param would land in the def (and
    // `structural_sig`) with nothing draining its state.
    let mut g = Graph::<N>::default();
    let l = g.add_node(N::Lag(Lag::default()));
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(l, s, Edge::new(0.into(), 0.into())); // ~lag -> sinosc freq (dsp)
    g.add_edge(s, o, Edge::new(0.into(), 0.into())); // sinosc -> ~out (dsp)

    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let def = &derived.def;
    // Units: Lag(0), SinOsc(1), level-mul, channel-mul, Out.
    assert_eq!(def.units.len(), 5);
    assert_eq!(def.units[0].name, "Lag");
    assert_eq!(def.units[1].name, "SinOsc");
    assert!(
        matches!(
            def.units[1].inputs[0],
            InputRef::Unit { unit: 0, output: 0 }
        ),
        "the SinOsc freq input reads the lag's output wire",
    );
    assert!(
        def.params.iter().all(|p| !p.name.ends_with("/freq")),
        "a wired freq bakes no fallback param",
    );
    assert!(
        derived
            .params
            .iter()
            .all(|b| b.node_path != vec![s.index()]),
        "no binding drives the wired sinosc",
    );
    // The lag's own dur param is unaffected.
    assert!(def.params.iter().any(|p| p.name.ends_with("/dur")));
}

#[test]
fn kr_modulator_fm_keeps_its_own_freq_param() {
    // `~sinosc(kr) -> ~sinosc.freq -> ~out` (vibrato duty): the modulator's own
    // freq input is unconnected so it keeps its fallback param, while the
    // carrier's freq is the kr wire (no param).
    let mut modulator = SinOsc::default();
    modulator.set_rate(NodeRate::Control);
    let mut g = Graph::<N>::default();
    let m = g.add_node(N::SinOsc(modulator));
    let c = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(m, c, Edge::new(0.into(), 0.into()));
    g.add_edge(c, o, Edge::new(0.into(), 0.into()));

    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let def = &derived.def;
    let oscs: Vec<_> = def.units.iter().filter(|u| u.name == "SinOsc").collect();
    assert_eq!(oscs.len(), 2);
    assert!(matches!(oscs[0].rate, Rate::Control));
    assert!(
        matches!(oscs[0].inputs[0], InputRef::Param(_)),
        "the modulator's freq falls back to its param",
    );
    assert!(matches!(oscs[1].rate, Rate::Audio));
    assert!(
        matches!(oscs[1].inputs[0], InputRef::Unit { .. }),
        "the carrier's freq reads the kr wire",
    );
    assert_eq!(
        def.params
            .iter()
            .filter(|p| p.name.ends_with("/freq"))
            .count(),
        1,
        "one freq param: the modulator's",
    );
    assert_eq!(
        derived
            .params
            .iter()
            .filter(|b| b.node_path == vec![m.index()])
            .count(),
        1,
        "the freq binding maps to the modulator",
    );
}

#[test]
fn multichannel_freq_expands_an_osc_per_channel() {
    // `~pack`(2) (unconnected: a 2-wide silent group) -> `~sinosc.freq` -> `~out`
    // on a 2-channel device: the oscillator expands to one SinOsc per freq
    // channel and its output group is 2 wide.
    let mut g = Graph::<N>::default();
    let pk = g.add_node(N::Pack(Pack::default()));
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(pk, s, Edge::new(0.into(), 0.into()));
    g.add_edge(s, o, Edge::new(0.into(), 0.into()));

    let def = derive_synthdef(&g, 2, "t").expect("derive").def;
    let oscs: Vec<_> = def.units.iter().filter(|u| u.name == "SinOsc").collect();
    assert_eq!(oscs.len(), 2, "one SinOsc per freq channel");
    assert!(
        oscs.iter()
            .all(|u| matches!(u.inputs[0], InputRef::Constant(c) if c == 0.0)),
        "each osc reads its own (silent) freq channel",
    );
    assert!(def.params.iter().all(|p| !p.name.ends_with("/freq")));
    let out = def.units.iter().find(|u| u.name == "Out").expect("Out");
    assert_eq!(
        out.inputs.len(),
        1 + 2,
        "the 2-wide group reaches both device channels",
    );
}

#[test]
fn freq_wire_changes_structural_sig() {
    // Connecting a dsp wire into freq swaps the freq param for the wire (and
    // pulls the modulator chain into the def), so the structural sig changes
    // and the driver respawns (crossfaded).
    let def = |fm: bool| {
        let mut g = Graph::<N>::default();
        let m = g.add_node(N::SinOsc(SinOsc::default()));
        let s = g.add_node(N::SinOsc(SinOsc::default()));
        let o = g.add_node(N::Out(Out::default()));
        if fm {
            g.add_edge(m, s, Edge::new(0.into(), 0.into()));
        }
        g.add_edge(s, o, Edge::new(0.into(), 0.into()));
        derive_synthdef(&g, 1, "t").expect("derive").def
    };
    assert_ne!(
        structural_sig(&def(false)),
        structural_sig(&def(true)),
        "a freq connect/disconnect must change the structural signature",
    );
}

#[test]
fn dangling_unpack_port_into_freq_keeps_the_param() {
    // A stale `~unpack` edge (output 1 of a count-1 unpack) into `~sinosc.freq`:
    // the summand materializes no signal, so freq must fall back to its param.
    // (The Steel side cannot see the port is dangling and keeps queueing state
    // updates; only a def-present param is drained by the driver.)
    let mut unpack = Unpack::default();
    unpack.set_count(1);
    let mut g = Graph::<N>::default();
    let src = g.add_node(N::SinOsc(SinOsc::default()));
    let up = g.add_node(N::Unpack(unpack));
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(src, up, Edge::new(0.into(), 0.into()));
    g.add_edge(up, s, Edge::new(1.into(), 0.into())); // stale: output 1 of 1
    g.add_edge(s, o, Edge::new(0.into(), 0.into()));

    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let def = &derived.def;
    // Both the upstream source's and the carrier's freq params are baked.
    assert_eq!(
        def.params
            .iter()
            .filter(|p| p.name.ends_with("/freq"))
            .count(),
        2,
        "the dangling-port freq falls back to its param",
    );
    // The carrier (emitted after the source) reads a param, not a wire.
    let carrier = def
        .units
        .iter()
        .filter(|u| u.name == "SinOsc")
        .last()
        .expect("carrier SinOsc");
    assert!(matches!(carrier.inputs[0], InputRef::Param(_)));
}

#[test]
fn scopeout_output_into_freq_keeps_the_param() {
    // `~scopeout` output 1 (its Steel channel-count output - the node has NO dsp
    // outputs) wired into `~sinosc.freq`: the summand materializes no signal, so
    // freq must fall back to its param. Guards the "None iff nothing
    // materialized" rule: keying on raw summands would bake silence here while
    // the node's Steel expr queues the (numeric) channel count into a param
    // absent from the def, growing `pending` unboundedly.
    let mut g = Graph::<N>::default();
    let t = g.add_node(N::ScopeOut(ScopeOut::default()));
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(t, s, Edge::new(1.into(), 0.into())); // scope channel count -> freq
    g.add_edge(s, o, Edge::new(0.into(), 0.into()));

    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let def = &derived.def;
    assert!(
        def.params.iter().any(|p| p.name.ends_with("/freq")),
        "freq falls back to its param",
    );
    let osc = def
        .units
        .iter()
        .find(|u| u.name == "SinOsc")
        .expect("SinOsc");
    assert!(matches!(osc.inputs[0], InputRef::Param(_)));
}

#[test]
fn graph_without_sink_is_rejected() {
    // A graph with no dsp sink (no `~out`, no `~scopeout`) has nothing to root a
    // synthdef at.
    let mut g = Graph::<N>::default();
    g.add_node(N::Other);
    assert!(matches!(
        derive_synthdef(&g, 1, "nope"),
        Err(DeriveError::NoSink)
    ));
}

#[test]
fn port_shapes_record_width_and_rate() {
    // Every dsp output port derivation materializes a signal for gets a
    // `(path, port) -> (width, rate)` entry. `~out` has no dsp outputs, so a
    // bare `~sinosc -> ~out` records exactly the oscillator's port.
    let derived = derive_synthdef(&sine_to_out(), 1, "t").expect("derive");
    let shape = |w, r| PortShape { width: w, rate: r };
    assert_eq!(
        derived.shapes.iter().collect::<Vec<_>>(),
        vec![(&(vec![0], 0), &shape(1, Rate::Audio))],
    );

    // Two oscs packed into one group: the pack's port is 2 wide.
    let mut g = Graph::<N>::default();
    let a = g.add_node(N::SinOsc(SinOsc::default()));
    let b = g.add_node(N::SinOsc(SinOsc::default()));
    let pk = g.add_node(N::Pack(Pack::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(a, pk, Edge::new(0.into(), 0.into()));
    g.add_edge(b, pk, Edge::new(0.into(), 1.into()));
    g.add_edge(pk, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    assert_eq!(derived.shapes[&(vec![2], 0)], shape(2, Rate::Audio));

    // A control-rate osc's port stays kr even though `~out` lifts it via K2A.
    let mut g = Graph::<N>::default();
    let mut sine = SinOsc::default();
    sine.set_rate(NodeRate::Control);
    let s = g.add_node(N::SinOsc(sine));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    assert_eq!(derived.shapes[&(vec![0], 0)], shape(1, Rate::Control));
}

#[test]
fn derive_error_displays_readably() {
    // Derive failures surface in the UI, so each variant formats as a
    // readable message rather than a `Debug` dump.
    let ca = gantz_ca::ContentAddr([9; 32]);
    assert_eq!(
        DeriveError::NoSink.to_string(),
        "no dsp sink (no `~out` output and no `~scopeout` monitor)",
    );
    assert_eq!(
        DeriveError::BusCycle.to_string(),
        "`~bus`/instance boundaries form a cycle between parts",
    );
    assert_eq!(
        DeriveError::Unresolved(ca).to_string(),
        format!("unresolved instanced reference: {ca}"),
    );
    assert_eq!(
        DeriveError::RefCycle(ca).to_string(),
        format!("instanced references form a cycle through {ca}"),
    );
}

#[test]
fn scopeout_joins_output_in_one_def() {
    // `~sinosc -> ~out` and `~sinosc -> ~scopeout`: the tap is a second sink that shares the
    // sine's chain, so one synthdef carries SinOsc, Out and a ScopeOut, with a single
    // monitor binding at the tap's node path - and the shared SinOsc is emitted once.
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    let t = g.add_node(N::ScopeOut(ScopeOut::default()));
    g.add_edge(s, o, Edge::new(0.into(), 0.into())); // sine -> ~out (audio)
    g.add_edge(s, t, Edge::new(0.into(), 0.into())); // sine -> ~scopeout (dsp input 0)

    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let names: Vec<&str> = derived.def.units.iter().map(|u| u.name.as_str()).collect();
    assert!(names.contains(&"SinOsc"), "units: {names:?}");
    assert!(names.contains(&"Out"), "units: {names:?}");
    assert!(names.contains(&"ScopeOut"), "units: {names:?}");
    assert_eq!(
        names.iter().filter(|n| **n == "SinOsc").count(),
        1,
        "the signal feeding both sinks is emitted once",
    );

    assert_eq!(
        derived.monitors.len(),
        1,
        "one ~scopeout -> one monitor binding"
    );
    let mon = &derived.monitors[0];
    assert_eq!(mon.node_path, vec![t.index()]);
    assert_eq!(mon.size, ScopeOut::DEFAULT_SIZE);

    // The binding's `scope_unit` names the ScopeOut; its bufnum (input 0) is the
    // no-lag control param the driver sets via `set_control`, and its value
    // input (1) is the tapped sine.
    let scope = &derived.def.units[mon.scope_unit];
    assert_eq!(scope.name, "ScopeOut", "scope_unit must name the ScopeOut");
    assert!(
        matches!(scope.inputs[0], InputRef::Param(_)),
        "bufnum is a no-lag control param the driver sets post-spawn",
    );
    assert!(
        matches!(scope.inputs[1], InputRef::Unit { .. }),
        "ScopeOut value input is the tapped signal",
    );
}

/// A hand-built 2-channel signal from two mono wires.
fn stereo(ch0: InputRef, ch1: InputRef) -> Signal {
    Signal::concat([Signal::mono(ch0), Signal::mono(ch1)])
}

#[test]
fn scopeout_taps_a_multichannel_signal() {
    // A `~scopeout` fed a 2-channel signal: its one dsp input carries the whole
    // group; the ScopeOut takes `bufnum` + one signal input per channel, and the
    // binding records the *inferred* width (which the driver passes to `cue_scope`).
    let mut b = DspBuilder::new(1);
    let sig = stereo(
        InputRef::Unit { unit: 7, output: 0 },
        InputRef::Unit { unit: 8, output: 0 },
    );
    let outs = ScopeOut::default().ugens(&[2], &[Some(sig)], &mut b);
    assert!(outs.is_empty(), "a tap sink has no dsp outputs");

    let Finished { def, monitors, .. } = b.finish("t");
    let scope = def
        .units
        .iter()
        .find(|u| u.name == "ScopeOut")
        .expect("ScopeOut unit");
    assert_eq!(scope.inputs.len(), 3, "bufnum + one signal per channel (2)");
    assert!(matches!(scope.inputs[1], InputRef::Unit { unit: 7, .. }));
    assert!(matches!(scope.inputs[2], InputRef::Unit { unit: 8, .. }));
    assert_eq!(monitors.len(), 1);
    assert_eq!(
        monitors[0].channels, 2,
        "binding records the inferred width"
    );
    assert_eq!(monitors[0].node_path, vec![2]);
}

#[test]
fn lag_smooths_each_channel() {
    // `~lag` on a 2-channel signal: one `Lag` unit per channel, all sharing the
    // single `dur` param (params broadcast across the group); width in = width out.
    let mut b = DspBuilder::new(1);
    let sig = stereo(InputRef::Constant(0.25), InputRef::Constant(0.5));
    let outs = Lag::default().ugens(&[0], &[Some(sig)], &mut b);
    assert_eq!(outs.len(), 1, "one dsp output port");
    assert_eq!(outs[0].width(), 2, "width flows through");

    let Finished { def, params, .. } = b.finish("t");
    let lags: Vec<_> = def.units.iter().filter(|u| u.name == "Lag").collect();
    assert_eq!(lags.len(), 2, "one Lag per channel");
    assert_eq!(def.params.len(), 1, "one shared dur param");
    assert!(def.params[0].name.ends_with("/dur"));
    assert!(
        lags.iter()
            .all(|u| matches!(u.inputs[1], InputRef::Param(0))),
        "every channel's Lag reads the shared dur param",
    );
    assert_eq!(params.len(), 1);
}

#[test]
fn out_writes_multichannel_channel_per_bus() {
    // A 2-channel signal into `~out` on a 2-channel device: channel i -> bus i,
    // each through its own gain multiply sharing the single gain param (no mono
    // fan-out - the two written wires stay distinct).
    let mut b = DspBuilder::new(2);
    let sig = stereo(InputRef::Constant(0.25), InputRef::Constant(0.5));
    let outs = Out::default().ugens(&[0], &[Some(sig)], &mut b);
    assert!(outs.is_empty());

    let Finished { def, .. } = b.finish("t");
    // One control-rate level mul (gain * fade) shared by two per-channel muls.
    let kr_muls: Vec<_> = def
        .units
        .iter()
        .filter(|u| u.name == "BinaryOpUGen" && matches!(u.rate, Rate::Control))
        .collect();
    assert_eq!(kr_muls.len(), 1, "one shared level (gain * fade) multiply");
    let muls: Vec<_> = def
        .units
        .iter()
        .filter(|u| u.name == "BinaryOpUGen" && matches!(u.rate, Rate::Audio))
        .collect();
    assert_eq!(muls.len(), 2, "one level multiply per written channel");
    assert!(matches!(muls[0].inputs[0], InputRef::Constant(c) if c == 0.25));
    assert!(matches!(muls[1].inputs[0], InputRef::Constant(c) if c == 0.5));
    assert_eq!(def.params.len(), 2, "one shared gain param + its fade");
    let out = def
        .units
        .iter()
        .find(|u| u.name == "Out")
        .expect("Out unit");
    assert_eq!(out.inputs.len(), 1 + 2);
    // The channel multiplies are units 1 and 2 in this builder (the level mul is
    // 0): bus channel 0 reads the first, bus channel 1 the second.
    assert!(matches!(out.inputs[1], InputRef::Unit { unit: 1, .. }));
    assert!(matches!(out.inputs[2], InputRef::Unit { unit: 2, .. }));
}

#[test]
fn out_drops_excess_channels() {
    // A 3-channel signal on a 2-channel device: only 2 channels are written, and
    // only 2 gain multiplies are emitted (dead units would pollute the
    // structural sig and burn audio CPU).
    let mut b = DspBuilder::new(2);
    let sig = Signal::concat([
        Signal::mono(InputRef::Constant(0.1)),
        Signal::mono(InputRef::Constant(0.2)),
        Signal::mono(InputRef::Constant(0.3)),
    ]);
    Out::default().ugens(&[0], &[Some(sig)], &mut b);

    let Finished { def, .. } = b.finish("t");
    let n_muls = def
        .units
        .iter()
        .filter(|u| u.name == "BinaryOpUGen" && matches!(u.rate, Rate::Audio))
        .count();
    assert_eq!(n_muls, 2, "no channel multiply for the dropped channel");
    let out = def
        .units
        .iter()
        .find(|u| u.name == "Out")
        .expect("Out unit");
    assert_eq!(out.inputs.len(), 1 + 2);
}

#[test]
fn pack_widens_a_scopeout_tap() {
    // Two sines -> `~pack`(2) -> `~scopeout`: the pack concatenates the two mono
    // groups into one 2-wide edge, so the tap infers 2 channels - and neither
    // routing node emits any units.
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let pk = g.add_node(N::Pack(Pack::default()));
    let t = g.add_node(N::ScopeOut(ScopeOut::default()));
    g.add_edge(s0, pk, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, pk, Edge::new(0.into(), 1.into()));
    g.add_edge(pk, t, Edge::new(0.into(), 0.into()));

    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    assert_eq!(
        derived.def.units.len(),
        3,
        "2 SinOsc + ScopeOut; ~pack emits nothing",
    );
    let scope = derived
        .def
        .units
        .iter()
        .find(|u| u.name == "ScopeOut")
        .expect("ScopeOut unit");
    assert_eq!(scope.inputs.len(), 3, "bufnum + one signal per channel (2)");
    assert_eq!(derived.monitors[0].channels, 2, "width inferred as 2");
}

#[test]
fn pack_to_out_writes_two_device_channels() {
    // Two sines -> `~pack`(2) -> `~out` on a 2-channel device: channel i -> bus i,
    // each through its own gain multiply sharing the one gain param (no mono fan).
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let pk = g.add_node(N::Pack(Pack::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s0, pk, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, pk, Edge::new(0.into(), 1.into()));
    g.add_edge(pk, o, Edge::new(0.into(), 0.into()));

    let def = derive_synthdef(&g, 2, "t").expect("derive").def;
    assert_eq!(
        def.units.len(),
        6,
        "2 SinOsc + level mul + 2 channel muls + Out",
    );
    let muls: Vec<_> = def
        .units
        .iter()
        .filter(|u| u.name == "BinaryOpUGen" && matches!(u.rate, Rate::Audio))
        .collect();
    assert_eq!(muls.len(), 2, "one level multiply per written channel");
    assert_eq!(
        def.params
            .iter()
            .filter(|p| p.name.ends_with("/gain"))
            .count(),
        1,
        "the channels share one gain param",
    );
    let out = def.units.iter().find(|u| u.name == "Out").expect("Out");
    assert_eq!(out.inputs.len(), 1 + 2);
    // The two written channels reach distinct sine chains (not a fanned mono).
    let bus_units: Vec<u32> = out.inputs[1..]
        .iter()
        .map(|i| match i {
            InputRef::Unit { unit, .. } => *unit,
            other => panic!("expected a unit ref, got {other:?}"),
        })
        .collect();
    assert_ne!(bus_units[0], bus_units[1], "channels must stay distinct");
}

#[test]
fn pack_unpack_routes_a_channel() {
    // sine0 + sine1 -> `~pack`(2) -> `~unpack`(2), output 1 -> `~out`: pure
    // re-routing that must deliver *sine1*'s wire to the out (identified via its
    // freq param's node-path binding). The unreached sine0 chain still derives
    // (it was pulled), but the out's gain mul must read sine1.
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let pk = g.add_node(N::Pack(Pack::default()));
    let up = g.add_node(N::Unpack(Unpack::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s0, pk, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, pk, Edge::new(0.into(), 1.into()));
    g.add_edge(pk, up, Edge::new(0.into(), 0.into()));
    g.add_edge(up, o, Edge::new(1.into(), 0.into())); // unpack output 1 -> ~out

    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let def = &derived.def;
    assert_eq!(
        def.units.len(),
        5,
        "2 SinOsc + level/channel muls + Out; no routing units"
    );

    // The channel mul's signal input is a SinOsc unit output...
    let mul = def
        .units
        .iter()
        .find(|u| u.name == "BinaryOpUGen" && matches!(u.rate, Rate::Audio))
        .expect("channel mul");
    let sine_unit = match mul.inputs[0] {
        InputRef::Unit { unit, .. } => unit as usize,
        other => panic!("expected a unit ref, got {other:?}"),
    };
    assert_eq!(def.units[sine_unit].name, "SinOsc");
    // ...and that SinOsc's freq param binds to *sine1*'s node path: channel 1 of
    // the packed group is sine1.
    let freq_param = match def.units[sine_unit].inputs[0] {
        InputRef::Param(p) => p as usize,
        other => panic!("expected a param ref, got {other:?}"),
    };
    let binding = derived
        .params
        .iter()
        .find(|b| b.index == freq_param)
        .expect("freq binding");
    assert_eq!(binding.node_path, vec![s1.index()], "channel 1 is sine1");
}

#[test]
fn pack_count_changes_structural_sig() {
    // Widening a pack (2 -> 3 inputs) widens the tapped group, changing the
    // ScopeOut's input count and thus the structural sig (the driver respawns).
    let scope_def = |count: usize| {
        let mut pack = Pack::default();
        pack.set_count(count);
        let mut g = Graph::<N>::default();
        let s = g.add_node(N::SinOsc(SinOsc::default()));
        let pk = g.add_node(N::Pack(pack));
        let t = g.add_node(N::ScopeOut(ScopeOut::default()));
        g.add_edge(s, pk, Edge::new(0.into(), 0.into()));
        g.add_edge(pk, t, Edge::new(0.into(), 0.into()));
        derive_synthdef(&g, 1, "t").expect("derive").def
    };
    assert_ne!(
        structural_sig(&scope_def(2)),
        structural_sig(&scope_def(3)),
        "a width change must change the structural signature",
    );
}

#[test]
fn unpack_stale_output_edge_derives_silently() {
    // An edge left hanging off a removed `~unpack` output (count shrunk to 1,
    // edge still on output 1): the Steel compile surfaces a diagnostic, but
    // synthdef derivation must not panic - the missing port resolves to mono
    // silence.
    let mut unpack = Unpack::default();
    unpack.set_count(1);
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let up = g.add_node(N::Unpack(unpack));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, up, Edge::new(0.into(), 0.into()));
    g.add_edge(up, o, Edge::new(1.into(), 0.into())); // stale: output 1 of 1

    let derived = derive_synthdef(&g, 1, "t").expect("derive must not panic");
    let mul = derived
        .def
        .units
        .iter()
        .find(|u| u.name == "BinaryOpUGen" && matches!(u.rate, Rate::Audio))
        .expect("channel mul");
    assert!(
        matches!(mul.inputs[0], InputRef::Constant(c) if c == 0.0),
        "the missing port must resolve to silence",
    );
}

#[test]
fn scopeout_without_output_still_derives() {
    // A monitor-only graph (`~sinosc -> ~scopeout`, no `~out`) derives a silent synthdef:
    // a `~scopeout` is a sink in its own right, so there is something to root at.
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let t = g.add_node(N::ScopeOut(ScopeOut::default()));
    g.add_edge(s, t, Edge::new(0.into(), 0.into()));

    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let names: Vec<&str> = derived.def.units.iter().map(|u| u.name.as_str()).collect();
    assert!(
        names.contains(&"SinOsc") && names.contains(&"ScopeOut"),
        "{names:?}"
    );
    assert!(
        !names.contains(&"Out"),
        "no ~out means no Out unit: {names:?}"
    );
    assert_eq!(derived.monitors.len(), 1);
}

#[test]
fn scopeout_streams_every_sample() {
    // `~sinosc -> ~scopeout` (no ~out): the tap's ScopeOut streams *every* sample of the
    // sine off the audio thread into a cued scope stream. Draining it recovers the
    // full-rate 220 Hz signal - the stream the driver appends into the tap's ring.
    const BLOCK: usize = 64;
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let t = g.add_node(N::ScopeOut(ScopeOut::default()));
    g.add_edge(s, t, Edge::new(0.into(), 0.into()));

    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    // The driver sets the ScopeOut's bufnum param to the cued scope index via
    // `set_control` after spawning; here that index is 0 (the param default).
    let bufnum_param = derived.monitors[0].bufnum_param;
    let scope_unit = derived.monitors[0].scope_unit;
    assert_eq!(derived.def.units[scope_unit].name, "ScopeOut");

    // A pool generous enough to hold the whole run, so nothing overruns before the
    // single drain at the end (one chunk per block).
    let blocks = 128;
    let (mut controller, _nrt, mut world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 1,
        block_size: BLOCK,
        ..Options::default()
    });
    let mut consumer = controller
        .cue_scope(0, 1, SR as f64, BLOCK, blocks + 2)
        .expect("cue_scope");
    controller.add_synthdef(derived.def);
    let node = controller
        .synth_new("t", ROOT_GROUP_ID, AddAction::Tail)
        .expect("synth_new");
    controller
        .set_control(node, bufnum_param, 0.0)
        .expect("set bufnum");

    // Render a stretch of audio, then drain every streamed sample.
    let mut buf = vec![0.0f32; BLOCK];
    for _ in 0..blocks {
        world.fill(&mut buf, 1);
    }
    let mut got = Vec::new();
    while let Some(chunk) = consumer.pop_filled() {
        got.extend_from_slice(chunk.filled_samples());
        consumer.recycle(chunk);
    }

    assert_eq!(
        got.len(),
        blocks * BLOCK,
        "the scope must stream every input sample",
    );
    assert!(
        got.iter().any(|&s| s.abs() > 0.1),
        "scope stream was silent"
    );
    assert!(
        got.iter().all(|&s| s.abs() <= 1.001),
        "scope exceeded full scale",
    );
    // It carries the real 220 Hz signal, not aliased garbage.
    let (m220, m440) = (goertzel(&got, 220.0), goertzel(&got, 440.0));
    assert!(
        m220 > 5.0 * m440,
        "scope must carry the 220 Hz signal: m220={m220}, m440={m440}",
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

/// Render `frames` of mono audio, cycling buffer sizes to exercise the reblocker.
fn render(world: &mut World, frames: usize) -> Vec<f32> {
    let sizes = [64usize, 100, 128, 480, 512, 333];
    let mut out = Vec::with_capacity(frames + 512);
    let mut buf = Vec::new();
    let mut i = 0;
    while out.len() < frames {
        let size = sizes[i % sizes.len()];
        i += 1;
        buf.clear();
        buf.resize(size, 0.0);
        world.fill(&mut buf, 1);
        out.extend_from_slice(&buf);
    }
    out.truncate(frames);
    out
}

#[test]
fn derived_synth_plays_expected_tone() {
    let g = sine_to_out();
    let derived = derive_synthdef(&g, 1, "test").expect("derive");

    let (mut controller, _nrt, mut world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 1,
        ..Options::default()
    });
    controller.add_synthdef(derived.def);
    let node = controller
        .synth_new("test", ROOT_GROUP_ID, AddAction::Tail)
        .expect("synth_new");
    {
        let mut backend = Embedded::new(&mut controller);
        for gain in &derived.gains {
            backend.set_control(node, gain.index, 1.0).expect("fade in");
        }
    }

    // The freq/gain params default to the nodes' nominal defaults (220 Hz, 0.2), so
    // the synth plays a 220 Hz tone with no set_control.
    let a = render(&mut world, SR as usize / 2);
    assert!(
        a.iter().any(|s| s.abs() > 0.1),
        "derived synth produced silence"
    );
    assert!(
        a.iter().all(|s| s.abs() <= 1.001),
        "output exceeded full scale"
    );
    let m220 = goertzel(&a, 220.0);
    let m440 = goertzel(&a, 440.0);
    assert!(
        m220 > 5.0 * m440,
        "expected 220 Hz dominant: m220={m220}, m440={m440}"
    );
}

#[test]
fn audio_rate_fm_renders_through_the_wire() {
    // `~sinosc(ar) -> ~sinosc.freq -> ~out` rendered offline: the carrier's freq
    // is the modulator's raw [-1, 1] Hz signal, so the output is the faint
    // phase-wobble tone at the modulator's 220 Hz (the carrier's own 220 Hz
    // *param* default must NOT sound - the wire wins). Proves the audio-rate
    // freq path end to end through the real engine.
    let mut g = Graph::<N>::default();
    let m = g.add_node(N::SinOsc(SinOsc::default()));
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(m, s, Edge::new(0.into(), 0.into()));
    g.add_edge(s, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");

    let (mut controller, _nrt, mut world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 1,
        ..Options::default()
    });
    controller.add_synthdef(derived.def);
    let node = controller
        .synth_new("t", ROOT_GROUP_ID, AddAction::Tail)
        .expect("synth_new");
    {
        let mut backend = Embedded::new(&mut controller);
        for gain in &derived.gains {
            backend.set_control(node, gain.index, 1.0).expect("fade in");
        }
    }

    let a = render(&mut world, SR as usize / 2);
    assert!(
        a.iter().all(|s| s.is_finite() && s.abs() <= 1.001),
        "output must stay finite and within full scale",
    );
    // A ±1 Hz freq wobbles the phase by ~1/(2*pi*220), so the tone at 220 Hz has
    // a tiny but clearly-detectable magnitude - and 330 Hz carries nothing.
    let (m220, m330) = (goertzel(&a, 220.0), goertzel(&a, 330.0));
    assert!(m220 > 1e-6, "expected the 220 Hz wobble: m220={m220}");
    assert!(
        m220 > 5.0 * m330,
        "220 Hz must dominate: m220={m220}, m330={m330}",
    );
}

#[test]
fn stereo_pack_plays_per_channel_tones() {
    // Two sines -> `~pack`(2) -> `~out` rendered offline on a 2-channel device:
    // each device channel carries its own sine (220 Hz left, 330 Hz right - the
    // second sine re-tuned via set_control), proving the channel-per-bus write
    // end to end through the real engine.
    const BLOCK: usize = 64;
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let pk = g.add_node(N::Pack(Pack::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s0, pk, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, pk, Edge::new(0.into(), 1.into()));
    g.add_edge(pk, o, Edge::new(0.into(), 0.into()));

    let derived = derive_synthdef(&g, 2, "t").expect("derive");
    let s1_freq = derived
        .params
        .iter()
        .find(|b| b.node_path == [s1.index()])
        .expect("sine1 freq binding")
        .index;

    let (mut controller, _nrt, mut world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 2,
        block_size: BLOCK,
        ..Options::default()
    });
    controller.add_synthdef(derived.def);
    let node = controller
        .synth_new("t", ROOT_GROUP_ID, AddAction::Tail)
        .expect("synth_new");
    {
        let mut backend = Embedded::new(&mut controller);
        for gain in &derived.gains {
            backend.set_control(node, gain.index, 1.0).expect("fade in");
        }
    }
    Embedded::new(&mut controller)
        .set_control(node, s1_freq, 330.0)
        .expect("re-tune sine1");

    // Render half a second of interleaved stereo, then split the channels.
    let mut out = vec![0.0f32; (SR as usize / 2 / BLOCK) * BLOCK * 2];
    for block in out.chunks_mut(BLOCK * 2) {
        world.fill(block, 2);
    }
    let left: Vec<f32> = out.iter().copied().step_by(2).collect();
    let right: Vec<f32> = out.iter().skip(1).copied().step_by(2).collect();

    let (l220, l330) = (goertzel(&left, 220.0), goertzel(&left, 330.0));
    assert!(
        l220 > 5.0 * l330,
        "left must carry the 220 Hz sine: l220={l220}, l330={l330}",
    );
    let (r220, r330) = (goertzel(&right, 220.0), goertzel(&right, 330.0));
    assert!(
        r330 > 5.0 * r220,
        "right must carry the 330 Hz sine: r220={r220}, r330={r330}",
    );
}

/// A control change scheduled via [`Embedded::set_control_at`] takes effect at its
/// scheduled time (not immediately): a freq change to 440 Hz scheduled for 0.25 s
/// leaves the first quarter-second at 220 Hz and the rest at 440 Hz. This guards the
/// `begin_scheduled`/`set_control`/`end_scheduled` wrapper and the `fill_at` clock.
#[test]
fn scheduled_control_change_takes_effect_at_its_time() {
    /// OSC/NTP fixed-point units per second (32.32 fixed point: 2^32).
    const OSC_UNITS_PER_SEC: f64 = 4_294_967_296.0;
    const BLOCK: usize = 64;

    let g = sine_to_out();
    let derived = derive_synthdef(&g, 1, "test").expect("derive");
    // The sine's freq param (the node at path `[0]`) and its index within the synth.
    let freq_index = derived
        .params
        .iter()
        .find(|b| b.node_path == [0])
        .expect("freq binding")
        .index;

    let (mut controller, _nrt, mut world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 1,
        ..Options::default()
    });
    controller.add_synthdef(derived.def);
    let node = controller
        .synth_new("test", ROOT_GROUP_ID, AddAction::Tail)
        .expect("synth_new");
    {
        let mut backend = Embedded::new(&mut controller);
        for gain in &derived.gains {
            backend.set_control(node, gain.index, 1.0).expect("fade in");
        }
    }

    // Schedule freq -> 440 Hz at 0.25 s on the engine's OSC clock.
    let osc = |secs: f64| (secs * OSC_UNITS_PER_SEC) as u64;
    let switch_secs = 0.25;
    Embedded::new(&mut controller)
        .set_control_at(node, freq_index, 440.0, osc(switch_secs))
        .expect("schedule freq change");

    // Render 0.5 s in nominal blocks, anchoring the engine clock to each block's
    // nominal OSC time (the clock starts at 0 and advances by `inc` per block).
    let inc = (BLOCK as f64 * OSC_UNITS_PER_SEC / SR as f64) as u64;
    let total = (SR as usize / 2 / BLOCK) * BLOCK;
    let mut out = vec![0.0f32; total];
    for (n, block) in out.chunks_mut(BLOCK).enumerate() {
        world.fill_at(block, 1, n as u64 * inc);
    }

    // Before the switch: still 220 Hz (so it was scheduled, not applied at once).
    let switch = (switch_secs * SR as f64) as usize;
    let before = &out[..switch];
    let (b220, b440) = (goertzel(before, 220.0), goertzel(before, 440.0));
    assert!(
        b220 > 4.0 * b440,
        "expected 220 Hz before the scheduled switch: m220={b220}, m440={b440}",
    );
    // After the switch (skipping the boundary block): now 440 Hz.
    let after = &out[switch + BLOCK..];
    let (a220, a440) = (goertzel(after, 220.0), goertzel(after, 440.0));
    assert!(
        a440 > 4.0 * a220,
        "expected 440 Hz after the scheduled switch: m220={a220}, m440={a440}",
    );
}

#[test]
fn out_registers_a_fade_gain() {
    // `~out` carries a driver-owned fade gain - the crossfade lever - recorded
    // in `Derived.gains` with the fade ramp time, and deliberately WITHOUT a
    // param binding (no node state feeds it; the driver alone drives it). The
    // user's gain param keeps its ordinary binding for live value sync.
    let g = sine_to_out();
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    assert_eq!(derived.gains.len(), 1);
    let fade = derived.gains[0];
    assert!(derived.def.params[fade.index].name.ends_with("/fade"));
    assert_eq!(fade.lag, gantz_plyphon::FADE_LAG);
    assert!(
        !derived.params.iter().any(|b| b.index == fade.index),
        "the fade must have no node binding",
    );
    let gain = derived
        .def
        .params
        .iter()
        .position(|p| p.name.ends_with("/gain"))
        .expect("gain param");
    assert!(
        derived.params.iter().any(|b| b.index == gain),
        "the user gain keeps its node binding for live value sync",
    );
}

#[test]
fn zeroed_gain_default_keeps_sig() {
    // The fade default is baked at 0.0 (the spawn-silent half of the crossfade);
    // the sig must exclude defaults, or every re-derive would spuriously
    // respawn. Changing the default to any value must leave the sig untouched.
    let g = sine_to_out();
    let mut derived = derive_synthdef(&g, 1, "t").expect("derive");
    for g in &derived.gains {
        assert_eq!(derived.def.params[g.index].default, 0.0, "fade bakes 0.0");
    }
    let sig = structural_sig(&derived.def);
    for g in &derived.gains {
        derived.def.params[g.index].default = 1.0;
    }
    assert_eq!(sig, structural_sig(&derived.def));
}

#[test]
fn patched_fade_default_fades_in() {
    // The crossfade's fade-in half: the fade default is baked at 0.0 so the
    // synth spawns SILENT (defaults seed the lag state too - no ramp-from-zero
    // surprise in reverse), and restoring the fade to unity ramps the output in
    // over `FADE_LAG` rather than stepping.
    let g = sine_to_out();
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let fade = derived.gains[0].index;

    let (mut controller, _nrt, mut world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 1,
        ..Options::default()
    });
    controller.add_synthdef(derived.def);
    let node = controller
        .synth_new("t", ROOT_GROUP_ID, AddAction::Tail)
        .expect("synth_new");

    // Silent while the fade sits at its patched 0.0 default.
    let quiet = render(&mut world, SR as usize / 10);
    assert!(quiet.iter().all(|s| s.abs() < 1e-4), "must spawn silent");

    // Restoring the fade ramps the tone in: the first block is much quieter
    // than the settled tone (the LagControl steps toward unity per control
    // tick; at FADE_LAG the first step is a small fraction of the target).
    Embedded::new(&mut controller)
        .set_control(node, fade, 1.0)
        .expect("set fade");
    let ramp = render(&mut world, SR as usize / 2);
    let start = rms(&ramp[..64]);
    let settled = rms(&ramp[ramp.len() - SR as usize / 10..]);
    assert!(settled > 0.05, "tone must settle in: settled={settled}");
    assert!(
        start < 0.4 * settled,
        "fade must ramp, not step: start={start}, settled={settled}",
    );
}

#[test]
fn redefining_a_def_name_keeps_old_synth_playing() {
    // The driver reuses ONE def name per head across replacements: plyphon
    // retires the previous compiled def when a name is re-added and a running
    // synth keeps its own reference - so the old synth must keep sounding while
    // a replacement installed under the same name fades in, and the overlap
    // must stay smooth (this is the crossfade the driver builds on).
    let g = sine_to_out();
    let derived_old = derive_synthdef(&g, 1, "t").expect("derive");
    let fade = derived_old.gains[0].index;

    let (mut controller, _nrt, mut world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 1,
        ..Options::default()
    });
    controller.add_synthdef(derived_old.def);
    let old = controller
        .synth_new("t", ROOT_GROUP_ID, AddAction::Tail)
        .expect("old synth");
    Embedded::new(&mut controller)
        .set_control(old, fade, 1.0)
        .expect("fade old in");
    let before = render(&mut world, SR as usize / 10);
    assert!(rms(&before) > 0.05, "old synth must sound");

    // Re-add the SAME name (a fresh derive, fade default baked at 0.0): the old
    // synth keeps playing, unaffected.
    let derived_new = derive_synthdef(&g, 1, "t").expect("derive");
    controller.add_synthdef(derived_new.def);
    let after_redef = render(&mut world, SR as usize / 10);
    assert!(
        rms(&after_redef) > 0.05,
        "old synth must keep playing after the re-add",
    );

    // Crossfade: spawn the replacement silent, ramp it in and the old out - the
    // whole overlap stays smooth (no hard-cut discontinuity; the largest jump is
    // the fade's first per-block lag step, a small fraction of the amplitude).
    let new = controller
        .synth_new("t", ROOT_GROUP_ID, AddAction::Tail)
        .expect("new synth");
    let mut backend = Embedded::new(&mut controller);
    backend
        .set_control(new, fade, 1.0)
        .expect("fade the new in");
    backend
        .set_control(old, fade, 0.0)
        .expect("fade the old out");
    let overlap = render(&mut world, SR as usize / 5);
    let max_delta = overlap
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_delta < 0.06,
        "crossfade must stay smooth: max_delta={max_delta}",
    );

    // The (faded-out) old synth frees without a pop; the replacement carries on.
    controller.free(old).expect("free old");
    let after = render(&mut world, SR as usize / 10);
    assert!(rms(&after) > 0.05, "the replacement carries the tone");
}

/// Root-mean-square level of `samples`.
fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt()
}

#[test]
fn kr_sinosc_lifts_via_k2a_at_out() {
    // A control-rate sine into `~out`: `Out.ar` reads its inputs strictly as
    // audio (a kr wire would be silence), so the out lifts the channel with a
    // `K2A` before its level multiply - and the rate flip changes the sig.
    let ar = derive_synthdef(&sine_to_out(), 1, "t").expect("derive").def;

    let mut g = Graph::<N>::default();
    let mut sine = SinOsc::default();
    sine.set_rate(NodeRate::Control);
    let s = g.add_node(N::SinOsc(sine));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, o, Edge::new(0.into(), 0.into()));
    let kr = derive_synthdef(&g, 1, "t").expect("derive").def;

    assert!(matches!(kr.units[0].rate, Rate::Control), "kr SinOsc");
    let k2a = kr
        .units
        .iter()
        .position(|u| u.name == "K2A")
        .expect("K2A lift");
    assert!(matches!(kr.units[k2a].rate, Rate::Audio));
    assert!(
        matches!(kr.units[k2a].inputs[0], InputRef::Unit { unit: 0, .. }),
        "the K2A lifts the kr sine",
    );
    assert!(
        !ar.units.iter().any(|u| u.name == "K2A"),
        "no lift for an audio-rate sine",
    );
    assert_ne!(
        structural_sig(&ar),
        structural_sig(&kr),
        "a rate flip must change the structural signature",
    );
}

#[test]
fn kr_into_scopeout_needs_no_lift() {
    // `~scopeout` broadcasts control-rate inputs natively - no `K2A`.
    let mut g = Graph::<N>::default();
    let mut sine = SinOsc::default();
    sine.set_rate(NodeRate::Control);
    let s = g.add_node(N::SinOsc(sine));
    let t = g.add_node(N::ScopeOut(ScopeOut::default()));
    g.add_edge(s, t, Edge::new(0.into(), 0.into()));
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    assert!(!def.units.iter().any(|u| u.name == "K2A"));
    assert_eq!(def.units.len(), 2, "SinOsc + ScopeOut only");
}

#[test]
fn rate_is_part_of_node_identity() {
    // Node identity is the erased (data-layer) content address.
    fn content_addr<T>(n: &T) -> gantz_ca::ContentAddr
    where
        T: gantz_nodetag::NodeTag + serde::Serialize + gantz_core::Node,
    {
        gantz_core::data::erase_node_typed(n)
            .unwrap()
            .content_addr()
    }
    // The default (audio) rate leaves existing addresses unchanged; kr changes
    // them. Same for ~lag.
    assert_eq!(
        content_addr(&SinOsc::default()),
        content_addr(&{
            let mut s = SinOsc::default();
            s.set_rate(NodeRate::Audio);
            s
        }),
    );
    let mut kr_sine = SinOsc::default();
    kr_sine.set_rate(NodeRate::Control);
    assert_ne!(content_addr(&SinOsc::default()), content_addr(&kr_sine));
    let mut kr_lag = Lag::default();
    kr_lag.set_rate(NodeRate::Control);
    assert_ne!(content_addr(&Lag::default()), content_addr(&kr_lag));
}

#[test]
fn kr_source_reaches_output() {
    // End to end through the real engine: a kr sine lifted via K2A still lands
    // on the output bus with its (block-held, ramped) 220 Hz content dominant.
    let mut g = Graph::<N>::default();
    let mut sine = SinOsc::default();
    sine.set_rate(NodeRate::Control);
    let s = g.add_node(N::SinOsc(sine));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");

    let (mut controller, _nrt, mut world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 1,
        ..Options::default()
    });
    controller.add_synthdef(derived.def);
    let node = controller
        .synth_new("t", ROOT_GROUP_ID, AddAction::Tail)
        .expect("synth_new");
    {
        let mut backend = Embedded::new(&mut controller);
        for gain in &derived.gains {
            backend.set_control(node, gain.index, 1.0).expect("fade in");
        }
    }

    let out = render(&mut world, SR as usize / 2);
    assert!(rms(&out) > 0.02, "kr source must be audible");
    let (m220, m440) = (goertzel(&out, 220.0), goertzel(&out, 440.0));
    assert!(
        m220 > 3.0 * m440,
        "expected 220 Hz dominant: m220={m220}, m440={m440}",
    );
}

/// The summing units of a def: `Sum3`/`Sum4` and add-selector `BinaryOpUGen`s
/// (`special_index` 0 - the gain muls select 2).
fn sum_units(def: &SynthDef) -> Vec<&UnitSpec> {
    def.units
        .iter()
        .filter(|u| {
            u.name == "Sum3"
                || u.name == "Sum4"
                || (u.name == "BinaryOpUGen" && u.special_index == 0)
        })
        .collect()
}

/// The `(unit, output)` wires among a unit's inputs.
fn unit_refs(u: &UnitSpec) -> Vec<(u32, u32)> {
    u.inputs
        .iter()
        .filter_map(|i| match i {
            InputRef::Unit { unit, output } => Some((*unit, *output)),
            _ => None,
        })
        .collect()
}

#[test]
fn single_edge_input_sums_unit_free() {
    // A lone summand passes through: no summing units, so a single-edge patch
    // derives exactly as before implicit summing existed.
    let g = sine_to_out();
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    assert!(sum_units(&def).is_empty());
}

#[test]
fn two_edges_into_one_input_sum() {
    // Two `~sinosc` into one `~out` input: the input is their unity-gain mix
    // via a single audio-rate add, and both sines land in the def (neither
    // edge is dropped).
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s0, o, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, o, Edge::new(0.into(), 0.into()));
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;

    assert_eq!(def.units.iter().filter(|u| u.name == "SinOsc").count(), 2);
    let adds = sum_units(&def);
    assert_eq!(adds.len(), 1, "one binary add for two summands");
    assert_eq!(adds[0].name, "BinaryOpUGen");
    assert!(matches!(adds[0].rate, Rate::Audio));
    // The add reads both sines.
    assert_eq!(unit_refs(adds[0]).len(), 2);
}

#[test]
fn summands_tile_sum3_and_sum4() {
    // Three summands lower to one `Sum3`; five to a `Sum4` fed back into a
    // binary add.
    for (n, expected) in [(3, vec!["Sum3"]), (5, vec!["Sum4", "BinaryOpUGen"])] {
        let mut g = Graph::<N>::default();
        let o = g.add_node(N::Out(Out::default()));
        for _ in 0..n {
            let s = g.add_node(N::SinOsc(SinOsc::default()));
            g.add_edge(s, o, Edge::new(0.into(), 0.into()));
        }
        let def = derive_synthdef(&g, 1, "t").expect("derive").def;
        let names: Vec<&str> = sum_units(&def).iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, expected, "{n} summands");
    }
}

#[test]
fn summand_order_is_canonical() {
    // The same two-source graph with its edges added in either order derives
    // the same def: summands sort canonically, so `structural_sig` (and the
    // content def name) is independent of edge insertion order.
    let build = |flip: bool| {
        let mut g = Graph::<N>::default();
        let s0 = g.add_node(N::SinOsc(SinOsc::default()));
        let s1 = g.add_node(N::SinOsc(SinOsc::default()));
        let o = g.add_node(N::Out(Out::default()));
        let (a, b) = if flip { (s1, s0) } else { (s0, s1) };
        g.add_edge(a, o, Edge::new(0.into(), 0.into()));
        g.add_edge(b, o, Edge::new(0.into(), 0.into()));
        derive_synthdef(&g, 1, "t").expect("derive").def
    };
    assert_eq!(structural_sig(&build(false)), structural_sig(&build(true)));
}

#[test]
fn mono_broadcasts_across_a_summed_stereo() {
    // A mono `~sinosc` summed with a stereo `~pack` into `~out` on a 2-channel
    // device: the sum is stereo (one add per channel) and the mono summand
    // feeds BOTH adds (broadcast, not left-only).
    let mut g = Graph::<N>::default();
    let m = g.add_node(N::SinOsc(SinOsc::default()));
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let pk = g.add_node(N::Pack(Pack::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s0, pk, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, pk, Edge::new(0.into(), 1.into()));
    g.add_edge(m, o, Edge::new(0.into(), 0.into()));
    g.add_edge(pk, o, Edge::new(0.into(), 0.into()));
    let def = derive_synthdef(&g, 2, "t").expect("derive").def;

    let adds = sum_units(&def);
    assert_eq!(adds.len(), 2, "one add per summed channel");
    let (a, b) = (unit_refs(adds[0]), unit_refs(adds[1]));
    let shared: Vec<_> = a.iter().filter(|r| b.contains(r)).collect();
    assert_eq!(shared.len(), 1, "the mono summand feeds both channels");
    // The `~out` writes both summed channels.
    let out = def.units.iter().find(|u| u.name == "Out").expect("Out");
    assert_eq!(out.inputs.len(), 1 + 2);
}

#[test]
fn narrower_summand_pads_with_silence() {
    // A stereo `~pack` summed with a 3-wide `~pack`: channels 0 and 1 sum a
    // pair each, channel 2 passes the wide summand's own channel through
    // unsummed (the narrower summand contributes silence there, which folds
    // away).
    let mut g = Graph::<N>::default();
    let mut wide = Pack::default();
    wide.set_count(3);
    let p2 = g.add_node(N::Pack(Pack::default()));
    let p3 = g.add_node(N::Pack(wide));
    let o = g.add_node(N::Out(Out::default()));
    for i in 0..5 {
        let s = g.add_node(N::SinOsc(SinOsc::default()));
        let (pk, input) = if i < 2 { (p2, i) } else { (p3, i - 2) };
        g.add_edge(s, pk, Edge::new(0.into(), (input as u16).into()));
    }
    g.add_edge(p2, o, Edge::new(0.into(), 0.into()));
    g.add_edge(p3, o, Edge::new(0.into(), 0.into()));
    let def = derive_synthdef(&g, 3, "t").expect("derive").def;

    assert_eq!(sum_units(&def).len(), 2, "adds on channels 0 and 1 only");
    let out = def.units.iter().find(|u| u.name == "Out").expect("Out");
    assert_eq!(out.inputs.len(), 1 + 3, "all three channels written");
}

#[test]
fn summed_rate_is_audio_iff_any_summand_is() {
    let build = |rates: [NodeRate; 2]| {
        let mut g = Graph::<N>::default();
        let o = g.add_node(N::Out(Out::default()));
        for rate in rates {
            let mut sine = SinOsc::default();
            sine.set_rate(rate);
            let s = g.add_node(N::SinOsc(sine));
            g.add_edge(s, o, Edge::new(0.into(), 0.into()));
        }
        derive_synthdef(&g, 1, "t").expect("derive").def
    };
    let kk = build([NodeRate::Control, NodeRate::Control]);
    assert!(matches!(sum_units(&kk)[0].rate, Rate::Control));
    let ka = build([NodeRate::Control, NodeRate::Audio]);
    assert!(matches!(sum_units(&ka)[0].rate, Rate::Audio));
}

#[test]
fn summed_sines_are_both_audible() {
    // Two sines summed into one `~out` input, the second re-tuned to 330 Hz:
    // the rendered audio carries BOTH tones - the end-to-end proof that no
    // edge is dropped.
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s0, o, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let s1_freq = derived
        .params
        .iter()
        .find(|b| b.node_path == [s1.index()])
        .expect("sine1 freq binding")
        .index;

    let (mut controller, _nrt, mut world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 1,
        ..Options::default()
    });
    controller.add_synthdef(derived.def);
    let node = controller
        .synth_new("t", ROOT_GROUP_ID, AddAction::Tail)
        .expect("synth_new");
    {
        let mut backend = Embedded::new(&mut controller);
        for gain in &derived.gains {
            backend.set_control(node, gain.index, 1.0).expect("fade in");
        }
        backend
            .set_control(node, s1_freq, 330.0)
            .expect("re-tune sine1");
    }

    let out = render(&mut world, SR as usize / 2);
    assert!(
        out.iter().all(|s| s.abs() <= 1.001),
        "output exceeded full scale"
    );
    let (m220, m330, m550) = (
        goertzel(&out, 220.0),
        goertzel(&out, 330.0),
        goertzel(&out, 550.0),
    );
    assert!(
        m220 > 5.0 * m550 && m330 > 5.0 * m550,
        "both summands must be audible: m220={m220}, m330={m330}, m550={m550}",
    );
}

#[test]
fn sum_node_mixes_mono_and_stereo() {
    // `~sum` of a mono sine and a stereo `~pack`: the node's output is the
    // stereo unity-gain mix (one add per channel, the mono input broadcast),
    // exactly the implicit-summing width policy as an explicit node.
    let mut g = Graph::<N>::default();
    let m = g.add_node(N::SinOsc(SinOsc::default()));
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let pk = g.add_node(N::Pack(Pack::default()));
    let sm = g.add_node(N::Sum(Sum::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s0, pk, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, pk, Edge::new(0.into(), 1.into()));
    g.add_edge(m, sm, Edge::new(0.into(), 0.into()));
    g.add_edge(pk, sm, Edge::new(0.into(), 1.into()));
    g.add_edge(sm, o, Edge::new(0.into(), 0.into()));
    let def = derive_synthdef(&g, 2, "t").expect("derive").def;

    let adds = sum_units(&def);
    assert_eq!(adds.len(), 2, "one add per output channel");
    let (a, b) = (unit_refs(adds[0]), unit_refs(adds[1]));
    let shared: Vec<_> = a.iter().filter(|r| b.contains(r)).collect();
    assert_eq!(shared.len(), 1, "the mono input broadcasts into both");
    let out = def.units.iter().find(|u| u.name == "Out").expect("Out");
    assert_eq!(out.inputs.len(), 1 + 2, "the stereo mix reaches both buses");
}

#[test]
fn sum_node_with_single_input_is_unit_free() {
    // A count-1 (or singly-fed) `~sum` passes its input through with no
    // units, so it derives identically to a plain wire.
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let mut sum = Sum::default();
    sum.set_count(1);
    let sm = g.add_node(N::Sum(sum));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, sm, Edge::new(0.into(), 0.into()));
    g.add_edge(sm, o, Edge::new(0.into(), 0.into()));
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    assert!(sum_units(&def).is_empty());
    // The same unit chain as a bare `~sinosc -> ~out` (params differ only by
    // node path).
    let bare = derive_synthdef(&sine_to_out(), 1, "t").expect("derive").def;
    let names = |d: &SynthDef| d.units.iter().map(|u| u.name.clone()).collect::<Vec<_>>();
    assert_eq!(
        names(&def),
        names(&bare),
        "a pass-through ~sum leaves no trace in the def",
    );
}
