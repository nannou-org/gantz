//! Offline tests for `derive_template`/`instantiate`: instance composition
//! (shared synthdefs for nested-graph refs, #295).

use std::collections::HashMap;

use gantz_core::edge::Edge;
use gantz_core::node::graph::Graph;
use gantz_plyphon::flatten::{Flat, RefKind, flatten};
use gantz_plyphon::instance::{
    BusKey, DefCache, GraphTemplate, Part, ResolvedPart, derive_template, instantiate,
};
use gantz_plyphon::{DeriveError, Lag, NodeDsp, Out, Pack, SinOsc, ToNodeDsp};

/// A minimal erased node enum, standing in for the app's `Box<dyn Node>`.
#[derive(Clone)]
enum N {
    SinOsc(SinOsc),
    Lag(Lag),
    Out(Out),
    Pack(Pack),
    Inlet,
    Outlet,
    /// A ref standing in for an instanced graph: child CA + its arity
    /// (the reference reports the child's inlet/outlet counts).
    Ref(gantz_ca::ContentAddr, usize, usize),
}

impl ToNodeDsp for N {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        match self {
            N::SinOsc(s) => Some(s),
            N::Lag(l) => Some(l),
            N::Out(o) => Some(o),
            N::Pack(p) => Some(p),
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

fn ca(byte: u8) -> gantz_ca::ContentAddr {
    gantz_ca::ContentAddr([byte; 32])
}

/// A child graph that produces a 220 Hz sine through its own `~out` (no
/// inlets/outlets): a complete, self-contained subgraph.
fn sine_out_child() -> Graph<N> {
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, o, Edge::new(0.into(), 0.into()));
    g
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

/// Flatten with every ref lowering as an instance marker.
fn flatten_with<'g>(
    graph: &'g Graph<N>,
    map: &'g HashMap<gantz_ca::ContentAddr, Graph<N>>,
) -> Graph<Flat<&'g N>> {
    let resolve = |n: &N| match n {
        N::Ref(c, _, _) => Some((*c, RefKind::Instance, map.get(c))),
        _ => None,
    };
    flatten(&|_| None, graph, &resolve).expect("flatten")
}

/// Pre-flatten every child in `map` so `derive_template`'s `resolve` can
/// return `&Graph<Flat<N>>`.
fn flat_children<'g>(
    map: &'g HashMap<gantz_ca::ContentAddr, Graph<N>>,
) -> HashMap<gantz_ca::ContentAddr, Graph<Flat<&'g N>>> {
    map.iter()
        .map(|(c, child)| (*c, flatten_with(child, map)))
        .collect()
}

/// Derive a head graph's template + cache against `map`'s children.
fn derive(
    g: &Graph<N>,
    map: &HashMap<gantz_ca::ContentAddr, Graph<N>>,
) -> Result<(GraphTemplate, DefCache), DeriveError> {
    let flat = flatten_with(g, map);
    let children = flat_children(map);
    let resolve = |c: &gantz_ca::ContentAddr| children.get(c);
    let mut cache = DefCache::new();
    let template = derive_template(&flat, 1, &resolve, &mut cache)?;
    Ok((template, cache))
}

fn regions(t: &GraphTemplate) -> Vec<&gantz_plyphon::TemplateRegion> {
    t.parts
        .iter()
        .filter_map(|p| match p {
            Part::Region(r) => Some(r),
            Part::Instance(_) => None,
        })
        .collect()
}

fn instances(t: &GraphTemplate) -> Vec<&gantz_plyphon::InstancePart> {
    t.parts
        .iter()
        .filter_map(|p| match p {
            Part::Instance(i) => Some(i),
            Part::Region(_) => None,
        })
        .collect()
}

#[test]
fn no_instances_delegates_to_derive_synthdefs() {
    // A plain graph (no markers) derives via the region path: one region per
    // `~out` sink, no cached variants, content-hashed def name.
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Out(Out::default()));
    g.add_edge(s, o, Edge::new(0.into(), 0.into()));

    let (template, cache) = derive(&g, &HashMap::new()).expect("derive");
    assert!(cache.is_empty(), "no instances -> no cached variants");
    assert_eq!(template.parts.len(), 1, "one region");
    let r = &regions(&template)[0];
    assert_eq!(
        r.def.name,
        gantz_plyphon::compile::content_def_name(r.sig),
        "defs are content-named",
    );
}

#[test]
fn two_instances_share_one_variant() {
    // parent: two instances of the same sine-producing child (each child has
    // its own `~out`, so each instance is a sink). Both share one VariantKey
    // (same child, no inlets/outlets), so the DefCache holds one entry and
    // both resolved parts name the same def.
    let map = HashMap::from([(ca(1), sine_out_child())]);
    let mut g = Graph::<N>::default();
    let _r0 = g.add_node(N::Ref(ca(1), 0, 0));
    let _r1 = g.add_node(N::Ref(ca(1), 0, 0));

    let (template, cache) = derive(&g, &map).expect("derive");
    assert_eq!(cache.len(), 1, "two instances share one variant");
    assert_eq!(instances(&template).len(), 2, "two instance parts");

    let resolved = instantiate(&template, &cache);
    assert_eq!(resolved.len(), 2, "each instance spawns the child's region");
    assert_eq!(
        resolved[0].def.name, resolved[1].def.name,
        "both instances share one installed def",
    );
    assert_ne!(resolved[0].key, resolved[1].key, "distinct identities");
    let paths: Vec<_> = resolved
        .iter()
        .map(|p| p.params[0].node_path.clone())
        .collect();
    assert!(
        paths.contains(&vec![0, 0]) && paths.contains(&vec![1, 0]),
        "bindings carry absolute instance-prefixed paths: {paths:?}",
    );
}

#[test]
fn distinct_children_produce_distinct_variants() {
    // Two instances of DIFFERENT children (different content addresses)
    // produce two distinct variants.
    let map = HashMap::from([(ca(1), sine_out_child()), (ca(2), sine_out_child())]);
    let mut g = Graph::<N>::default();
    let _r0 = g.add_node(N::Ref(ca(1), 0, 0));
    let _r1 = g.add_node(N::Ref(ca(2), 0, 0));

    let (_template, cache) = derive(&g, &map).expect("derive");
    assert_eq!(cache.len(), 2, "distinct children -> distinct variants");
}

#[test]
fn staging_diamond_derives_two_stages() {
    // src -> instance -> mix plus src -> mix directly: `src` must run before
    // the instance and `mix` after it, so they cannot share a def. The
    // staging pass splits them into two regions and the direct edge lowers to
    // an implicit `Src` bus - no `BusCycle`.
    let map = HashMap::from([(ca(1), lag_child())]);
    let mut g = Graph::<N>::default();
    let src = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1), 1, 1));
    let mix = g.add_node(N::Pack(Pack::default()));
    let out = g.add_node(N::Out(Out::default()));
    g.add_edge(src, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, mix, Edge::new(0.into(), 0.into()));
    g.add_edge(src, mix, Edge::new(0.into(), 1.into()));
    g.add_edge(mix, out, Edge::new(0.into(), 0.into()));

    let (template, _cache) = derive(&g, &map).expect("the diamond derives");
    let rs = regions(&template);
    assert_eq!(rs.len(), 2, "src and mix land in separate stage regions");
    assert_eq!(instances(&template).len(), 1);

    let src_key = BusKey::Src {
        path: vec![src.index()],
        output: 0,
    };
    let writer = rs
        .iter()
        .find(|r| r.bus_writes.iter().any(|w| w.key == src_key))
        .expect("the stage-0 region writes src's endpoint bus");
    assert_eq!(
        writer.bus_writes.len(),
        1,
        "one write serves both the instance inlet and the cross-stage read",
    );
    let reader = rs
        .iter()
        .find(|r| r.bus_reads.iter().any(|b| b.key == src_key))
        .expect("the mix region reads src's endpoint bus");
    let inst_key = BusKey::InstOut {
        path: vec![r.index()],
        outlet: 0,
        summand: 0,
    };
    assert!(
        reader.bus_reads.iter().any(|b| b.key == inst_key),
        "the mix region also reads the instance's outlet bus",
    );
    let inst = &instances(&template)[0];
    assert_eq!(inst.inlet_keys, vec![vec![src_key]]);
}

#[test]
fn width_flows_through_an_instance() {
    // A stereo (2ch pack) signal into the instance's inlet: the variant keys
    // the width, the child's `In` is 2 wide, the child's outlet carries width
    // 2 and the downstream reader's `In` sees width 2.
    let map = HashMap::from([(ca(1), lag_child())]);
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let pack = g.add_node(N::Pack(Pack::default()));
    let r = g.add_node(N::Ref(ca(1), 1, 1));
    let out = g.add_node(N::Out(Out::default()));
    g.add_edge(s0, pack, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, pack, Edge::new(0.into(), 1.into()));
    g.add_edge(pack, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, out, Edge::new(0.into(), 0.into()));

    let (template, cache) = derive(&g, &map).expect("derive");
    let inst = &instances(&template)[0];
    assert_eq!(inst.variant.inlets, vec![vec![2]], "inlet width keyed at 2");

    let child = cache.get(&inst.variant).expect("cached child template");
    let child_region = &regions(&child)[0];
    let in_read = child_region
        .bus_reads
        .iter()
        .find(|b| matches!(b.key, BusKey::IfaceIn { inlet: 0, .. }))
        .expect("the child's In reads its interface inlet");
    assert_eq!(in_read.channels, 2, "the child's In is 2 wide");
    assert_eq!(
        child.outlets[0].first().map(|(_, w)| *w),
        Some(2),
        "the child's outlet carries width 2",
    );

    let reader = regions(&template)
        .into_iter()
        .find(|r| !r.bus_reads.is_empty() && r.bus_writes.is_empty())
        .expect("the out region reads the instance's outlet");
    assert_eq!(reader.bus_reads[0].channels, 2, "the reader sees width 2");
}

#[test]
fn unconnected_inlet_bakes_silence() {
    // A child with two inlets, only the first fed: the variant records
    // `[Some(1), None]` and the child def holds exactly one interface `In`.
    let mut child = Graph::<N>::default();
    let i0 = child.add_node(N::Inlet);
    let i1 = child.add_node(N::Inlet);
    let pack = child.add_node(N::Pack(Pack::default()));
    let o = child.add_node(N::Outlet);
    child.add_edge(i0, pack, Edge::new(0.into(), 0.into()));
    child.add_edge(i1, pack, Edge::new(0.into(), 1.into()));
    child.add_edge(pack, o, Edge::new(0.into(), 0.into()));
    let map = HashMap::from([(ca(1), child)]);

    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1), 2, 1));
    let out = g.add_node(N::Out(Out::default()));
    g.add_edge(s, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, out, Edge::new(0.into(), 0.into()));

    let (template, cache) = derive(&g, &map).expect("derive");
    let inst = &instances(&template)[0];
    assert_eq!(inst.variant.inlets, vec![vec![1], vec![]]);
    assert_eq!(inst.inlet_keys.len(), 2);
    assert!(
        inst.inlet_keys[1].is_empty(),
        "unconnected inlet has no bus"
    );

    let child = cache.get(&inst.variant).expect("cached child");
    let child_region = &regions(&child)[0];
    let iface_reads = child_region
        .bus_reads
        .iter()
        .filter(|b| matches!(b.key, BusKey::IfaceIn { .. }))
        .count();
    assert_eq!(iface_reads, 1, "one In; the silent inlet bakes silence");
}

#[test]
fn instance_inlet_drives_hybrid_freq() {
    // A child `inlet -> ~sinosc.freq -> ~out` (an FM voice whose modulation
    // input is the interface inlet). Fed variant: the carrier reads its freq
    // from the interface `In` wire and bakes no freq param. Unfed variant: the
    // freq falls back to its param - a distinct `VariantKey` (inlet
    // connectivity is part of the key), so the two defs never collide in the
    // cache.
    let mut child = Graph::<N>::default();
    let i = child.add_node(N::Inlet);
    let s = child.add_node(N::SinOsc(SinOsc::default()));
    let o = child.add_node(N::Out(Out::default()));
    child.add_edge(i, s, Edge::new(0.into(), 0.into()));
    child.add_edge(s, o, Edge::new(0.into(), 0.into()));
    let map = HashMap::from([(ca(1), child)]);

    // Fed: a parent modulator wired into the instance's inlet.
    let mut g = Graph::<N>::default();
    let m = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1), 1, 0));
    g.add_edge(m, r, Edge::new(0.into(), 0.into()));
    let (template, cache) = derive(&g, &map).expect("derive");
    let inst = &instances(&template)[0];
    assert_eq!(inst.variant.inlets, vec![vec![1]]);
    let child = cache.get(&inst.variant).expect("cached child");
    let child_region = &regions(&child)[0];
    let in_read = child_region
        .bus_reads
        .iter()
        .find(|b| matches!(b.key, BusKey::IfaceIn { inlet: 0, .. }))
        .expect("the child's In reads its interface inlet");
    let osc = child_region
        .def
        .units
        .iter()
        .find(|u| u.name == "SinOsc")
        .expect("carrier SinOsc");
    assert!(
        matches!(osc.inputs[0], plyphon::synthdef::InputRef::Unit { unit, .. }
            if unit as usize == in_read.unit),
        "the carrier's freq reads the inlet In wire",
    );
    assert!(
        !child_region
            .def
            .params
            .iter()
            .any(|p| p.name.ends_with("/freq")),
        "the wired carrier bakes no freq param",
    );

    // Unfed: the same child with nothing into the inlet.
    let mut g2 = Graph::<N>::default();
    let _r = g2.add_node(N::Ref(ca(1), 1, 0));
    let (template2, cache2) = derive(&g2, &map).expect("derive");
    let inst2 = &instances(&template2)[0];
    assert_ne!(
        inst.variant, inst2.variant,
        "inlet connectivity keys the variant",
    );
    let child2 = cache2.get(&inst2.variant).expect("cached child");
    assert!(
        regions(&child2)[0]
            .def
            .params
            .iter()
            .any(|p| p.name.ends_with("/freq")),
        "an unfed inlet leaves the freq param baked",
    );
}

#[test]
fn consumed_outlet_mask_keys_the_variant() {
    // A child with two outlets, only the second consumed: the variant records
    // `[false, true]` and the child def writes exactly one outlet bus.
    let mut child = Graph::<N>::default();
    let s0 = child.add_node(N::SinOsc(SinOsc::default()));
    let s1 = child.add_node(N::Lag(Lag::default()));
    let o0 = child.add_node(N::Outlet);
    let o1 = child.add_node(N::Outlet);
    child.add_edge(s0, o0, Edge::new(0.into(), 0.into()));
    child.add_edge(s1, o1, Edge::new(0.into(), 0.into()));
    let map = HashMap::from([(ca(1), child)]);

    let mut g = Graph::<N>::default();
    let r = g.add_node(N::Ref(ca(1), 0, 2));
    let out = g.add_node(N::Out(Out::default()));
    g.add_edge(r, out, Edge::new(1.into(), 0.into()));

    let (template, cache) = derive(&g, &map).expect("derive");
    let inst = &instances(&template)[0];
    assert_eq!(inst.variant.outlets, vec![false, true]);

    let child = cache.get(&inst.variant).expect("cached child");
    assert!(child.outlets[0].is_empty(), "unconsumed outlet has no bus");
    assert!(
        !child.outlets[1].is_empty(),
        "consumed outlet carries a bus"
    );
    let writes: usize = regions(&child).iter().map(|r| r.bus_writes.len()).sum();
    assert_eq!(writes, 1, "one outlet write");
}

#[test]
fn def_names_are_stable_across_heads() {
    // The same child variant derived under two different heads (fresh caches)
    // yields identical content-hashed def names - the cross-head sharing the
    // driver's install refcounting relies on.
    let map = HashMap::from([(ca(1), sine_out_child())]);

    let mut g1 = Graph::<N>::default();
    let _r = g1.add_node(N::Ref(ca(1), 0, 0));

    let mut g2 = Graph::<N>::default();
    // A different head: its own sine plus the same child instance.
    let s = g2.add_node(N::SinOsc(SinOsc::default()));
    let o = g2.add_node(N::Out(Out::default()));
    g2.add_edge(s, o, Edge::new(0.into(), 0.into()));
    let _r = g2.add_node(N::Ref(ca(1), 0, 0));

    let (t1, c1) = derive(&g1, &map).expect("derive head 1");
    let (t2, c2) = derive(&g2, &map).expect("derive head 2");
    let name = |t: &GraphTemplate, c: &DefCache| {
        let inst = instances(t)[0];
        let child = c.get(&inst.variant).expect("cached");
        regions(&child)[0].def.name.clone()
    };
    assert_eq!(
        name(&t1, &c1),
        name(&t2, &c2),
        "same variant, same content name, regardless of head",
    );
}

#[test]
fn bus_params_are_unlagged_and_named_by_key() {
    // Every read/write bus param indexes a no-lag control in its own def,
    // named by the key's path + label - the contract the driver's post-spawn
    // `set_control` wiring relies on.
    let map = HashMap::from([(ca(1), lag_child())]);
    let mut g = Graph::<N>::default();
    let src = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1), 1, 1));
    let mix = g.add_node(N::Pack(Pack::default()));
    let out = g.add_node(N::Out(Out::default()));
    g.add_edge(src, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, mix, Edge::new(0.into(), 0.into()));
    g.add_edge(src, mix, Edge::new(0.into(), 1.into()));
    g.add_edge(mix, out, Edge::new(0.into(), 0.into()));

    let (template, cache) = derive(&g, &map).expect("derive");
    let resolved = instantiate(&template, &cache);
    for part in &resolved {
        for bus in part.bus_reads.iter().chain(&part.bus_writes) {
            let param = &part.def.params[bus.param];
            assert_eq!(param.lag, None, "bus-index params must be unlagged");
            assert!(
                param.name.ends_with("bus") || param.name.contains("/bus"),
                "bus param named by its key: {}",
                param.name,
            );
        }
    }
    // Two readers share src's endpoint bus: the child's interface In (its
    // param named by the child-local inlet marker path) and the mix region
    // (its param named by the source path + port). One bus, two synths, each
    // with its own def-local param.
    let src_key = BusKey::Src {
        path: vec![src.index()],
        output: 0,
    };
    let readers: Vec<&ResolvedPart> = resolved
        .iter()
        .filter(|p| p.bus_reads.iter().any(|b| b.key == src_key))
        .collect();
    assert_eq!(readers.len(), 2, "the child In and the mix region");
    let names: Vec<&str> = readers
        .iter()
        .map(|p| {
            let read = p.bus_reads.iter().find(|b| b.key == src_key).unwrap();
            p.def.params[read.param].name.as_str()
        })
        .collect();
    assert!(
        names.contains(&"0/bus"),
        "child-local inlet param: {names:?}"
    );
    assert!(
        names.contains(&"0/bus0"),
        "mix-side source param: {names:?}"
    );
}

#[test]
fn instance_ref_cycle_errors() {
    // A instances B instances A: no finite template exists.
    let mut a = Graph::<N>::default();
    let s = a.add_node(N::SinOsc(SinOsc::default()));
    let o = a.add_node(N::Out(Out::default()));
    a.add_edge(s, o, Edge::new(0.into(), 0.into()));
    a.add_node(N::Ref(ca(2), 0, 0));
    let mut b = Graph::<N>::default();
    b.add_node(N::Ref(ca(1), 0, 0));
    let map = HashMap::from([(ca(1), a), (ca(2), b)]);

    let mut g = Graph::<N>::default();
    g.add_node(N::Ref(ca(1), 0, 0));

    match derive(&g, &map) {
        Err(DeriveError::RefCycle(_)) => {}
        Err(e) => panic!("expected RefCycle, got {e:?}"),
        Ok(_) => panic!("expected RefCycle, derived fine"),
    }
}

#[test]
fn recursive_instantiate_prefixes_paths() {
    // head -> I1(child A), where A contains I2(child B, self-contained sine).
    // The resolved list carries B's region at the absolute prefix [I1, I2],
    // in global topo order, with absolute binding paths.
    let mut b = Graph::<N>::default();
    let s = b.add_node(N::SinOsc(SinOsc::default()));
    let o = b.add_node(N::Out(Out::default()));
    b.add_edge(s, o, Edge::new(0.into(), 0.into()));

    let mut a = Graph::<N>::default();
    let _i2 = a.add_node(N::Ref(ca(2), 0, 0));
    let map = HashMap::from([(ca(1), a), (ca(2), b)]);

    let mut g = Graph::<N>::default();
    let i1 = g.add_node(N::Ref(ca(1), 0, 0));

    let (template, cache) = derive(&g, &map).expect("derive");
    let resolved = instantiate(&template, &cache);
    assert_eq!(resolved.len(), 1, "one resolved part: B's region via A");
    let part = &resolved[0];
    assert_eq!(
        part.params[0].node_path,
        vec![i1.index(), 0, 0],
        "the sine's binding path is instance-prefixed through both levels",
    );
}

#[test]
fn describe_parts_renders_readably() {
    // Substring checks only - the exact layout is free to iterate.
    let (template, cache) = derive(&sine_out_child(), &HashMap::new()).expect("derive");
    let resolved = instantiate(&template, &cache);
    let text = gantz_plyphon::describe_parts(&resolved);
    assert!(text.contains(&resolved[0].def.name), "def name:\n{text}");
    assert!(text.contains("SinOsc ar"), "unit lines:\n{text}");
    assert!(text.contains("freq"), "param names:\n{text}");
    assert!(text.contains("[0].0: 1ch ar"), "port shapes:\n{text}");
}

#[test]
fn resolved_part_shapes_are_instance_prefixed() {
    // head -> I1(child: sine -> out): the child's osc port shape resolves at
    // the absolute path [I1, sine].
    let map = HashMap::from([(ca(1), sine_out_child())]);
    let mut g = Graph::<N>::default();
    let i1 = g.add_node(N::Ref(ca(1), 0, 0));

    let (template, cache) = derive(&g, &map).expect("derive");
    let resolved = instantiate(&template, &cache);
    assert_eq!(resolved.len(), 1);
    let shapes = &resolved[0].shapes;
    assert_eq!(shapes.len(), 1, "only the osc has a dsp output port");
    let shape = shapes[&(vec![i1.index(), 0], 0)];
    assert_eq!((shape.width, shape.rate), (1, plyphon::Rate::Audio));
}

#[test]
fn instance_to_instance_shares_one_bus() {
    // I1's consumed outlet feeds I2's inlet: both sides resolve to ONE
    // absolute bus (the bus carrying I1's child outlet signal) - no relay.
    let mut producer = Graph::<N>::default();
    let s = producer.add_node(N::SinOsc(SinOsc::default()));
    let o = producer.add_node(N::Outlet);
    producer.add_edge(s, o, Edge::new(0.into(), 0.into()));

    let mut consumer = Graph::<N>::default();
    let i = consumer.add_node(N::Inlet);
    let out = consumer.add_node(N::Out(Out::default()));
    consumer.add_edge(i, out, Edge::new(0.into(), 0.into()));

    let map = HashMap::from([(ca(1), producer), (ca(2), consumer)]);
    let mut g = Graph::<N>::default();
    let i1 = g.add_node(N::Ref(ca(1), 0, 1));
    let i2 = g.add_node(N::Ref(ca(2), 1, 0));
    g.add_edge(i1, i2, Edge::new(0.into(), 0.into()));

    let (template, cache) = derive(&g, &map).expect("derive");
    let resolved = instantiate(&template, &cache);
    assert_eq!(resolved.len(), 2, "producer region + consumer region");
    let writes: Vec<&ResolvedPart> = resolved
        .iter()
        .filter(|p| !p.bus_writes.is_empty())
        .collect();
    let reads: Vec<&ResolvedPart> = resolved
        .iter()
        .filter(|p| !p.bus_reads.is_empty())
        .collect();
    assert_eq!(writes.len(), 1, "one writer (inside I1)");
    assert_eq!(reads.len(), 1, "one reader (inside I2)");
    assert_eq!(
        writes[0].bus_writes[0].key, reads[0].bus_reads[0].key,
        "both sides name one absolute bus - no relay",
    );
    assert_eq!(
        writes[0].bus_writes[0].key,
        BusKey::Src {
            path: vec![i1.index(), s.index()],
            output: 0,
        },
        "the bus is the producer's absolutized source endpoint",
    );
}

/// A child graph of two sines both feeding one `outlet` (a multi-fed root
/// outlet).
fn two_sines_outlet_child() -> Graph<N> {
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let o = g.add_node(N::Outlet);
    g.add_edge(s0, o, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, o, Edge::new(0.into(), 0.into()));
    g
}

/// The add-selector summing units of a def (`special_index` 0; `Sum3`/`Sum4`
/// would count too, but these tests sum pairs).
fn n_adds(def: &plyphon::synthdef::SynthDef) -> usize {
    def.units
        .iter()
        .filter(|u| {
            u.name == "Sum3"
                || u.name == "Sum4"
                || (u.name == "BinaryOpUGen" && u.special_index == 0)
        })
        .count()
}

#[test]
fn multi_fed_inlet_sums_inside_the_child() {
    // Two `~sinosc` feeding ONE instance inlet: the variant keys both summand
    // widths, the instance records one bus key per summand, and the child def
    // reads two interface `In`s summed where the inlet is consumed.
    let map = HashMap::from([(ca(1), lag_child())]);
    let mut g = Graph::<N>::default();
    let s0 = g.add_node(N::SinOsc(SinOsc::default()));
    let s1 = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1), 1, 1));
    let out = g.add_node(N::Out(Out::default()));
    g.add_edge(s0, r, Edge::new(0.into(), 0.into()));
    g.add_edge(s1, r, Edge::new(0.into(), 0.into()));
    g.add_edge(r, out, Edge::new(0.into(), 0.into()));

    let (template, cache) = derive(&g, &map).expect("derive");
    let inst = &instances(&template)[0];
    assert_eq!(inst.variant.inlets, vec![vec![1, 1]]);
    assert_eq!(
        inst.inlet_keys,
        vec![vec![
            BusKey::Src {
                path: vec![s0.index()],
                output: 0,
            },
            BusKey::Src {
                path: vec![s1.index()],
                output: 0,
            },
        ]],
    );

    let child = cache.get(&inst.variant).expect("cached child");
    let region = &regions(&child)[0];
    let iface: Vec<&BusKey> = region
        .bus_reads
        .iter()
        .filter(|b| matches!(b.key, BusKey::IfaceIn { .. }))
        .map(|b| &b.key)
        .collect();
    assert_eq!(
        iface,
        vec![
            &BusKey::IfaceIn {
                inlet: 0,
                summand: 0,
            },
            &BusKey::IfaceIn {
                inlet: 0,
                summand: 1,
            },
        ],
    );
    assert_eq!(n_adds(&region.def), 1, "the child sums its inlet summands");
}

#[test]
fn instance_outlet_and_plain_node_sum_at_a_parent_input() {
    // A plain `~sinosc` and an instance's outlet both wired into one `~out`
    // input: the reader region emits an `In` per source (the sine sits a
    // stage below the instance-fed consumer, so it also crosses regions) and
    // sums them.
    let map = HashMap::from([(ca(1), lag_child())]);
    let mut g = Graph::<N>::default();
    let s = g.add_node(N::SinOsc(SinOsc::default()));
    let feed = g.add_node(N::SinOsc(SinOsc::default()));
    let r = g.add_node(N::Ref(ca(1), 1, 1));
    let out = g.add_node(N::Out(Out::default()));
    g.add_edge(feed, r, Edge::new(0.into(), 0.into()));
    g.add_edge(s, out, Edge::new(0.into(), 0.into()));
    g.add_edge(r, out, Edge::new(0.into(), 0.into()));

    let (template, _cache) = derive(&g, &map).expect("derive");
    let reader = regions(&template)
        .into_iter()
        .find(|reg| {
            reg.bus_reads
                .iter()
                .any(|b| matches!(b.key, BusKey::InstOut { .. }))
        })
        .expect("a region reads the instance outlet");
    let keys: Vec<&BusKey> = reader.bus_reads.iter().map(|b| &b.key).collect();
    assert_eq!(
        keys,
        vec![
            &BusKey::Src {
                path: vec![s.index()],
                output: 0,
            },
            &BusKey::InstOut {
                path: vec![r.index()],
                outlet: 0,
                summand: 0,
            },
        ],
    );
    assert_eq!(n_adds(&reader.def), 1, "the two reads sum at the input");
}

#[test]
fn multi_fed_outlet_exports_a_bus_per_summand() {
    // A child whose outlet is fed by two sines: the child exports one bus per
    // summand (no relay def sums inside the child), the parent reader emits
    // one `In` per summand and sums, and `instantiate` resolves each summand
    // read to the child's own endpoint bus.
    let map = HashMap::from([(ca(1), two_sines_outlet_child())]);
    let mut g = Graph::<N>::default();
    let r = g.add_node(N::Ref(ca(1), 0, 1));
    let out = g.add_node(N::Out(Out::default()));
    g.add_edge(r, out, Edge::new(0.into(), 0.into()));

    let (template, cache) = derive(&g, &map).expect("derive");
    let inst = &instances(&template)[0];
    let child = cache.get(&inst.variant).expect("cached child");
    assert_eq!(child.outlets.len(), 1);
    assert_eq!(
        child.outlets[0]
            .iter()
            .map(|(k, w)| (k.clone(), *w))
            .collect::<Vec<_>>(),
        vec![
            (
                BusKey::Src {
                    path: vec![0],
                    output: 0,
                },
                1,
            ),
            (
                BusKey::Src {
                    path: vec![1],
                    output: 0,
                },
                1,
            ),
        ],
        "one exported bus per outlet summand",
    );

    // The parent reader holds one `In` per summand and sums them.
    let reader = regions(&template)
        .into_iter()
        .find(|reg| !reg.bus_reads.is_empty())
        .expect("the out region reads the instance outlet");
    assert_eq!(reader.bus_reads.len(), 2);
    assert_eq!(n_adds(&reader.def), 1);

    // `instantiate` resolves each summand read to the child's endpoint bus,
    // absolutized under the instance path.
    let resolved = instantiate(&template, &cache);
    let rreader = resolved
        .iter()
        .find(|p| p.bus_reads.len() == 2)
        .expect("resolved reader");
    let keys: Vec<&BusKey> = rreader.bus_reads.iter().map(|b| &b.key).collect();
    assert_eq!(
        keys,
        vec![
            &BusKey::Src {
                path: vec![r.index(), 0],
                output: 0,
            },
            &BusKey::Src {
                path: vec![r.index(), 1],
                output: 0,
            },
        ],
    );
}
