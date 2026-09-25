//! Tests that `derive_synthdef` builds the right plyphon `SynthDef` from a DSP
//! graph, and that the derived def produces the expected audio when run through
//! the real engine offline.

use gantz_core::edge::Edge;
use gantz_core::node::graph::Graph;
use gantz_plyphon::{
    Backend, DeriveError, Derived, DspBuilder, Embedded, Finished, NodeDsp, NodeRate, Out, Pack,
    PortShape, ScopeOut, Signal, Sum, ToNodeDsp, UNITS, UnitNode, UnitRate, Unpack,
    derive_synthdef, structural_sig,
};
use plyphon::synthdef::{InputRef, SynthDef, UnitSpec};
use plyphon::{AddAction, Options, ROOT_GROUP_ID, Rate, World, engine};

const SR: f32 = 48_000.0;

/// A minimal erased node enum, standing in for the app's `Box<dyn Node>`.
/// `Other` stands in for any non-DSP control-rate node.
enum N {
    Out(Out),
    ScopeOut(ScopeOut),
    Pack(Pack),
    Sum(Sum),
    Unpack(Unpack),
    Unit(UnitNode),
    Other,
}

impl ToNodeDsp for N {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        match self {
            N::Out(o) => Some(o),
            N::ScopeOut(t) => Some(t),
            N::Pack(p) => Some(p),
            N::Sum(s) => Some(s),
            N::Unpack(u) => Some(u),
            N::Unit(u) => Some(u),
            N::Other => None,
        }
    }
}

fn sinosc_unit() -> UnitNode {
    UnitNode::from_unit("SinOsc").expect("SinOsc row")
}

fn lag_unit() -> UnitNode {
    UnitNode::from_unit("Lag").expect("Lag row")
}

fn sinosc() -> N {
    N::Unit(sinosc_unit())
}

fn lag() -> N {
    N::Unit(lag_unit())
}

/// A `~sinosc -> ~out` graph with default params.
fn sine_to_out() -> Graph<N> {
    let mut g = Graph::<N>::default();
    let s = g.add_node(sinosc());
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

    // Three control params. The sine's freq is 0 and the out's gain is 1. Each
    // carries the node's nominal default. The live value lives in node state
    // and is applied via set_control. The out's driver-owned fade is 2.
    assert_eq!(def.params.len(), 3);
    assert!(def.params[0].name.ends_with("/freq"));
    assert_eq!(def.params[0].default, 220.0);
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

    // Bindings map each param back to its dsp node, the sine at `[0]` and the
    // out at `[1]`. The fade has no binding. The driver alone drives it.
    assert_eq!(derived.params.len(), 2);
    assert_eq!(derived.params[0].node_path, vec![0]);
    assert_eq!(derived.params[0].index, 0);
    assert_eq!(derived.params[1].node_path, vec![1]);
    assert_eq!(derived.params[1].index, 1);

    // Unit 0 is `SinOsc.ar(freq-param, 0)`.
    assert_eq!(def.units[0].name, "SinOsc");
    assert!(matches!(def.units[0].inputs[0], InputRef::Param(0)));

    // Unit 1 is the control-rate multiply `level = gain * fade`, emitted once.
    assert_eq!(def.units[1].name, "BinaryOpUGen");
    assert_eq!(def.units[1].special_index, 2, "multiply selector");
    assert!(matches!(def.units[1].rate, Rate::Control));
    assert!(matches!(def.units[1].inputs[0], InputRef::Param(1)));
    assert!(matches!(def.units[1].inputs[1], InputRef::Param(2)));

    // Unit 2 is the audio-rate multiply `SinOsc * level`.
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

    // Unit 3 is `Out.ar(0, levelled)`.
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
    // The param value lives in node state, not in the synthdef, so a value
    // change cannot alter the def. The lag is structural, so it does.
    let g = sine_to_out();
    let base = derive_synthdef(&g, 1, "t").expect("derive").def;

    let mut g2 = Graph::<N>::default();
    let mut lagged_sine = sinosc_unit();
    lagged_sine.set_lag("freq", 0.5);
    let s = g2.add_node(N::Unit(lagged_sine));
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
    // Node identity is the erased data-layer content address.
    let content_addr = |n: &UnitNode| {
        gantz_core::data::erase_node_typed(n)
            .unwrap()
            .content_addr()
    };
    assert_eq!(
        content_addr(&sinosc_unit()),
        content_addr(&sinosc_unit()),
        "identical nodes share a content address",
    );
    let mut lagged = sinosc_unit();
    lagged.set_lag("freq", 0.5);
    assert_ne!(
        content_addr(&sinosc_unit()),
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
    // `~sinosc -> ~lag -> ~out`. The Lag UGen sits between the SinOsc and the
    // gain mul, smoothing the signal, with its own `dur` control param.
    let mut g = Graph::<N>::default();
    let s = g.add_node(sinosc());
    let l = g.add_node(lag());
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, l, Edge::new(0.into(), 0.into()));
    g.add_edge(l, o, Edge::new(0.into(), 0.into()));
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;

    // The units are SinOsc(0), Lag(1), level-mul(2), channel-mul(3) and Out(4).
    assert_eq!(def.units.len(), 5);
    assert_eq!(def.units[1].name, "Lag");
    // Lag input 0 is the SinOsc output. Input 1 is the dur param.
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

    // The `dur` param is the lag time. It defaults to 0.1 s.
    let dur = def
        .params
        .iter()
        .find(|p| p.name.ends_with("/dur"))
        .expect("dur param");
    assert_eq!(dur.default, 0.1);
}

#[test]
fn control_edge_on_root_does_not_panic() {
    // A non-DSP control source connected to the `~out` gain must not panic the
    // synthdef derivation. Gain is input 1, beyond the single dsp input. The
    // pull is seeded over only the dsp inputs, so the control edge falls
    // outside the eval conns and is ignored.
    let mut g = Graph::<N>::default();
    let s = g.add_node(sinosc());
    let o = g.add_node(N::Out(Out::default()));
    let ctrl = g.add_node(N::Other);
    g.add_edge(s, o, Edge::new(0.into(), 0.into())); // audio -> ~out input 0
    g.add_edge(ctrl, o, Edge::new(0.into(), 1.into())); // control -> ~out gain (input 1)

    let derived = derive_synthdef(&g, 1, "t").expect("derive must not panic");
    // The control source is filtered out. The dsp graph is still SinOsc, muls
    // and Out.
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
    // `~lag -> ~sinosc.freq -> ~out`. Freq is a hybrid dsp input, so the
    // connected chain emits units and the `Lag` output wire drives the
    // oscillator's freq input directly. The freq fallback param must never be
    // baked. The wire wins. An undriven param would land in the def and in
    // `structural_sig` with nothing draining its state.
    let mut g = Graph::<N>::default();
    let l = g.add_node(lag());
    let s = g.add_node(sinosc());
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(l, s, Edge::new(0.into(), 0.into())); // ~lag -> sinosc freq (dsp)
    g.add_edge(s, o, Edge::new(0.into(), 0.into())); // sinosc -> ~out (dsp)

    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let def = &derived.def;
    // The units are Lag(0), SinOsc(1), level-mul, channel-mul and Out.
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
    // `~sinosc(kr) -> ~sinosc.freq -> ~out`, a vibrato. The modulator's own
    // freq input is unconnected, so it keeps its fallback param. The carrier's
    // freq is the kr wire with no param.
    let mut modulator = sinosc_unit();
    modulator.set_rate(NodeRate::Control);
    let mut g = Graph::<N>::default();
    let m = g.add_node(N::Unit(modulator));
    let c = g.add_node(sinosc());
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
    // `~pack(2) -> ~sinosc.freq -> ~out` on a 2-channel device. The pack is
    // unconnected, a 2-wide silent group. The oscillator expands to one SinOsc
    // per freq channel and its output group is 2 wide.
    let mut g = Graph::<N>::default();
    let pk = g.add_node(N::Pack(Pack::default()));
    let s = g.add_node(sinosc());
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
    // Connecting a dsp wire into freq swaps the freq param for the wire and
    // pulls the modulator chain into the def. The structural sig changes and
    // the driver respawns with a crossfade.
    let def = |fm: bool| {
        let mut g = Graph::<N>::default();
        let m = g.add_node(sinosc());
        let s = g.add_node(sinosc());
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
    // A stale `~unpack` edge into `~sinosc.freq`, from output 1 of a count-1
    // unpack. The summand materializes no signal, so freq must fall back to
    // its param. The Steel side cannot see the port is dangling and keeps
    // queueing state updates. The driver drains only a def-present param.
    let mut unpack = Unpack::default();
    unpack.set_count(1);
    let mut g = Graph::<N>::default();
    let src = g.add_node(sinosc());
    let up = g.add_node(N::Unpack(unpack));
    let s = g.add_node(sinosc());
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
    // The carrier is emitted after the source. It reads a param, not a wire.
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
    // `~scopeout` output 1 wired into `~sinosc.freq`. That is its Steel
    // channel-count output, since the node has no dsp outputs. The summand
    // materializes no signal, so freq must fall back to its param. This guards
    // the rule that an input is `None` only when nothing materialized. Keying
    // on raw summands would bake silence here while the node's Steel expr
    // queues the numeric channel count into a param absent from the def. That
    // would grow `pending` without bound.
    let mut g = Graph::<N>::default();
    let t = g.add_node(N::ScopeOut(ScopeOut::default()));
    let s = g.add_node(sinosc());
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
    // A graph with no `~out` and no `~scopeout` has no dsp sink to root a
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
    // Every dsp output port that derivation materializes a signal for gets a
    // `(path, port) -> (width, rate)` entry. `~out` has no dsp outputs, so a
    // bare `~sinosc -> ~out` records exactly the oscillator's port.
    let derived = derive_synthdef(&sine_to_out(), 1, "t").expect("derive");
    let shape = |w, r| PortShape { width: w, rate: r };
    assert_eq!(
        derived.shapes.iter().collect::<Vec<_>>(),
        vec![(&(vec![0], 0), &shape(1, Rate::Audio))],
    );

    // Two oscs packed into one group. The pack's port is 2 wide.
    let mut g = Graph::<N>::default();
    let a = g.add_node(sinosc());
    let b = g.add_node(sinosc());
    let pk = g.add_node(N::Pack(Pack::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(a, pk, Edge::new(0.into(), 0.into()));
    g.add_edge(b, pk, Edge::new(0.into(), 1.into()));
    g.add_edge(pk, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    assert_eq!(derived.shapes[&(vec![2], 0)], shape(2, Rate::Audio));

    // A control-rate osc's port stays kr even though `~out` lifts it via K2A.
    let mut g = Graph::<N>::default();
    let mut sine = sinosc_unit();
    sine.set_rate(NodeRate::Control);
    let s = g.add_node(N::Unit(sine));
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
    // `~sinosc -> ~out` and `~sinosc -> ~scopeout`. The tap is a second sink that
    // shares the sine's chain. One synthdef therefore carries SinOsc, Out and a
    // ScopeOut with a single monitor binding at the tap's node path. The shared
    // SinOsc is emitted once.
    let mut g = Graph::<N>::default();
    let s = g.add_node(sinosc());
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

    // The binding's `scope_unit` names the ScopeOut. Its bufnum at input 0 is
    // the no-lag control param the driver sets via `set_control`. Its value
    // input 1 is the tapped sine.
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
    // A `~scopeout` fed a 2-channel signal. Its one dsp input carries the whole
    // group. The ScopeOut takes `bufnum` plus one signal input per channel. The
    // binding records the inferred width, which the driver passes to `cue_scope`.
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
    // `~lag` on a 2-channel signal emits one `Lag` unit per channel. All share
    // the single `dur` param, since params broadcast across the group. Width in
    // equals width out.
    let mut b = DspBuilder::new(1);
    let sig = stereo(InputRef::Constant(0.25), InputRef::Constant(0.5));
    let outs = lag_unit().ugens(&[0], &[Some(sig)], &mut b);
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
    // A 2-channel signal into `~out` on a 2-channel device writes channel i to
    // bus i. Each goes through its own gain multiply sharing the single gain
    // param. There is no mono fan-out. The two written wires stay distinct.
    let mut b = DspBuilder::new(2);
    let sig = stereo(InputRef::Constant(0.25), InputRef::Constant(0.5));
    let outs = Out::default().ugens(&[0], &[Some(sig)], &mut b);
    assert!(outs.is_empty());

    let Finished { def, .. } = b.finish("t");
    // One control-rate level mul, `gain * fade`, shared by two per-channel muls.
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
    // The level mul is unit 0 and the channel multiplies are units 1 and 2 in
    // this builder. Bus channel 0 reads the first and bus channel 1 the second.
    assert!(matches!(out.inputs[1], InputRef::Unit { unit: 1, .. }));
    assert!(matches!(out.inputs[2], InputRef::Unit { unit: 2, .. }));
}

#[test]
fn out_drops_excess_channels() {
    // A 3-channel signal on a 2-channel device writes only 2 channels and emits
    // only 2 gain multiplies. Dead units would pollute the structural sig and
    // burn audio CPU.
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
    // `two sines -> ~pack(2) -> ~scopeout`. The pack concatenates the two mono
    // groups into one 2-wide edge, so the tap infers 2 channels. Neither
    // routing node emits any units.
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(sinosc());
    let s1 = g.add_node(sinosc());
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
    // `two sines -> ~pack(2) -> ~out` on a 2-channel device writes channel i to
    // bus i. Each goes through its own gain multiply sharing the one gain param.
    // There is no mono fan.
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(sinosc());
    let s1 = g.add_node(sinosc());
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
    // The two written channels reach distinct sine chains, not a fanned mono.
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
    // `sine0 + sine1 -> ~pack(2) -> ~unpack(2)`, then output 1 into `~out`. This
    // pure re-routing must deliver the sine1 wire to the out. The wire is
    // identified via its freq param's node-path binding. The unreached sine0
    // chain still derives since it was pulled, but the out's gain mul must read
    // sine1.
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(sinosc());
    let s1 = g.add_node(sinosc());
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

    // The channel mul's signal input is a SinOsc unit output.
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
    // That SinOsc's freq param binds to the sine1 node path. Channel 1 of the
    // packed group is sine1.
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
    // Widening a pack from 2 to 3 inputs widens the tapped group. That changes
    // the ScopeOut's input count and so the structural sig. The driver respawns.
    let scope_def = |count: usize| {
        let mut pack = Pack::default();
        pack.set_count(count);
        let mut g = Graph::<N>::default();
        let s = g.add_node(sinosc());
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
    // An edge left hanging off a removed `~unpack` output. The count shrank to
    // 1 with the edge still on output 1. The Steel compile surfaces a
    // diagnostic, but synthdef derivation must not panic. The missing port
    // resolves to silence via `input_or_silent` in `dsp.rs`.
    let mut unpack = Unpack::default();
    unpack.set_count(1);
    let mut g = Graph::<N>::default();
    let s = g.add_node(sinosc());
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
    // A monitor-only graph, `~sinosc -> ~scopeout` with no `~out`, derives a
    // silent synthdef. A `~scopeout` is a sink in its own right, so there is
    // something to root at.
    let mut g = Graph::<N>::default();
    let s = g.add_node(sinosc());
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
    // `~sinosc -> ~scopeout` with no `~out`. The tap's ScopeOut streams every
    // sample of the sine off the audio thread into a cued scope stream. Draining
    // it recovers the full-rate 220 Hz signal. The driver appends this stream
    // into the tap's ring.
    const BLOCK: usize = 64;
    let mut g = Graph::<N>::default();
    let s = g.add_node(sinosc());
    let t = g.add_node(N::ScopeOut(ScopeOut::default()));
    g.add_edge(s, t, Edge::new(0.into(), 0.into()));

    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    // The driver sets the ScopeOut's bufnum param to the cued scope index via
    // `set_control` after spawning. Here that index is 0, the param default.
    let bufnum_param = derived.monitors[0].bufnum_param;
    let scope_unit = derived.monitors[0].scope_unit;
    assert_eq!(derived.def.units[scope_unit].name, "ScopeOut");

    // A pool large enough to hold the whole run at one chunk per block, so
    // nothing overruns before the single drain at the end.
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

/// Goertzel magnitude estimate at `freq` in Hz over mono `samples` sampled at [`SR`].
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

    // The freq and gain params default to the nodes' nominal defaults of 220 Hz
    // and 0.2, so the synth plays a 220 Hz tone with no set_control.
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
    // `~sinosc(ar) -> ~sinosc.freq -> ~out` rendered offline. The carrier's freq
    // is the modulator's raw [-1, 1] Hz signal, so the output is the faint
    // phase-wobble tone at the modulator's 220 Hz. The carrier's own 220 Hz
    // param default must never sound. The wire wins. This proves the
    // audio-rate freq path end to end through the real engine.
    let mut g = Graph::<N>::default();
    let m = g.add_node(sinosc());
    let s = g.add_node(sinosc());
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
    // A ±1 Hz freq wobbles the phase by about 1/(2*pi*220), so the tone at
    // 220 Hz has a tiny but detectable magnitude. 330 Hz carries nothing.
    let (m220, m330) = (goertzel(&a, 220.0), goertzel(&a, 330.0));
    assert!(m220 > 1e-6, "expected the 220 Hz wobble: m220={m220}");
    assert!(
        m220 > 5.0 * m330,
        "220 Hz must dominate: m220={m220}, m330={m330}",
    );
}

#[test]
fn stereo_pack_plays_per_channel_tones() {
    // `two sines -> ~pack(2) -> ~out` rendered offline on a 2-channel device.
    // Each device channel carries its own sine, 220 Hz left and 330 Hz right.
    // The second sine is re-tuned via set_control. This proves the
    // channel-per-bus write end to end through the real engine.
    const BLOCK: usize = 64;
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(sinosc());
    let s1 = g.add_node(sinosc());
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
/// scheduled time, not immediately. A freq change to 440 Hz scheduled for 0.25 s
/// leaves the first quarter-second at 220 Hz and the rest at 440 Hz. This guards the
/// `begin_scheduled`, `set_control` and `end_scheduled` wrapper and the `fill_at`
/// clock.
#[test]
fn scheduled_control_change_takes_effect_at_its_time() {
    /// OSC fixed-point units per second. NTP 32.32 fixed point, so 2^32.
    const OSC_UNITS_PER_SEC: f64 = 4_294_967_296.0;
    const BLOCK: usize = 64;

    let g = sine_to_out();
    let derived = derive_synthdef(&g, 1, "test").expect("derive");
    // The index of the sine's freq param within the synth. The sine is the node
    // at path `[0]`.
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

    // Schedule freq to 440 Hz at 0.25 s on the engine's OSC clock.
    let osc = |secs: f64| (secs * OSC_UNITS_PER_SEC) as u64;
    let switch_secs = 0.25;
    Embedded::new(&mut controller)
        .set_control_at(node, freq_index, 440.0, osc(switch_secs))
        .expect("schedule freq change");

    // Render 0.5 s in nominal blocks, anchoring the engine clock to each block's
    // nominal OSC time. The clock starts at 0 and advances by `inc` per block.
    let inc = (BLOCK as f64 * OSC_UNITS_PER_SEC / SR as f64) as u64;
    let total = (SR as usize / 2 / BLOCK) * BLOCK;
    let mut out = vec![0.0f32; total];
    for (n, block) in out.chunks_mut(BLOCK).enumerate() {
        world.fill_at(block, 1, n as u64 * inc);
    }

    // Before the switch the tone is still 220 Hz, so the change was scheduled,
    // not applied at once.
    let switch = (switch_secs * SR as f64) as usize;
    let before = &out[..switch];
    let (b220, b440) = (goertzel(before, 220.0), goertzel(before, 440.0));
    assert!(
        b220 > 4.0 * b440,
        "expected 220 Hz before the scheduled switch: m220={b220}, m440={b440}",
    );
    // After the switch, skipping the boundary block, the tone is 440 Hz.
    let after = &out[switch + BLOCK..];
    let (a220, a440) = (goertzel(after, 220.0), goertzel(after, 440.0));
    assert!(
        a440 > 4.0 * a220,
        "expected 440 Hz after the scheduled switch: m220={a220}, m440={a440}",
    );
}

#[test]
fn out_registers_a_fade_gain() {
    // `~out` carries a driver-owned fade gain, the crossfade lever. It is
    // recorded in `Derived.gains` with the fade ramp time and has no param
    // binding. No node state feeds it. The driver alone drives it. The user's
    // gain param keeps its ordinary binding for live value sync.
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
    // The fade default is baked at 0.0, the spawn-silent half of the crossfade.
    // The sig must exclude defaults, or every re-derive would respawn.
    // Changing the default to any value must leave the sig untouched.
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
    // The crossfade's fade-in half. The fade default is baked at 0.0, so the
    // synth spawns silent. Defaults seed the lag state too, so there is no
    // ramp-from-zero surprise in reverse. Restoring the fade to unity ramps the
    // output in over `FADE_LAG` rather than stepping.
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

    // Restoring the fade ramps the tone in. The first block is much quieter
    // than the settled tone. The LagControl steps toward unity per control
    // tick, and at FADE_LAG the first step is a small fraction of the target.
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
    // The driver reuses one def name per head across replacements. plyphon
    // retires the previous compiled def when a name is re-added and a running
    // synth keeps its own reference. The old synth must keep sounding while a
    // replacement installed under the same name fades in. The overlap must
    // stay smooth. This is the crossfade the driver builds on.
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

    // Re-add the same name from a fresh derive with the fade default baked at
    // 0.0. The old synth keeps playing, unaffected.
    let derived_new = derive_synthdef(&g, 1, "t").expect("derive");
    controller.add_synthdef(derived_new.def);
    let after_redef = render(&mut world, SR as usize / 10);
    assert!(
        rms(&after_redef) > 0.05,
        "old synth must keep playing after the re-add",
    );

    // Crossfade. Spawn the replacement silent, then ramp it in and the old out.
    // The whole overlap stays smooth with no hard-cut discontinuity. The
    // largest jump is the fade's first per-block lag step, a small fraction of
    // the amplitude.
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

    // The faded-out old synth frees without a pop. The replacement carries on.
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
    // A control-rate sine into `~out`. `Out.ar` reads its inputs strictly as
    // audio and a kr wire would be silence, so the out lifts the channel with a
    // `K2A` before its level multiply. The rate flip changes the sig.
    let ar = derive_synthdef(&sine_to_out(), 1, "t").expect("derive").def;

    let mut g = Graph::<N>::default();
    let mut sine = sinosc_unit();
    sine.set_rate(NodeRate::Control);
    let s = g.add_node(N::Unit(sine));
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
    // `~scopeout` broadcasts control-rate inputs natively, so no `K2A`.
    let mut g = Graph::<N>::default();
    let mut sine = sinosc_unit();
    sine.set_rate(NodeRate::Control);
    let s = g.add_node(N::Unit(sine));
    let t = g.add_node(N::ScopeOut(ScopeOut::default()));
    g.add_edge(s, t, Edge::new(0.into(), 0.into()));
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    assert!(!def.units.iter().any(|u| u.name == "K2A"));
    assert_eq!(def.units.len(), 2, "SinOsc + ScopeOut only");
}

#[test]
fn rate_is_part_of_node_identity() {
    // Node identity is the erased data-layer content address.
    fn content_addr<T>(n: &T) -> gantz_ca::ContentAddr
    where
        T: gantz_nodetag::NodeTag + serde::Serialize + gantz_core::Node,
    {
        gantz_core::data::erase_node_typed(n)
            .unwrap()
            .content_addr()
    }
    // The default audio rate leaves existing addresses unchanged. Control rate
    // changes them. The same holds for `~lag`.
    assert_eq!(
        content_addr(&sinosc_unit()),
        content_addr(&{
            let mut s = sinosc_unit();
            s.set_rate(NodeRate::Audio);
            s
        }),
    );
    let mut kr_sine = sinosc_unit();
    kr_sine.set_rate(NodeRate::Control);
    assert_ne!(content_addr(&sinosc_unit()), content_addr(&kr_sine));
    let mut kr_lag = lag_unit();
    kr_lag.set_rate(NodeRate::Control);
    assert_ne!(content_addr(&lag_unit()), content_addr(&kr_lag));
}

#[test]
fn kr_source_reaches_output() {
    // End to end through the real engine. A kr sine lifted via K2A still lands
    // on the output bus with its block-held, ramped 220 Hz content dominant.
    let mut g = Graph::<N>::default();
    let mut sine = sinosc_unit();
    sine.set_rate(NodeRate::Control);
    let s = g.add_node(N::Unit(sine));
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

/// The summing units of a def. `Sum3`, `Sum4` and add-selector `BinaryOpUGen`s
/// with `special_index` 0 count. The gain muls select 2.
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
    // A lone summand passes through with no summing units.
    let g = sine_to_out();
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    assert!(sum_units(&def).is_empty());
}

#[test]
fn two_edges_into_one_input_sum() {
    // Two `~sinosc` into one `~out` input. The input is their unity-gain mix
    // via a single audio-rate add. Both sines land in the def, so neither
    // edge is dropped.
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(sinosc());
    let s1 = g.add_node(sinosc());
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
    // Three summands lower to one `Sum3`. Five lower to a `Sum4` fed back into
    // a binary add.
    for (n, expected) in [(3, vec!["Sum3"]), (5, vec!["Sum4", "BinaryOpUGen"])] {
        let mut g = Graph::<N>::default();
        let o = g.add_node(N::Out(Out::default()));
        for _ in 0..n {
            let s = g.add_node(sinosc());
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
    // the same def. Summands sort canonically, so `structural_sig` and the
    // content def name are independent of edge insertion order.
    let build = |flip: bool| {
        let mut g = Graph::<N>::default();
        let s0 = g.add_node(sinosc());
        let s1 = g.add_node(sinosc());
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
    // device. The sum is stereo with one add per channel. The mono summand
    // broadcasts into both adds rather than feeding only the left.
    let mut g = Graph::<N>::default();
    let m = g.add_node(sinosc());
    let s0 = g.add_node(sinosc());
    let s1 = g.add_node(sinosc());
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
    // A stereo `~pack` summed with a 3-wide `~pack`. Channels 0 and 1 sum a
    // pair each. Channel 2 passes the wide summand's own channel through
    // unsummed, since the narrower summand contributes silence there and that
    // folds away.
    let mut g = Graph::<N>::default();
    let mut wide = Pack::default();
    wide.set_count(3);
    let p2 = g.add_node(N::Pack(Pack::default()));
    let p3 = g.add_node(N::Pack(wide));
    let o = g.add_node(N::Out(Out::default()));
    for i in 0..5 {
        let s = g.add_node(sinosc());
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
            let mut sine = sinosc_unit();
            sine.set_rate(rate);
            let s = g.add_node(N::Unit(sine));
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
    // Two sines summed into one `~out` input, the second re-tuned to 330 Hz.
    // The rendered audio carries both tones. That is the end-to-end proof that
    // no edge is dropped.
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(sinosc());
    let s1 = g.add_node(sinosc());
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
    // `~sum` of a mono sine and a stereo `~pack`. The node's output is the
    // stereo unity-gain mix, with one add per channel and the mono input
    // broadcast. That is the implicit-summing width policy as an explicit node.
    let mut g = Graph::<N>::default();
    let m = g.add_node(sinosc());
    let s0 = g.add_node(sinosc());
    let s1 = g.add_node(sinosc());
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
    // A count-1 or singly-fed `~sum` passes its input through with no units,
    // so it derives identically to a plain wire.
    let mut g = Graph::<N>::default();
    let s = g.add_node(sinosc());
    let mut sum = Sum::default();
    sum.set_count(1);
    let sm = g.add_node(N::Sum(sum));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, sm, Edge::new(0.into(), 0.into()));
    g.add_edge(sm, o, Edge::new(0.into(), 0.into()));
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    assert!(sum_units(&def).is_empty());
    // The same unit chain as a bare `~sinosc -> ~out`. Params differ only by
    // node path.
    let bare = derive_synthdef(&sine_to_out(), 1, "t").expect("derive").def;
    let names = |d: &SynthDef| d.units.iter().map(|u| u.name.clone()).collect::<Vec<_>>();
    assert_eq!(
        names(&def),
        names(&bare),
        "a pass-through ~sum leaves no trace in the def",
    );
}

// UnitNode descriptor-table tests.

/// Every descriptor row must derive and build through the real engine, both
/// unconnected with hybrids baked as control params, and with a signal wired
/// into socket 0. `ensure_compiled` runs plyphon's `UnitDef::build` for every
/// unit in the def. An arity mistake or a wired required-constant input, such
/// as `maxdelay` or a limiter's `dur`, then fails here rather than at runtime.
#[test]
fn every_descriptor_row_derives_and_builds() {
    let (mut controller, _nrt, _world) = engine(Options {
        sample_rate: SR as f64,
        output_channels: 2,
        ..Options::default()
    });
    for desc in UNITS {
        let name = format!("sweep-{}", desc.unit);
        let mut g = Graph::<N>::default();
        let n = g.add_node(N::Unit(UnitNode::from_desc(desc)));
        let o = g.add_node(N::Out(Out::default()));
        g.add_edge(n, o, Edge::new(0.into(), 0.into()));
        let derived = derive_synthdef(&g, 2, &name)
            .unwrap_or_else(|e| panic!("{}: derive failed: {e}", desc.unit));
        controller.add_synthdef(derived.def);
        controller
            .ensure_compiled(&name)
            .unwrap_or_else(|e| panic!("{}: def failed to build: {e:?}", desc.unit));

        // The wired variant exercises the per-channel signal path.
        if desc.n_sockets() > 0 {
            let name = format!("sweep-wired-{}", desc.unit);
            let mut g = Graph::<N>::default();
            let s = g.add_node(N::Unit(UnitNode::from_unit("SinOsc").expect("SinOsc row")));
            let n = g.add_node(N::Unit(UnitNode::from_desc(desc)));
            let o = g.add_node(N::Out(Out::default()));
            g.add_edge(s, n, Edge::new(0.into(), 0.into()));
            g.add_edge(n, o, Edge::new(0.into(), 0.into()));
            let derived = derive_synthdef(&g, 2, &name)
                .unwrap_or_else(|e| panic!("{}: wired derive failed: {e}", desc.unit));
            controller.add_synthdef(derived.def);
            controller
                .ensure_compiled(&name)
                .unwrap_or_else(|e| panic!("{}: wired def failed to build: {e:?}", desc.unit));
        }
    }
}

/// A fixed-rate row emits its fixed plyphon rate whatever the weight says.
#[test]
fn fixed_rate_rows_emit_their_rate() {
    let fixed = UNITS.iter().filter_map(|d| match d.rate {
        UnitRate::Fixed(rate) => Some((d, rate)),
        UnitRate::Any => None,
    });
    let mut seen = 0;
    for (desc, rate) in fixed {
        let other = match rate {
            NodeRate::Audio => NodeRate::Control,
            NodeRate::Control => NodeRate::Audio,
        };
        for attempt in [rate, other] {
            let mut node = UnitNode::from_desc(desc);
            node.set_rate(attempt);
            let mut g = Graph::<N>::default();
            let n = g.add_node(N::Unit(node));
            let o = g.add_node(N::Out(Out::default()));
            g.add_edge(n, o, Edge::new(0.into(), 0.into()));
            let derived = derive_synthdef(&g, 1, "t").expect("derive");
            let unit = derived
                .def
                .units
                .iter()
                .find(|u| u.name == desc.emitted_unit())
                .expect("the row's unit");
            assert_eq!(unit.rate, rate.to_plyphon(), "{}", desc.unit);
        }
        seen += 1;
    }
    assert!(seen > 0, "the table has fixed-rate rows");
}

/// A control-rate row feeding `~out` is lifted to audio by the sink.
#[test]
fn kr_only_row_lifts_to_audio_at_out() {
    let mut g = Graph::<N>::default();
    let s = g.add_node(sinosc());
    let a = g.add_node(N::Unit(UnitNode::from_unit("A2K").expect("A2K row")));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, a, Edge::new(0.into(), 0.into()));
    g.add_edge(a, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let names: Vec<&str> = derived.def.units.iter().map(|u| u.name.as_str()).collect();
    assert!(names.contains(&"A2K"), "{names:?}");
    assert!(names.contains(&"K2A"), "{names:?}");
}

/// The unit emitted by a lone row node, wired from a sine into socket 0 and
/// on to `~out`.
fn wired_row_unit(node: UnitNode) -> (Derived, UnitSpec) {
    let emitted = node.desc().emitted_unit();
    let mut g = Graph::<N>::default();
    let s = g.add_node(sinosc());
    let n = g.add_node(N::Unit(node));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, n, Edge::new(0.into(), 0.into()));
    g.add_edge(n, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let unit = derived
        .def
        .units
        .iter()
        .find(|u| u.name == emitted)
        .expect("the row's unit")
        .clone();
    (derived, unit)
}

/// `Median`'s window length is a plyphon build constant, so the row bakes it
/// from the init value ahead of the signal socket.
#[test]
fn median_bakes_length_as_a_constant() {
    let median = UnitNode::from_unit("Median").expect("Median row");
    let (derived, unit) = wired_row_unit(median.clone());
    assert!(matches!(unit.inputs[0], InputRef::Constant(c) if c == 3.0));
    assert!(matches!(unit.inputs[1], InputRef::Unit { .. }));
    let mut longer = median;
    longer.set_init("length", 5.0);
    let (derived5, unit5) = wired_row_unit(longer);
    assert!(matches!(unit5.inputs[0], InputRef::Constant(c) if c == 5.0));
    assert_ne!(
        structural_sig(&derived.def),
        structural_sig(&derived5.def),
        "the length is structural",
    );
}

/// `LFGauss` loops forever and never fires a done action, since gantz owns
/// the synth's lifecycle. Both are baked constants after the three params.
#[test]
fn lfgauss_bakes_loop_and_done_action() {
    let (_, unit) = wired_row_unit(UnitNode::from_unit("LFGauss").expect("LFGauss row"));
    assert_eq!(unit.inputs.len(), 5);
    assert!(
        matches!(unit.inputs[3], InputRef::Constant(c) if c == 1.0),
        "loop"
    );
    assert!(
        matches!(unit.inputs[4], InputRef::Constant(c) if c == 0.0),
        "doneAction"
    );
}

/// `Pluck` takes its excitation and trigger as wires, bakes the build-constant
/// `maxdelay` between them, and keeps its remaining controls as params.
#[test]
fn pluck_bakes_maxdelay_and_takes_wires() {
    let mut g = Graph::<N>::default();
    let noise = g.add_node(N::Unit(UnitNode::from_unit("WhiteNoise").expect("row")));
    let imp = g.add_node(N::Unit(UnitNode::from_unit("Impulse").expect("row")));
    let pluck = g.add_node(N::Unit(UnitNode::from_unit("Pluck").expect("row")));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(noise, pluck, Edge::new(0.into(), 0.into()));
    g.add_edge(imp, pluck, Edge::new(0.into(), 1.into()));
    g.add_edge(pluck, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let unit = derived
        .def
        .units
        .iter()
        .find(|u| u.name == "Pluck")
        .expect("Pluck unit");
    assert_eq!(unit.inputs.len(), 6);
    assert!(matches!(unit.inputs[0], InputRef::Unit { .. }), "in");
    assert!(matches!(unit.inputs[1], InputRef::Unit { .. }), "trig");
    assert!(
        matches!(unit.inputs[2], InputRef::Constant(c) if c == 0.2),
        "maxdelay"
    );
    for (ix, name) in [(3, "delay"), (4, "decay"), (5, "coef")] {
        assert!(matches!(unit.inputs[ix], InputRef::Param(_)), "{name}");
    }
}

/// A multi-output row yields one signal per unit output and asks plyphon for
/// that many outputs.
#[test]
fn multi_output_rows_expose_every_output() {
    for (unit, outputs) in [
        ("Hilbert", 2),
        ("FreeVerb2", 2),
        ("Pan4", 4),
        ("PanB", 4),
        ("PanB2", 3),
    ] {
        let node = UnitNode::from_unit(unit).expect("row");
        assert_eq!(node.n_dsp_outputs(), outputs, "{unit}");
        let (_, spec) = wired_row_unit(node);
        assert_eq!(spec.num_outputs, outputs, "{unit}");
    }
}

/// An unconnected `UnitNode` bakes each hybrid as one keyed control param.
#[test]
fn unit_node_pushes_keyed_params() {
    let mut g = Graph::<N>::default();
    let p = g.add_node(N::Unit(UnitNode::from_unit("Pulse").expect("Pulse row")));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(p, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");

    // One keyed binding per hybrid, freq and width, in def-param order.
    let keyed: Vec<_> = derived
        .params
        .iter()
        .filter_map(|b| b.key.as_deref().map(|k| (k, b.index)))
        .collect();
    assert_eq!(keyed.len(), 2, "Pulse has two hybrid params");
    for (key, index) in keyed {
        let param = &derived.def.params[index];
        assert_eq!(param.name, format!("0/{key}"));
        let pulse = derived
            .def
            .units
            .iter()
            .find(|u| u.name == "Pulse")
            .expect("Pulse unit");
        assert!(
            pulse
                .inputs
                .iter()
                .any(|i| matches!(i, InputRef::Param(p) if *p as usize == index)),
            "the Pulse unit must read its `{key}` param",
        );
    }
}

/// A signal wired into a hybrid socket replaces the param per channel. This
/// generalises `~sinosc` FM.
#[test]
fn unit_hybrid_socket_takes_the_wire() {
    let mut g = Graph::<N>::default();
    let m = g.add_node(N::Unit(UnitNode::from_unit("SinOsc").expect("row")));
    let f = g.add_node(N::Unit(UnitNode::from_unit("RLPF").expect("row")));
    let o = g.add_node(N::Out(Out::default()));
    // The modulator drives the filter's `freq` at socket 1. `in` and `rq` are
    // left unconnected.
    g.add_edge(m, f, Edge::new(0.into(), 1.into()));
    g.add_edge(f, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");

    let rlpf = derived
        .def
        .units
        .iter()
        .find(|u| u.name == "RLPF")
        .expect("RLPF unit");
    // Inputs in plyphon order. The unconnected `in` is silence, the wired
    // `freq` is a unit output and the unconnected `rq` is a param.
    assert!(
        matches!(rlpf.inputs[0], InputRef::Constant(c) if c == 0.0),
        "in is silent",
    );
    assert!(
        matches!(rlpf.inputs[1], InputRef::Unit { .. }),
        "freq must take the wire",
    );
    assert!(
        matches!(rlpf.inputs[2], InputRef::Param(_)),
        "rq falls back to its param",
    );
    // Only `rq` remains as a keyed binding for the filter node.
    assert!(
        derived
            .params
            .iter()
            .any(|b| b.node_path == vec![1] && b.key.as_deref() == Some("rq")),
        "rq must stay param-bound",
    );
    assert!(
        !derived
            .params
            .iter()
            .any(|b| b.node_path == vec![1] && b.key.as_deref() == Some("freq")),
        "a wired hybrid must not also push its param",
    );
}

/// Operator rows must select the right operator, not merely a supported one.
/// A transposed `special_index` still derives and builds, so the sweep alone
/// cannot catch it. Rendered offline, the same sine wired into both operands
/// makes `~mul` emit sin^2 and `~sub` silence, while `~abs` full-wave
/// rectifies. Everything is scaled by the out's default gain.
#[test]
fn operator_rows_apply_the_right_operator() {
    // Render `~sinosc` wired into the first `sockets` inputs of the operator
    // row identified by `unit`, through `~out`.
    let render_op = |unit: &str, sockets: usize| -> Vec<f32> {
        let mut g = Graph::<N>::default();
        let s = g.add_node(sinosc());
        let n = g.add_node(N::Unit(UnitNode::from_unit(unit).expect("operator row")));
        let o = g.add_node(N::Out(Out::default()));
        for socket in 0..sockets {
            g.add_edge(s, n, Edge::new(0.into(), (socket as u16).into()));
        }
        g.add_edge(n, o, Edge::new(0.into(), 0.into()));
        let derived = derive_synthdef(&g, 1, "op").expect("derive");

        let (mut controller, _nrt, mut world) = engine(Options {
            sample_rate: SR as f64,
            output_channels: 1,
            ..Options::default()
        });
        controller.add_synthdef(derived.def);
        let node = controller
            .synth_new("op", ROOT_GROUP_ID, AddAction::Tail)
            .expect("synth_new");
        {
            let mut backend = Embedded::new(&mut controller);
            for gain in &derived.gains {
                backend.set_control(node, gain.index, 1.0).expect("fade in");
            }
        }
        let a = render(&mut world, SR as usize / 4);
        // Analyse past the fade-in ramp.
        a[a.len() / 2..].to_vec()
    };
    let mean = |xs: &[f32]| xs.iter().sum::<f32>() / xs.len() as f32;
    let gain = Out::DEFAULT_GAIN;

    let sq = render_op("Mul", 2);
    assert!(
        sq.iter().all(|&s| s >= -1e-3),
        "sin * sin must be non-negative"
    );
    let m = mean(&sq);
    assert!(
        (m - 0.5 * gain).abs() < 0.1 * gain,
        "sin^2 mean must be gain/2, got {m}",
    );

    let zero = render_op("Sub", 2);
    assert!(
        zero.iter().all(|&s| s.abs() < 1e-4),
        "sin - sin must be silence",
    );

    let rect = render_op("Abs", 1);
    assert!(
        rect.iter().all(|&s| s >= -1e-3),
        "|sin| must be non-negative"
    );
    let peak = rect.iter().fold(0.0f32, |p, &s| p.max(s));
    assert!(
        peak > 0.9 * gain && peak <= gain * 1.01,
        "|sin| peak must reach the gain, got {peak}",
    );
    let m = mean(&rect);
    let expected = gain * 2.0 / std::f32::consts::PI;
    assert!(
        (m - expected).abs() < 0.1 * gain,
        "|sin| mean must be gain * 2/pi, got {m}",
    );
}

/// Init-only values are baked into the def as constants. Changing one changes
/// the structural sig and respawns, as does a param smoothing lag.
#[test]
fn unit_init_and_lag_are_structural() {
    let derive_with = |node: UnitNode| {
        let mut g = Graph::<N>::default();
        let n = g.add_node(N::Unit(node));
        let o = g.add_node(N::Out(Out::default()));
        g.add_edge(n, o, Edge::new(0.into(), 0.into()));
        derive_synthdef(&g, 1, "t").expect("derive")
    };
    let base = UnitNode::from_unit("CombC").expect("CombC row");
    let sig_base = structural_sig(&derive_with(base.clone()).def);

    // The default `maxdelay` reaches the CombC unit as a constant.
    let comb_inputs = |derived: &Derived| {
        derived
            .def
            .units
            .iter()
            .find(|u| u.name == "CombC")
            .expect("CombC unit")
            .inputs
            .clone()
    };
    let has_const = |inputs: &[InputRef], v: f32| {
        inputs
            .iter()
            .any(|i| matches!(i, InputRef::Constant(c) if *c == v))
    };
    assert!(
        has_const(&comb_inputs(&derive_with(base.clone())), 0.2),
        "default maxdelay must be baked as a constant",
    );

    let mut resized = base.clone();
    resized.set_init("maxdelay", 0.5);
    let resized_derived = derive_with(resized);
    assert!(has_const(&comb_inputs(&resized_derived), 0.5));
    assert_ne!(
        sig_base,
        structural_sig(&resized_derived.def),
        "resizing the delay line must respawn",
    );

    let mut lagged = base.clone();
    lagged.set_lag("delay", 0.02);
    assert_ne!(
        sig_base,
        structural_sig(&derive_with(lagged).def),
        "a param smoothing lag bakes a LagControl and must respawn",
    );
}

/// A multi-output unit's ports each carry the full channel group. A mono-fed
/// `Pan2` yields two mono ports from one unit. A stereo-fed one expands to
/// two units with two stereo ports.
#[test]
fn unit_multi_out_expands_per_channel() {
    // Mono. One Pan2 with two mono ports.
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::Unit(UnitNode::from_unit("SinOsc").expect("row")));
    let p = g.add_node(N::Unit(UnitNode::from_unit("Pan2").expect("row")));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, p, Edge::new(0.into(), 0.into()));
    g.add_edge(p, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let pan_count = |derived: &Derived| {
        derived
            .def
            .units
            .iter()
            .filter(|u| u.name == "Pan2")
            .count()
    };
    assert_eq!(pan_count(&derived), 1);
    let shape = |derived: &Derived, port: usize| derived.shapes[&(vec![1usize], port)];
    assert_eq!(shape(&derived, 0).width, 1, "left port is mono");
    assert_eq!(shape(&derived, 1).width, 1, "right port is mono");

    // Stereo. A 2-wide group fans out one Pan2 per channel. Each port is
    // 2 wide.
    let mut g = Graph::<N>::default();
    let a = g.add_node(N::Unit(UnitNode::from_unit("SinOsc").expect("row")));
    let b = g.add_node(N::Unit(UnitNode::from_unit("SinOsc").expect("row")));
    let pk = g.add_node(N::Pack(Pack::default()));
    let p = g.add_node(N::Unit(UnitNode::from_unit("Pan2").expect("row")));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(a, pk, Edge::new(0.into(), 0.into()));
    g.add_edge(b, pk, Edge::new(0.into(), 1.into()));
    g.add_edge(pk, p, Edge::new(0.into(), 0.into()));
    g.add_edge(p, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    assert_eq!(pan_count(&derived), 2, "one Pan2 per input channel");
    let shape = |derived: &Derived, port: usize| derived.shapes[&(vec![3usize], port)];
    assert_eq!(shape(&derived, 0).width, 2, "left port carries the group");
    assert_eq!(shape(&derived, 1).width, 2, "right port carries the group");
}

/// Unconnected hybrids broadcast one shared control param across the whole
/// channel group, as the `~lag` dur does.
#[test]
fn unit_params_broadcast_across_the_group() {
    let mut g = Graph::<N>::default();
    let a = g.add_node(N::Unit(UnitNode::from_unit("SinOsc").expect("row")));
    let b = g.add_node(N::Unit(UnitNode::from_unit("SinOsc").expect("row")));
    let pk = g.add_node(N::Pack(Pack::default()));
    let f = g.add_node(N::Unit(UnitNode::from_unit("LPF").expect("row")));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(a, pk, Edge::new(0.into(), 0.into()));
    g.add_edge(b, pk, Edge::new(0.into(), 1.into()));
    g.add_edge(pk, f, Edge::new(0.into(), 0.into()));
    g.add_edge(f, o, Edge::new(0.into(), 0.into()));
    let derived = derive_synthdef(&g, 1, "t").expect("derive");

    let lpfs: Vec<_> = derived
        .def
        .units
        .iter()
        .filter(|u| u.name == "LPF")
        .collect();
    assert_eq!(lpfs.len(), 2, "one LPF per channel");
    // The filter's freq binding. The sines push their own keyed freqs.
    let freq_binding = derived
        .params
        .iter()
        .find(|b| b.node_path == vec![3] && b.key.as_deref() == Some("freq"))
        .expect("freq binding");
    for lpf in &lpfs {
        assert!(
            lpf.inputs
                .iter()
                .any(|i| matches!(i, InputRef::Param(p) if *p as usize == freq_binding.index)),
            "each channel's LPF must share the one freq param",
        );
    }
}
