//! Tests for local buffer sources. A buffer source never joins a region and
//! never crosses a bus. Each def that reads it emits it on demand, and only
//! buffer inputs receive its wire.

use std::collections::HashMap;

use gantz_core::edge::Edge;
use gantz_core::node::graph::{Graph, NodeIx};
use gantz_plyphon::flatten::{Flat, RefKind, flatten};
use gantz_plyphon::instance::{
    BusKey, DefCache, GraphTemplate, Part, derive_template, instantiate,
};
use gantz_plyphon::{
    BufferSource, Bus, DspBuilder, NodeDsp, Out, Signal, ToNodeDsp, UnitNode, derive_synthdef,
    derive_synthdefs,
};
use plyphon::Rate;
use plyphon::synthdef::{InputRef, SynthDef, UnitSpec};

/// A two-channel scratch buffer source.
struct Src;

impl NodeDsp for Src {
    fn n_dsp_inputs(&self) -> usize {
        0
    }

    fn is_buffer_source(&self) -> bool {
        true
    }

    fn ugens(&self, path: &[usize], _: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        vec![b.push_buffer(path, BufferSource::Scratch { frames: 64 }, 2)]
    }
}

/// Input 0 takes a bufnum and input 1 a signal. It emits one `Reader` unit
/// with the bufnum, or `-1`, then the signal, or `0`.
struct Reader;

impl NodeDsp for Reader {
    fn n_dsp_inputs(&self) -> usize {
        2
    }

    fn is_buffer_input(&self, input: usize) -> bool {
        input == 0
    }

    fn ugens(&self, _: &[usize], inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        let first = |i: usize, default: f32| {
            inputs[i]
                .as_ref()
                .and_then(|s| s.channel(0))
                .unwrap_or(InputRef::Constant(default))
        };
        let spec = UnitSpec::new(
            "Reader",
            Rate::Audio,
            vec![first(0, -1.0), first(1, 0.0)],
            1,
        );
        let unit = b.push_unit(spec);
        vec![Signal::mono(InputRef::Unit { unit, output: 0 })]
    }
}

/// A buffer writer sink. Input 0 takes a bufnum and input 1 a signal.
struct Writer;

impl NodeDsp for Writer {
    fn n_dsp_inputs(&self) -> usize {
        2
    }

    fn n_dsp_outputs(&self) -> usize {
        0
    }

    fn is_buffer_input(&self, input: usize) -> bool {
        input == 0
    }

    fn is_writer(&self) -> bool {
        true
    }

    fn ugens(&self, _: &[usize], inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        let first = |i: usize, default: f32| {
            inputs[i]
                .as_ref()
                .and_then(|s| s.channel(0))
                .unwrap_or(InputRef::Constant(default))
        };
        let spec = UnitSpec::new(
            "Writer",
            Rate::Audio,
            vec![first(0, -1.0), first(1, 0.0)],
            1,
        );
        b.push_unit(spec);
        vec![]
    }
}

/// A minimal erased node enum, standing in for the app's `Box<dyn Node>`.
enum N {
    Src(Src),
    Reader(Reader),
    Writer(Writer),
    Sample(gantz_plyphon::Sample),
    Unit(UnitNode),
    Out(Out),
    Bus(Bus),
    Inlet,
    Outlet,
    /// A ref standing in for an instanced graph, with its child CA and its
    /// inlet and outlet counts.
    Ref(gantz_ca::ContentAddr, usize, usize),
}

impl ToNodeDsp for N {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        match self {
            N::Src(n) => Some(n),
            N::Reader(n) => Some(n),
            N::Writer(n) => Some(n),
            N::Sample(n) => Some(n),
            N::Unit(n) => Some(n),
            N::Out(n) => Some(n),
            N::Bus(n) => Some(n),
            N::Inlet | N::Outlet | N::Ref(..) => None,
        }
    }
}

impl gantz_core::Node for N {
    fn expr(&self, _ctx: gantz_core::node::ExprCtx<'_, '_>) -> gantz_core::node::ExprResult {
        gantz_core::node::parse_expr("'()")
    }

    fn inlet(&self, _ctx: gantz_core::node::MetaCtx) -> bool {
        matches!(self, N::Inlet)
    }

    fn outlet(&self, _ctx: gantz_core::node::MetaCtx) -> bool {
        matches!(self, N::Outlet)
    }

    fn n_inputs(&self, _ctx: gantz_core::node::MetaCtx) -> usize {
        match self {
            N::Ref(_, n_in, _) => *n_in,
            _ => 0,
        }
    }

    fn n_outputs(&self, _ctx: gantz_core::node::MetaCtx) -> usize {
        match self {
            N::Ref(_, _, n_out) => *n_out,
            _ => 0,
        }
    }
}

fn sinosc() -> N {
    N::Unit(UnitNode::from_unit("SinOsc").expect("SinOsc row"))
}

fn lag() -> N {
    N::Unit(UnitNode::from_unit("Lag").expect("Lag row"))
}

fn edge(g: &mut Graph<N>, from: NodeIx, to: NodeIx, input: u16) {
    g.add_edge(from, to, Edge::new(0.into(), input.into()));
}

/// The first unit named `name` in `def`.
fn unit<'a>(def: &'a SynthDef, name: &str) -> &'a UnitSpec {
    def.units
        .iter()
        .find(|u| u.name == name)
        .unwrap_or_else(|| panic!("no `{name}` unit"))
}

/// The name of the param `input` reads, if it reads one.
fn param_of(def: &SynthDef, input: InputRef) -> Option<&str> {
    match input {
        InputRef::Param(p) => Some(def.params[p as usize].name.as_str()),
        _ => None,
    }
}

fn is_minus_one(input: InputRef) -> bool {
    matches!(input, InputRef::Constant(c) if c == -1.0)
}

#[test]
fn buffer_input_reads_its_local_source() {
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::Src(Src));
    let r = g.add_node(N::Reader(Reader));
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, s, r, 0);
    edge(&mut g, r, o, 0);
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    let reader = unit(&def, "Reader");
    assert_eq!(param_of(&def, reader.inputs[0]), Some("0/bufnum"));
}

#[test]
fn unconnected_or_multi_fed_buffer_input_reads_minus_one() {
    // Unconnected.
    let mut g = Graph::<N>::default();
    let r = g.add_node(N::Reader(Reader));
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, r, o, 0);
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    assert!(is_minus_one(unit(&def, "Reader").inputs[0]));

    // Two sources into one buffer input never sum. The input reads as
    // unconnected and no source is emitted.
    let mut g = Graph::<N>::default();
    let a = g.add_node(N::Src(Src));
    let b = g.add_node(N::Src(Src));
    let r = g.add_node(N::Reader(Reader));
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, a, r, 0);
    edge(&mut g, b, r, 0);
    edge(&mut g, r, o, 0);
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    assert!(is_minus_one(unit(&def, "Reader").inputs[0]));
    assert!(def.params.iter().all(|p| !p.name.ends_with("/bufnum")));
}

#[test]
fn signal_into_buffer_input_is_dropped() {
    let mut g = Graph::<N>::default();
    let sine = g.add_node(sinosc());
    let r = g.add_node(N::Reader(Reader));
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, sine, r, 0);
    edge(&mut g, r, o, 0);
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    assert!(is_minus_one(unit(&def, "Reader").inputs[0]));
    assert!(
        def.units.iter().all(|u| u.name != "SinOsc"),
        "a source that feeds only a buffer input is not emitted",
    );
}

#[test]
fn buffer_source_into_signal_input_is_dropped() {
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::Src(Src));
    let r = g.add_node(N::Reader(Reader));
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, s, r, 1);
    edge(&mut g, r, o, 0);
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    let reader = unit(&def, "Reader");
    assert!(matches!(reader.inputs[1], InputRef::Constant(c) if c == 0.0));
    assert!(def.params.iter().all(|p| !p.name.ends_with("/bufnum")));
}

#[test]
fn buffer_source_is_emitted_per_region() {
    // One source feeds two independent chains. A source that joined regions
    // would fuse them into one synth. It is emitted again in each instead.
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::Src(Src));
    let r1 = g.add_node(N::Reader(Reader));
    let o1 = g.add_node(N::Out(Out::default()));
    let r2 = g.add_node(N::Reader(Reader));
    let o2 = g.add_node(N::Out(Out::default()));
    edge(&mut g, s, r1, 0);
    edge(&mut g, r1, o1, 0);
    edge(&mut g, s, r2, 0);
    edge(&mut g, r2, o2, 0);
    let regions = derive_synthdefs(&g, 1, "t").expect("derive");
    assert_eq!(regions.len(), 2);
    for region in &regions {
        let def = &region.derived.def;
        assert_eq!(
            param_of(def, unit(def, "Reader").inputs[0]),
            Some("0/bufnum")
        );
        assert!(region.bus_reads.is_empty() && region.bus_writes.is_empty());
    }
}

#[test]
fn buffer_wire_passes_through_a_bus_without_a_bus() {
    // A bus would fade-gain the bufnum during a crossfade. The source is
    // emitted on the reading side instead.
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::Src(Src));
    let b = g.add_node(N::Bus(Bus::default()));
    let r = g.add_node(N::Reader(Reader));
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, s, b, 0);
    edge(&mut g, b, r, 0);
    edge(&mut g, r, o, 0);
    let regions = derive_synthdefs(&g, 1, "t").expect("derive");
    assert_eq!(regions.len(), 1);
    let region = &regions[0];
    assert!(region.bus_reads.is_empty() && region.bus_writes.is_empty());
    let def = &region.derived.def;
    assert_eq!(
        param_of(def, unit(def, "Reader").inputs[0]),
        Some("0/bufnum")
    );
}

#[test]
fn writer_runs_without_out_and_sorts_first() {
    let mut g = Graph::<N>::default();
    let sine = g.add_node(sinosc());
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, sine, o, 0);
    let s = g.add_node(N::Src(Src));
    let w = g.add_node(N::Writer(Writer));
    edge(&mut g, s, w, 0);
    edge(&mut g, sine, w, 1);
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    let index = |name: &str| def.units.iter().position(|u| u.name == name).unwrap();
    assert!(
        index("Writer") < index("Out"),
        "a writer runs before `~out`"
    );
    let writer = unit(&def, "Writer");
    assert_eq!(param_of(&def, writer.inputs[0]), Some("2/bufnum"));
    assert!(matches!(writer.inputs[1], InputRef::Unit { .. }));
}

/// Flatten `g` with every ref lowering as an instance of its child in `map`,
/// then derive its template and the cache that holds its child variants.
fn template(
    g: &Graph<N>,
    map: &HashMap<gantz_ca::ContentAddr, Graph<N>>,
) -> (GraphTemplate, DefCache) {
    fn flat<'g>(
        g: &'g Graph<N>,
        map: &'g HashMap<gantz_ca::ContentAddr, Graph<N>>,
    ) -> Graph<Flat<&'g N>> {
        let resolve = |n: &N| match n {
            N::Ref(c, _, _) => Some((*c, RefKind::Instance, map.get(c))),
            _ => None,
        };
        flatten(&|_| None, g, &resolve).expect("flatten")
    }
    let head = flat(g, map);
    let children: HashMap<_, _> = map
        .iter()
        .map(|(c, child)| (*c, flat(child, map)))
        .collect();
    let resolve = |c: &gantz_ca::ContentAddr| children.get(c);
    let mut cache = DefCache::new();
    let template = derive_template(&head, 1, &resolve, &mut cache).expect("derive");
    (template, cache)
}

#[test]
fn buffer_source_crosses_a_stage_without_a_bus() {
    // `sine -> instance -> reader` forces the reader into a later stage than
    // the source. A plain source would reach it through a `Src` bus. A buffer
    // source is emitted in the reader's region instead.
    let ca = gantz_ca::ContentAddr([1; 32]);
    let mut child = Graph::<N>::default();
    let i = child.add_node(N::Inlet);
    let l = child.add_node(lag());
    let co = child.add_node(N::Outlet);
    edge(&mut child, i, l, 0);
    edge(&mut child, l, co, 0);
    let map = HashMap::from([(ca, child)]);

    let mut g = Graph::<N>::default();
    let sine = g.add_node(sinosc());
    let inst = g.add_node(N::Ref(ca, 1, 1));
    let s = g.add_node(N::Src(Src));
    let r = g.add_node(N::Reader(Reader));
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, sine, inst, 0);
    edge(&mut g, inst, r, 1);
    edge(&mut g, s, r, 0);
    edge(&mut g, r, o, 0);

    let (t, _cache) = template(&g, &map);
    let src_path = vec![s.index()];
    let mut read_bufnum = false;
    for part in &t.parts {
        let Part::Region(region) = part else { continue };
        for b in region.bus_reads.iter().chain(&region.bus_writes) {
            if let BusKey::Src { path, .. } = &b.key {
                assert_ne!(*path, src_path, "a buffer source never makes a bus");
            }
        }
        if let Some(reader) = region.def.units.iter().find(|u| u.name == "Reader") {
            read_bufnum = param_of(&region.def, reader.inputs[0]) == Some("2/bufnum");
        }
    }
    assert!(read_bufnum, "the reader's region emits the source");
}

#[test]
fn buffer_wire_into_an_inlet_reads_unconnected() {
    let ca = gantz_ca::ContentAddr([2; 32]);
    let mut child = Graph::<N>::default();
    let i = child.add_node(N::Inlet);
    let r = child.add_node(N::Reader(Reader));
    let o = child.add_node(N::Out(Out::default()));
    edge(&mut child, i, r, 0);
    edge(&mut child, r, o, 0);
    let map = HashMap::from([(ca, child)]);

    let mut g = Graph::<N>::default();
    let s = g.add_node(N::Src(Src));
    let inst = g.add_node(N::Ref(ca, 1, 0));
    edge(&mut g, s, inst, 0);

    let (t, cache) = template(&g, &map);
    let parts = instantiate(&t, &cache);
    let reader = parts
        .iter()
        .find_map(|p| p.def.units.iter().find(|u| u.name == "Reader"))
        .expect("the child's reader derives");
    assert!(is_minus_one(reader.inputs[0]));
}

#[test]
fn table_row_buffer_socket_reads_the_source() {
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::Src(Src));
    let frames = g.add_node(N::Unit(UnitNode::from_unit("BufFrames").expect("row")));
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, s, frames, 0);
    edge(&mut g, frames, o, 0);
    let derived = derive_synthdef(&g, 1, "t").expect("derive");
    let def = &derived.def;
    assert_eq!(
        param_of(def, unit(def, "BufFrames").inputs[0]),
        Some("0/bufnum")
    );
    assert_eq!(derived.buffers.len(), 1);
    assert_eq!(derived.buffers[0].node_path, vec![s.index()]);
}

#[test]
fn describe_lists_buffer_bindings() {
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::Src(Src));
    let r = g.add_node(N::Reader(Reader));
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, s, r, 0);
    edge(&mut g, r, o, 0);
    let (t, cache) = template(&g, &HashMap::new());
    let text = gantz_plyphon::describe_parts(&instantiate(&t, &cache));
    assert!(
        text.contains("buffer [0]: scratch 64 frames 2ch"),
        "buffer line:\n{text}"
    );
}

#[test]
fn playbuf_sizes_by_its_buffer_and_scales_its_speed() {
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::Src(Src));
    let p = g.add_node(N::Unit(UnitNode::from_unit("PlayBuf").expect("row")));
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, s, p, 0);
    edge(&mut g, p, o, 0);
    let def = derive_synthdef(&g, 2, "t").expect("derive").def;
    let playbuf = unit(&def, "PlayBuf");
    assert_eq!(playbuf.num_outputs, 2, "one output per buffer channel");
    assert_eq!(playbuf.inputs.len(), 6);
    assert!(
        matches!(playbuf.inputs[5], InputRef::Constant(c) if c == 0.0),
        "doneAction"
    );
    // The speed input is the speed param times `BufRateScale` of the buffer.
    let InputRef::Unit { unit: mul, .. } = playbuf.inputs[1] else {
        panic!("speed is scaled");
    };
    let mul = &def.units[mul as usize];
    assert_eq!((mul.name.as_str(), mul.special_index), ("BinaryOpUGen", 2));
    let InputRef::Unit { unit: scale, .. } = mul.inputs[1] else {
        panic!("by a unit");
    };
    let scale = &def.units[scale as usize];
    assert_eq!(scale.name, "BufRateScale");
    assert_eq!(param_of(&def, scale.inputs[0]), Some("0/bufnum"));
}

#[test]
fn unconnected_playbuf_reads_minus_one_without_a_rate_scale() {
    let mut g = Graph::<N>::default();
    let p = g.add_node(N::Unit(UnitNode::from_unit("PlayBuf").expect("row")));
    let o = g.add_node(N::Out(Out::default()));
    edge(&mut g, p, o, 0);
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    let playbuf = unit(&def, "PlayBuf");
    assert!(is_minus_one(playbuf.inputs[0]));
    assert_eq!(playbuf.num_outputs, 1);
    assert!(def.units.iter().all(|u| u.name != "BufRateScale"));
}

/// The socket index of `name` on the table row for `unit`.
fn socket(unit: &str, name: &str) -> u16 {
    let desc = gantz_plyphon::unit_desc(unit).expect("row");
    desc.sockets()
        .position(|i| i.name() == Some(name))
        .expect("socket") as u16
}

#[test]
fn recordbuf_runs_without_out_and_broadcasts_its_input() {
    // A mono sine into a two-channel buffer writes both channels.
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::Src(Src));
    let sine = g.add_node(sinosc());
    let rec = g.add_node(N::Unit(UnitNode::from_unit("RecordBuf").expect("row")));
    edge(&mut g, s, rec, socket("RecordBuf", "buf"));
    edge(&mut g, sine, rec, socket("RecordBuf", "in"));
    let derived = derive_synthdef(&g, 1, "t").expect("a writer is a sink");
    let def = &derived.def;
    let rec = unit(def, "RecordBuf");
    assert_eq!(param_of(def, rec.inputs[0]), Some("0/bufnum"));
    assert!(
        matches!(rec.inputs[1], InputRef::Constant(c) if c == 0.0),
        "offset"
    );
    assert!(
        matches!(rec.inputs[7], InputRef::Constant(c) if c == 0.0),
        "doneAction"
    );
    assert_eq!(rec.inputs.len(), 8 + 2, "one input per buffer channel");
    assert!(matches!(rec.inputs[8], InputRef::Unit { .. }));
    assert_eq!(
        format!("{:?}", rec.inputs[8]),
        format!("{:?}", rec.inputs[9])
    );
    assert_eq!(rec.num_outputs, 1);
}

#[test]
fn write_socket_rejects_an_asset() {
    let mut g = Graph::<N>::default();
    let addr = gantz_ca::blob_addr(b"shared");
    let s = g.add_node(N::Sample(gantz_plyphon::Sample::new(addr, 1, 64, 48_000.0)));
    let sine = g.add_node(sinosc());
    let rec = g.add_node(N::Unit(UnitNode::from_unit("RecordBuf").expect("row")));
    edge(&mut g, s, rec, socket("RecordBuf", "buf"));
    edge(&mut g, sine, rec, socket("RecordBuf", "in"));
    let def = derive_synthdef(&g, 1, "t").expect("derive").def;
    assert!(is_minus_one(unit(&def, "RecordBuf").inputs[0]));
}
