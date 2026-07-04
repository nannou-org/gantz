use dyn_clone::DynClone;
use dyn_hash::DynHash;
use std::any::Any;

/// A top-level blanket trait providing trait object cloning, hashing, and serialization.
pub trait Node:
    Any + DynClone + DynHash + gantz_ca::CaHash + gantz_core::Node + gantz_egui::NodeUi + Send + Sync
{
}

dyn_clone::clone_trait_object!(Node);
dyn_hash::hash_trait_object!(Node);

impl Node for gantz_core::node::Apply {}
impl Node for gantz_core::node::Branch {}
impl Node for gantz_core::node::Delay {}
impl Node for gantz_core::node::Expr {}
impl Node for gantz_core::node::Identity {}
impl Node for gantz_core::node::graph::Inlet {}
impl Node for gantz_core::node::graph::Outlet {}

impl Node for gantz_std::Bang {}
impl Node for gantz_std::Log {}
impl Node for gantz_std::Number {}

impl Node for gantz_egui::node::FnNamedRef {}
impl Node for gantz_egui::node::NamedRef {}

impl Node for gantz_egui::node::Comment {}
impl Node for bevy_gantz_egui::node::UpdateBang {}
impl Node for bevy_gantz_egui::node::TickBang {}
impl Node for gantz_egui::node::Inspect {}
impl Node for gantz_egui::node::Plot {}

impl Node for gantz_plyphon::SinOsc {}
impl Node for gantz_plyphon::Out {}
impl Node for gantz_plyphon::Lag {}
impl Node for gantz_plyphon::ScopeOut {}
impl Node for gantz_plyphon::Pack {}
impl Node for gantz_plyphon::Unpack {}
impl Node for gantz_plyphon::Bus {}

// `Box<dyn Node>`'s `Serialize`/`Deserialize`: compiled dispatch over the
// full node set, keyed by each type's `gantz_nodetag::NodeTag`. Adding a
// node type to the app is an `impl Node` above plus one line here (the
// `node_set_roundtrips_through_datum` gate test enforces the latter).
gantz_format::impl_node_set_serde! {
    dyn Node {
        gantz_core::node::Apply,
        gantz_core::node::Branch,
        gantz_core::node::Delay,
        gantz_core::node::Expr,
        gantz_core::node::Identity,
        gantz_core::node::graph::Inlet,
        gantz_core::node::graph::Outlet,
        gantz_std::Bang,
        gantz_std::Log,
        gantz_std::Number,
        gantz_egui::node::FnNamedRef,
        gantz_egui::node::NamedRef,
        gantz_egui::node::Comment,
        bevy_gantz_egui::node::UpdateBang,
        bevy_gantz_egui::node::TickBang,
        gantz_egui::node::Inspect,
        gantz_egui::node::Plot,
        gantz_plyphon::SinOsc,
        gantz_plyphon::Out,
        gantz_plyphon::Lag,
        gantz_plyphon::ScopeOut,
        gantz_plyphon::Pack,
        gantz_plyphon::Unpack,
        gantz_plyphon::Bus,
    }
}

impl From<gantz_egui::node::NamedRef> for Box<dyn Node> {
    fn from(named: gantz_egui::node::NamedRef) -> Self {
        Box::new(named)
    }
}

// Lets the reference-resync / rename machinery find `NamedRef`s within an
// erased node by downcasting.
impl gantz_egui::sync::AsNamedRefMut for Box<dyn Node> {
    fn as_named_ref_mut(&mut self) -> Option<&mut gantz_egui::node::NamedRef> {
        ((&mut **self) as &mut dyn Any).downcast_mut::<gantz_egui::node::NamedRef>()
    }
}

impl gantz_egui::sync::AsNamedRef for Box<dyn Node> {
    fn as_named_ref(&self) -> Option<&gantz_egui::node::NamedRef> {
        ((&**self) as &dyn Any).downcast_ref::<gantz_egui::node::NamedRef>()
    }
}

impl bevy_gantz_egui::node::ToUpdateBang for Box<dyn Node> {
    fn to_update_bang(&self) -> Option<&bevy_gantz_egui::node::UpdateBang> {
        let any: &dyn std::any::Any = &**self;
        any.downcast_ref()
    }
}

impl bevy_gantz_egui::node::ToTickBang for Box<dyn Node> {
    fn to_tick_bang(&self) -> Option<&bevy_gantz_egui::node::TickBang> {
        let any: &dyn std::any::Any = &**self;
        any.downcast_ref()
    }
}

// Lets the synthdef compiler and dsp driver find DSP nodes within an erased
// node by downcasting to each known `NodeDsp` type (mirrors `ToTickBang`).
impl gantz_plyphon::ToNodeDsp for Box<dyn Node> {
    fn to_node_dsp(&self) -> Option<&dyn gantz_plyphon::NodeDsp> {
        let any: &dyn Any = &**self;
        if let Some(n) = any.downcast_ref::<gantz_plyphon::SinOsc>() {
            return Some(n);
        }
        if let Some(n) = any.downcast_ref::<gantz_plyphon::Out>() {
            return Some(n);
        }
        if let Some(n) = any.downcast_ref::<gantz_plyphon::Lag>() {
            return Some(n);
        }
        if let Some(n) = any.downcast_ref::<gantz_plyphon::ScopeOut>() {
            return Some(n);
        }
        if let Some(n) = any.downcast_ref::<gantz_plyphon::Pack>() {
            return Some(n);
        }
        if let Some(n) = any.downcast_ref::<gantz_plyphon::Unpack>() {
            return Some(n);
        }
        if let Some(n) = any.downcast_ref::<gantz_plyphon::Bus>() {
            return Some(n);
        }
        None
    }
}

impl Node for Box<dyn Node> {}

/// The composite `.gantz` keyword sugar for this app's full node set: the
/// `gantz_core`, `gantz_std`, `gantz_egui` and `bevy_gantz_egui` node sugars.
impl gantz_format::NodeSugar for Box<dyn Node> {
    fn sugar() -> gantz_format::Sugars<'static> {
        gantz_format::Sugars(vec![
            &gantz_format::CoreSugar,
            &gantz_std::StdSugar,
            &gantz_egui::EguiSugar,
            &bevy_gantz_egui::BevySugar,
            &gantz_plyphon::PlyphonSugar,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::Node;

    /// Fire the push entrypoint of the node at `node_ix` (a flat-graph index).
    fn fire_push(
        vm: &mut gantz_core::steel::steel_vm::engine::Engine,
        eps: &[gantz_core::compile::Entrypoint],
        node_ix: usize,
    ) {
        use gantz_core::compile::{EvalKind, entry_fn_name};
        let ep = eps
            .iter()
            .find(|ep| {
                ep.0.iter()
                    .any(|s| s.kind == EvalKind::Push && s.path == [node_ix])
            })
            .expect("push entrypoint");
        vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
            .expect("push entrypoint");
    }

    /// Read an `inspect` node's stored value as a list of `f64`s.
    /// Gate test for the `.gantz` text format: confirm `Box<dyn Node>`
    /// round-trips through the self-describing `gantz_format::Datum` codec.
    /// The format bridges node specs to/from the node set's serde dispatch
    /// (`impl_node_set_serde!`) via this codec rather than hand-writing a
    /// parser per node type, so the mechanism must hold for every listed
    /// node - a type missing from the macro's list fails here.
    #[test]
    fn node_set_roundtrips_through_datum() {
        use gantz_format::{Datum, from_datum, to_datum};

        fn node_datum(tag: &str, fields: Vec<(&str, Datum)>) -> Datum {
            let mut entries = vec![("type".to_string(), Datum::Str(tag.to_string()))];
            entries.extend(fields.into_iter().map(|(k, v)| (k.to_string(), v)));
            Datum::Map(entries)
        }
        fn type_of(d: &Datum) -> Option<&str> {
            match d {
                Datum::Map(entries) => {
                    entries
                        .iter()
                        .find(|(k, _)| k == "type")
                        .and_then(|(_, v)| match v {
                            Datum::Str(s) => Some(s.as_str()),
                            _ => None,
                        })
                }
                _ => None,
            }
        }

        let cases = [
            node_datum("Inlet", vec![]),
            node_datum("Outlet", vec![]),
            node_datum("Apply", vec![]),
            node_datum("Delay", vec![]),
            node_datum("Identity", vec![]),
            node_datum("Bang", vec![]),
            node_datum("Inspect", vec![]),
            node_datum("UpdateBang", vec![]),
            node_datum(
                "TickBang",
                vec![(
                    "interval",
                    Datum::Map(vec![("Duration".to_string(), Datum::F64(0.5))]),
                )],
            ),
            node_datum(
                "TickBang",
                vec![(
                    "interval",
                    Datum::Map(vec![("Rate".to_string(), Datum::F64(60.0))]),
                )],
            ),
            node_datum("Number", vec![]),
            node_datum("Expr", vec![("src", Datum::Str("(* $l $r)".into()))]),
            node_datum(
                "Comment",
                vec![
                    ("text", Datum::Str("hi".into())),
                    ("size", Datum::Seq(vec![Datum::U64(100), Datum::U64(40)])),
                ],
            ),
            node_datum(
                "Branch",
                vec![
                    ("src", Datum::Str("(if $x (list 0 0) (list 1 0))".into())),
                    (
                        "branches",
                        Datum::Seq(vec![Datum::Str("10".into()), Datum::Str("01".into())]),
                    ),
                ],
            ),
            node_datum(
                "NamedRef",
                vec![
                    ("ref_", Datum::Str("0".repeat(64))),
                    ("name", Datum::Str("mul".into())),
                ],
            ),
            node_datum(
                "Plot",
                vec![
                    ("mode", Datum::Str("Signal".into())),
                    ("style", Datum::Str("Line".into())),
                    ("capacity", Datum::U64(128)),
                    ("width", Datum::U64(160)),
                    ("height", Datum::U64(90)),
                    (
                        "color",
                        Datum::Seq(vec![
                            Datum::U64(10),
                            Datum::U64(20),
                            Datum::U64(30),
                            Datum::U64(255),
                        ]),
                    ),
                    ("show_grid", Datum::Bool(false)),
                    ("show_axes", Datum::Bool(true)),
                    ("interactive", Datum::Bool(true)),
                    ("margin", Datum::Bool(false)),
                    ("y_min", Datum::F64(1.5)),
                    ("y_max", Datum::Null),
                ],
            ),
            node_datum("SinOsc", vec![]),
            node_datum("SinOsc", vec![("rate", Datum::Str("kr".into()))]),
            node_datum("Out", vec![]),
            node_datum("Lag", vec![]),
            node_datum("Lag", vec![("rate", Datum::Str("kr".into()))]),
            node_datum("ScopeOut", vec![]),
            node_datum("ScopeOut", vec![("size", Datum::U64(64))]),
            // Legacy: pre-channel-group `~scopeout` data carries a `channels`
            // field; serde ignores the unknown field so old registries still load
            // (the count is inferred from the input signal's width now).
            node_datum(
                "ScopeOut",
                vec![("channels", Datum::U64(3)), ("size", Datum::U64(64))],
            ),
            node_datum("Pack", vec![]),
            node_datum("Pack", vec![("count", Datum::U64(4))]),
            node_datum("Unpack", vec![]),
            node_datum("Unpack", vec![("count", Datum::U64(4))]),
            node_datum("Bus", vec![]),
        ];
        for value in cases {
            let node: Box<dyn Node> = from_datum(value.clone())
                .unwrap_or_else(|e| panic!("from_datum failed for {value:?}: {e}"));
            let back = to_datum(&node).unwrap_or_else(|e| panic!("to_datum failed: {e}"));
            // The re-serialized form must itself round-trip identically, proving
            // both directions of the node-set <-> Datum bridge are stable.
            let node2: Box<dyn Node> = from_datum(back.clone())
                .unwrap_or_else(|e| panic!("re-deserialize failed for {back:?}: {e}"));
            let back2 = to_datum(&node2).unwrap_or_else(|e| panic!("re-serialize failed: {e}"));
            assert_eq!(back, back2, "round-trip not stable for {value:?}");
            assert_eq!(
                type_of(&back),
                type_of(&value),
                "type tag changed for {value:?}",
            );
        }
    }

    /// Pins the exact node serde wire format in both the `Datum` codec (the
    /// `.gantz` text format bridge) and RON (registry persistence): a map
    /// carrying the `type` tag entry alongside the node's own fields.
    ///
    /// Exports and persisted registries produced by earlier builds must keep
    /// loading, and vice versa, so these literals must never change. Covers
    /// the three struct shapes: unit (`Bang`), fields (`Expr`) and newtype
    /// (`FnNamedRef`, which flattens to its inner node's fields).
    #[test]
    fn node_serde_wire_format() {
        use gantz_format::{Datum, from_datum, to_datum};

        let bang: Box<dyn Node> = Box::new(gantz_std::Bang);
        let expr: Box<dyn Node> = Box::new(gantz_core::node::Expr::new("(+ $a $b)").unwrap());
        let fn_named_ref: Box<dyn Node> =
            Box::new(gantz_core::node::Fn(gantz_egui::node::NamedRef::new(
                "mul".to_string(),
                gantz_core::node::Ref::new(gantz_ca::ContentAddr::from([0u8; 32])),
            )));

        // Note: the `Datum` codec sorts map entries by key; RON preserves
        // the written order (tag first, then declaration-order fields).
        fn datum(entries: Vec<(&str, Datum)>) -> Datum {
            Datum::Map(
                entries
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            )
        }
        let zeros = "0".repeat(64);
        let cases = [
            (
                bang,
                datum(vec![("type", Datum::Str("Bang".into()))]),
                r#"{"type":"Bang"}"#.to_string(),
            ),
            (
                expr,
                datum(vec![
                    ("outputs", Datum::U64(1)),
                    ("src", Datum::Str("(+ $a $b)".into())),
                    ("type", Datum::Str("Expr".into())),
                ]),
                r#"{"type":"Expr","src":"(+ $a $b)","outputs":1}"#.to_string(),
            ),
            (
                fn_named_ref,
                datum(vec![
                    ("name", Datum::Str("mul".into())),
                    ("ref_", Datum::Str(zeros.clone())),
                    ("sync", Datum::Bool(false)),
                    ("type", Datum::Str("FnNamedRef".into())),
                ]),
                // RON preserves the `Ref(ContentAddr)` newtype nesting.
                format!(
                    r#"{{"type":"FnNamedRef","ref_":(("{zeros}")),"name":"mul","sync":false}}"#
                ),
            ),
        ];

        for (node, expected_datum, expected_ron) in cases {
            let datum = to_datum(&node).unwrap();
            assert_eq!(datum, expected_datum);
            let ron = ron::to_string(&node).unwrap();
            assert_eq!(ron, expected_ron);
            // Both representations load back and re-serialize identically.
            let from_datum: Box<dyn Node> = from_datum(datum).unwrap();
            assert_eq!(to_datum(&from_datum).unwrap(), expected_datum);
            let from_ron: Box<dyn Node> = ron::de::from_str(&ron).unwrap();
            assert_eq!(to_datum(&from_ron).unwrap(), expected_datum);
        }
    }

    /// `PlyphonSugar` is composed into the app's `NodeSugar`, so the DSP nodes
    /// serialize as their `~`-prefixed keyword forms (not the generic
    /// `(node "SinOsc" ...)` fallback). Guards that the sugar stays wired in.
    #[test]
    fn dsp_nodes_use_plyphon_keyword_sugar() {
        use gantz_format::{NodeSugar, Sugar, to_datum};

        let sugar = <Box<dyn Node> as NodeSugar>::sugar();
        let cases: [(Box<dyn Node>, &str, &str); 7] = [
            (
                Box::new(gantz_plyphon::SinOsc::default()),
                "SinOsc",
                "~sinosc",
            ),
            (Box::new(gantz_plyphon::Out::default()), "Out", "~out"),
            (Box::new(gantz_plyphon::Lag::default()), "Lag", "~lag"),
            (
                Box::new(gantz_plyphon::ScopeOut::default()),
                "ScopeOut",
                "~scopeout",
            ),
            (Box::new(gantz_plyphon::Pack::default()), "Pack", "~pack"),
            (
                Box::new(gantz_plyphon::Unpack::default()),
                "Unpack",
                "~unpack",
            ),
            (Box::new(gantz_plyphon::Bus::default()), "Bus", "~bus"),
        ];
        for (node, tag, expected) in cases {
            let datum = to_datum(&node).expect("to_datum");
            assert_eq!(
                sugar.write_spec(tag, &datum).as_deref(),
                Some(expected),
                "default {tag} should sugar to bare `{expected}`",
            );
        }
    }

    /// The DSP nodes are inert in the control-rate Steel world (they emit
    /// placeholder exprs and declare no entrypoints), so a `~sinosc -> ~out` graph
    /// still compiles through the VM without error - and the app's `ToNodeDsp`
    /// impl lets a synthdef derive from the same graph. This guards the
    /// two-independent-backends design end to end.
    #[test]
    fn dsp_nodes_are_steel_inert_and_derive() {
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;
        use gantz_plyphon::derive_synthdef;
        type G = Graph<Box<dyn Node>>;

        // Two sines packed into one 2-wide edge, across a `~bus`, unpacked,
        // channel 1 to the out - covering the whole dsp node set including the
        // routing pair and the boundary (which the single-def `derive_synthdef`
        // fuses to a plain wire).
        let mut g: G = Graph::default();
        let s0 = g.add_node(Box::new(gantz_plyphon::SinOsc::default()) as Box<dyn Node>);
        let s1 = g.add_node(Box::new(gantz_plyphon::SinOsc::default()) as Box<dyn Node>);
        let pk = g.add_node(Box::new(gantz_plyphon::Pack::default()) as Box<dyn Node>);
        let bus = g.add_node(Box::new(gantz_plyphon::Bus::default()) as Box<dyn Node>);
        let up = g.add_node(Box::new(gantz_plyphon::Unpack::default()) as Box<dyn Node>);
        let o = g.add_node(Box::new(gantz_plyphon::Out::default()) as Box<dyn Node>);
        g.add_edge(s0, pk, Edge::new(0.into(), 0.into()));
        g.add_edge(s1, pk, Edge::new(0.into(), 1.into()));
        g.add_edge(pk, bus, Edge::new(0.into(), 0.into()));
        g.add_edge(bus, up, Edge::new(0.into(), 0.into()));
        g.add_edge(up, o, Edge::new(1.into(), 0.into()));

        // Steel-inert: compiles through the control-rate VM (no entrypoints, no
        // error) even though it is a pure-dsp graph.
        let get_node = |_: &gantz_ca::ContentAddr| -> Option<&dyn gantz_core::Node> { None };
        let config = gantz_core::compile::Config::default();
        gantz_core::vm::init(&get_node, &g, &[], &config)
            .expect("DSP graph must compile in the Steel VM");

        // The `~out` sink is discoverable via `ToNodeDsp` and a synthdef derives;
        // the routing pair emits no units.
        let derived = derive_synthdef(&g, 2, "test").expect("derive");
        assert_eq!(
            derived.def.units.len(),
            5,
            "2 SinOsc + level/channel muls + Out",
        );
    }

    /// A nested graph of DSP nodes sounds through the flattening pass: the
    /// child's `~lag` splices into the parent's synthdef carrying its nested
    /// path, and that path reaches the node's live param state in the VM -
    /// exactly the contract the audio driver's param sync relies on. Guards
    /// the whole nested-DSP pipeline (registry ref resolution, flattening,
    /// derivation, VM state bridge) end to end with the app's real node type.
    #[test]
    fn nested_dsp_graph_flattens_derives_and_bridges_state() {
        use gantz_egui::sync::AsNamedRef;
        use gantz_plyphon::ToNodeDsp;
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        // child `env:1`: inlet -> ~lag -> outlet, nested into
        // parent `env`: ~sinosc -> ref -> ~out.
        let text = "\
(graph env:1
  (i inlet) (l ~lag) (o outlet)
  (-> i (l 0)) (-> l o))

(graph env
  (s ~sinosc) (sub (ref env:1)) (out ~out)
  (-> s (sub 0)) (-> sub (out 0)))";
        let export: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(text, Duration::from_secs(0)).expect("from_str");
        let parent_head = gantz_ca::Head::Branch("env".into());
        let child_head = gantz_ca::Head::Branch("env:1".into());
        let parent = export.registry.head_graph(&parent_head).expect("env graph");
        let child = export
            .registry
            .head_graph(&child_head)
            .expect("env:1 graph");

        // The indices the flattened path must carry: the ref within the parent,
        // the lag within the child (its only dsp node).
        let ref_ix = parent
            .node_indices()
            .find(|&n| parent[n].as_named_ref().is_some())
            .expect("ref node")
            .index();
        let lag_ix = child
            .node_indices()
            .find(|&n| child[n].to_node_dsp().is_some())
            .expect("lag node")
            .index();

        let flat = gantz_plyphon::flatten_from_registry(parent, &export.registry).expect("flatten");
        let derived = gantz_plyphon::derive_synthdef(&flat, 1, "test").expect("derive");
        let binding = derived
            .params
            .iter()
            .find(|b| b.node_path == [ref_ix, lag_ix])
            .expect("a param binding keyed by the lag's nested path");

        // The binding's path reaches the nested lag's live param state in a VM
        // compiled from the same (un-flattened) graph.
        let builtins = crate::builtin::Builtins::new();
        let reg_ref = gantz_egui::RegistryRef::new(&export.registry, &builtins, &export.demos);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_ref.node(ca);
        let config = gantz_core::compile::Config::default();
        let (mut vm, _compiled) =
            gantz_core::vm::init(&get_node, parent, &[], &config).expect("vm init");
        let (value, pending) = gantz_plyphon::param::drain_param(&mut vm, &binding.node_path)
            .expect("nested lag param state");
        assert_eq!(value, f64::from(gantz_plyphon::Lag::DEFAULT_DUR));
        assert!(pending.is_empty());
    }

    /// `~unpack`'s placeholder expr honours the multi-output contract for any
    /// `count`: a single value for one output, a list of values otherwise. A
    /// wrong shape (e.g. `(list 0)` for count 1) fails `vm::init`'s compile.
    #[test]
    fn unpack_expr_is_steel_inert_for_any_count() {
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;
        type G = Graph<Box<dyn Node>>;

        for count in [1usize, 2, 3] {
            let mut unpack = gantz_plyphon::Unpack::default();
            unpack.set_count(count);
            let mut g: G = Graph::default();
            let s = g.add_node(Box::new(gantz_plyphon::SinOsc::default()) as Box<dyn Node>);
            let up = g.add_node(Box::new(unpack) as Box<dyn Node>);
            let insp = g.add_node(Box::new(gantz_egui::node::Inspect::default()) as Box<dyn Node>);
            g.add_edge(s, up, Edge::new(0.into(), 0.into()));
            // An edge off the last output forces the expr's output shape through
            // the lowerer.
            g.add_edge(up, insp, Edge::new(((count - 1) as u16).into(), 0.into()));

            let get_node = |_: &gantz_ca::ContentAddr| -> Option<&dyn gantz_core::Node> { None };
            let config = gantz_core::compile::Config::default();
            let eps = gantz_core::compile::push_pull_entrypoints(&get_node, &g);
            gantz_core::vm::init(&get_node, &g, &eps, &config)
                .unwrap_or_else(|e| panic!("~unpack count {count} must compile: {e:?}"));
        }
    }

    /// A control input on a DSP node: connecting a `number` to `~sinosc`'s freq
    /// socket and pushing the number writes the number's value into `~sinosc`'s VM
    /// state (which the dsp driver then applies via `set_control`). Guards the
    /// ctrl/dsp bridge end to end on the Steel side.
    #[test]
    fn control_input_writes_dsp_node_state() {
        use gantz_core::compile::{EvalKind, entry_fn_name, push_pull_entrypoints};
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;
        use gantz_core::steel::SteelVal;
        type G = Graph<Box<dyn Node>>;

        // number (a push source) -> ~sinosc.freq (control input at index 0).
        let mut g: G = Graph::default();
        let num = g.add_node(Box::new(gantz_std::Number::default()) as Box<dyn Node>);
        let sine = g.add_node(Box::new(gantz_plyphon::SinOsc::default()) as Box<dyn Node>);
        g.add_edge(num, sine, Edge::new(0.into(), 0.into()));

        let get_node = |_: &gantz_ca::ContentAddr| -> Option<&dyn gantz_core::Node> { None };
        let config = gantz_core::compile::Config::default();
        let eps = push_pull_entrypoints(&get_node, &g);
        let (mut vm, _compiled) =
            gantz_core::vm::init(&get_node, &g, &eps, &config).expect("compile number -> ~sinosc");

        // Stamp the firing time the queued control update should carry, set the
        // number's value, then fire its push entrypoint.
        vm.update_value(gantz_core::ARGS, gantz_core::args::time(1.25));
        gantz_core::node::state::update_value(&mut vm, &[num.index()], SteelVal::NumV(440.0))
            .expect("set number state");
        let ep = eps
            .iter()
            .find(|ep| {
                ep.0.iter()
                    .any(|s| s.kind == EvalKind::Push && s.path == [num.index()])
            })
            .expect("number push entrypoint");
        vm.call_function_by_name_with_args(&entry_fn_name(&ep.id()), vec![])
            .expect("push number");

        // A non-draining peek reports the queued-update count (the state-row summary
        // the inspector shows) - it must NOT drain the queue.
        let queued = gantz_core::node::state::extract_value(&vm, &[sine.index()])
            .expect("extract ~sinosc state")
            .expect("~sinosc state present");
        assert_eq!(
            gantz_plyphon::param::pending_len(&queued),
            1,
            "pending_len must count the queued update without draining",
        );

        // The control value landed in ~sinosc's freq param: the current value is
        // updated, and the timestamped update is queued for the dsp driver.
        let (value, pending) = gantz_plyphon::param::drain_param(&mut vm, &[sine.index()])
            .expect("~sinosc param state present");
        assert_eq!(value, 440.0, "control input must update the param value");
        assert_eq!(
            pending,
            vec![(1.25, 440.0)],
            "control input must queue (time, value) for sample-accurate scheduling",
        );
    }

    /// `~scopeout`'s control side: firing its trigger input outputs the per-channel
    /// ring-buffer state (which the dsp driver fills) as a list of rings on
    /// output 0, and the channel count - the number of rings - on output 1. Here
    /// the rings are seeded directly (standing in for the driver's `push_ring`), a
    /// `number` pushes the trigger, and the outputs land downstream in `inspect`
    /// nodes. Guards the dsp->control read-out path + the two-output `branches`
    /// contract on the Steel side.
    #[test]
    fn scopeout_trigger_outputs_rings_and_channels() {
        use gantz_core::compile::push_pull_entrypoints;
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;
        use gantz_core::steel::SteelVal;
        type G = Graph<Box<dyn Node>>;

        // number -> ~scopeout.trigger (input 1, after the dsp input); output 0 ->
        // inspect_samples, output 1 -> inspect_channels.
        let mut g: G = Graph::default();
        let num = g.add_node(Box::new(gantz_std::Number::default()) as Box<dyn Node>);
        let tap = g.add_node(Box::new(gantz_plyphon::ScopeOut::default()) as Box<dyn Node>);
        let samples = g.add_node(Box::new(gantz_egui::node::Inspect::default()) as Box<dyn Node>);
        let chans = g.add_node(Box::new(gantz_egui::node::Inspect::default()) as Box<dyn Node>);
        g.add_edge(num, tap, Edge::new(0.into(), 1.into()));
        g.add_edge(tap, samples, Edge::new(0.into(), 0.into()));
        g.add_edge(tap, chans, Edge::new(1.into(), 0.into()));

        let get_node = |_: &gantz_ca::ContentAddr| -> Option<&dyn gantz_core::Node> { None };
        let config = gantz_core::compile::Config::default();
        let eps = push_pull_entrypoints(&get_node, &g);
        let (mut vm, _compiled) = gantz_core::vm::init(&get_node, &g, &eps, &config)
            .expect("compile number -> ~scopeout");

        let ring = |vals: &[f64]| -> SteelVal {
            SteelVal::ListV(vals.iter().map(|&v| SteelVal::NumV(v)).collect())
        };
        let channel_count = |vm: &gantz_core::steel::steel_vm::engine::Engine| -> f64 {
            let ch = gantz_core::node::state::extract_value(vm, &[chans.index()])
                .expect("extract channel inspect")
                .expect("channel inspect present");
            match ch {
                SteelVal::IntV(i) => i as f64,
                SteelVal::NumV(f) => f,
                other => panic!("output 1 must be a number, got {other:?}"),
            }
        };

        // Seed the tap with one ring of known samples (as the dsp driver would),
        // then fire the number's push entrypoint: it triggers the tap, which fires
        // both outputs (branch 0) into the inspect nodes.
        let rings = SteelVal::ListV(vec![ring(&[1.0, 2.0, 3.0])].into_iter().collect());
        gantz_core::node::state::update_value(&mut vm, &[tap.index()], rings.clone())
            .expect("seed rings");
        fire_push(&mut vm, &eps, num.index());

        // Output 0: the list of rings, unchanged.
        let got = gantz_core::node::state::extract_value(&vm, &[samples.index()])
            .expect("extract samples inspect")
            .expect("samples inspect present");
        assert_eq!(got, rings, "output 0 must be the per-channel rings");

        // Output 1: the channel count is the number of rings.
        assert_eq!(channel_count(&vm), 1.0, "one ring -> channel count 1");

        // A second, wider write (a stereo tap after a rewire): the count follows.
        let rings = SteelVal::ListV(
            vec![ring(&[1.0, 2.0]), ring(&[-1.0, -2.0])]
                .into_iter()
                .collect(),
        );
        gantz_core::node::state::update_value(&mut vm, &[tap.index()], rings)
            .expect("seed stereo rings");
        fire_push(&mut vm, &eps, num.index());
        assert_eq!(channel_count(&vm), 2.0, "two rings -> channel count 2");
    }

    /// A push arriving through `~scopeout`'s *dsp* input (not the control trigger)
    /// must NOT surface the buffer - the node's `branches` gates the outlets on the
    /// trigger, so a plot downstream does not update on an inert dsp-edge push.
    #[test]
    fn scopeout_suppresses_output_without_trigger() {
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;
        use gantz_core::steel::SteelVal;
        type G = Graph<Box<dyn Node>>;

        // number -> ~scopeout.dsp (input 0); output 0 -> inspect. Firing the number
        // pushes the dsp input, leaving the trigger (input 1) inactive.
        let mut g: G = Graph::default();
        let num = g.add_node(Box::new(gantz_std::Number::default()) as Box<dyn Node>);
        let tap = g.add_node(Box::new(gantz_plyphon::ScopeOut::default()) as Box<dyn Node>);
        let inspect = g.add_node(Box::new(gantz_egui::node::Inspect::default()) as Box<dyn Node>);
        g.add_edge(num, tap, Edge::new(0.into(), 0.into()));
        g.add_edge(tap, inspect, Edge::new(0.into(), 0.into()));

        let get_node = |_: &gantz_ca::ContentAddr| -> Option<&dyn gantz_core::Node> { None };
        let config = gantz_core::compile::Config::default();
        let eps = gantz_core::compile::push_pull_entrypoints(&get_node, &g);
        let (mut vm, _compiled) = gantz_core::vm::init(&get_node, &g, &eps, &config)
            .expect("compile number -> ~scopeout dsp");

        let ring: SteelVal = SteelVal::ListV(
            vec![SteelVal::NumV(1.0), SteelVal::NumV(2.0)]
                .into_iter()
                .collect(),
        );
        let rings: SteelVal = SteelVal::ListV(vec![ring].into_iter().collect());
        gantz_core::node::state::update_value(&mut vm, &[tap.index()], rings).expect("seed rings");

        fire_push(&mut vm, &eps, num.index());

        // The inspect node was never fed (output suppressed): its state stays the
        // initial `Void`, not the ring list.
        let got = gantz_core::node::state::extract_value(&vm, &[inspect.index()])
            .expect("extract inspect state")
            .expect("inspect state present");
        assert!(
            matches!(got, SteelVal::Void),
            "a dsp-only push must not surface the buffer, got {got:?}",
        );
    }

    /// Lowering a hand-authored `mul` (declared in base.gantz's index order)
    /// must reproduce base.gantz's `mul` `GraphAddr`, proving verbatim `src`
    /// capture, declaration-order indexing, and the load path all agree with the
    /// content-addressed registry. The expected address is recomputed from
    /// base.gantz's own graph rather than its (possibly stale) stored key.
    #[test]
    fn lower_mul_matches_base_graph_addr() {
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let base: gantz_egui::export::Export<G> =
            gantz_egui::export::parse_export(gantz_base::BYTES).expect("parse base");
        let base_head = gantz_ca::Head::Branch("mul".to_string());
        let base_graph = base
            .registry
            .head_graph(&base_head)
            .expect("base mul graph");
        let base_addr = gantz_ca::ContentAddr::from(gantz_ca::graph_addr(base_graph)).to_string();

        let text = "\
(graph mul
  (m (expr (* $l $r)))
  (l (inlet \"number\" \"left operand\")) (r (inlet \"number\" \"right operand\")) (out (outlet \"number\" \"product\"))
  (-> l (m 0)) (-> r (m 1)) (-> m out))";
        let mine: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(text, Duration::from_secs(0)).expect("lower");
        let head = gantz_ca::Head::Branch("mul".to_string());
        let graph = mine.registry.head_graph(&head).expect("mul graph");
        let my_addr = gantz_ca::ContentAddr::from(gantz_ca::graph_addr(graph)).to_string();

        assert_eq!(my_addr, base_addr, "lowered mul graph addr must match base");
    }

    /// Round-tripping a consistent export (text -> Export -> text -> Export)
    /// must preserve every name, commit address and graph address. Exercises a
    /// cross-graph `ref` and the `(commits ...)`/`(names ...)` tables.
    #[test]
    fn text_roundtrip_preserves_addrs() {
        use std::collections::BTreeSet;
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let now = Duration::from_secs(1_000_000);
        let text1 = "\
(graph mul
  (m (expr (* $l $r)))
  (l inlet) (r inlet) (out outlet)
  (-> l (m 0)) (-> r (m 1)) (-> m out))

(graph use-mul
  (a inlet) (b inlet) (out outlet)
  (mref (ref mul))
  (-> a (mref 0)) (-> b (mref 1)) (-> mref out))";

        let export1: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(text1, now).expect("from_str 1");
        let text2 = gantz_egui::format::to_string(&export1).expect("to_string");
        let export2: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(&text2, Duration::from_secs(7)).expect("from_str 2");

        let names1: BTreeSet<_> = export1.registry.names().keys().cloned().collect();
        let names2: BTreeSet<_> = export2.registry.names().keys().cloned().collect();
        assert_eq!(names1, names2, "names must match\n--- text2 ---\n{text2}");

        for (name, &head1) in export1.registry.names() {
            let head2 = *export2.registry.names().get(name).expect("name present");
            assert_eq!(
                head1, head2,
                "commit addr for `{name}`\n--- text2 ---\n{text2}"
            );
            let g1 = export1.registry.commit_graph_ref(&head1).expect("g1");
            let g2 = export2.registry.commit_graph_ref(&head2).expect("g2");
            assert_eq!(
                gantz_ca::graph_addr(g1),
                gantz_ca::graph_addr(g2),
                "graph addr for `{name}`",
            );
        }
    }

    /// base.gantz (now consistent `.gantz` text) loads, re-serializes and
    /// reloads, preserving its names and the head commit address exactly (no
    /// healing needed - it is internally consistent).
    #[test]
    fn base_gantz_loads_and_reserializes() {
        use std::collections::BTreeSet;
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let base: gantz_egui::export::Export<G> =
            gantz_egui::export::parse_export(gantz_base::BYTES).expect("parse base");
        let text = gantz_egui::format::to_string(&base).expect("to_string");
        let back: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(&text, Duration::from_secs(0)).expect("from_str");

        let base_names: BTreeSet<_> = base.registry.names().keys().cloned().collect();
        let back_names: BTreeSet<_> = back.registry.names().keys().cloned().collect();
        assert_eq!(
            base_names, back_names,
            "names preserved\n--- text ---\n{text}"
        );

        // base.gantz is consistent: addresses survive the round-trip exactly.
        for (name, &head) in base.registry.names() {
            assert_eq!(
                Some(&head),
                back.registry.names().get(name),
                "commit addr for `{name}` preserved",
            );
        }
    }

    /// Nested graphs are now ordinary named graphs referenced by `(ref ...)`,
    /// so a parent referencing a `<parent>:<n>` child round-trips: both graph
    /// addresses are preserved through text -> Export -> text -> Export.
    #[test]
    fn nested_graph_roundtrips() {
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let now = Duration::from_secs(42);
        let text1 = "\
(graph env:1
  (i inlet) (o outlet)
  (e (expr (+ $x 1)))
  (-> i (e 0)) (-> e o))

(graph env
  (in inlet) (out outlet)
  (sub (ref env:1))
  (-> in (sub 0)) (-> sub out))";
        let e1: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(text1, now).expect("from_str 1");
        let text2 = gantz_egui::format::to_string(&e1).expect("to_string");
        let e2: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(&text2, now).expect("from_str 2");

        for name in ["env", "env:1"] {
            let head = gantz_ca::Head::Branch(name.to_string());
            let g1 = e1.registry.head_graph(&head).expect("g1");
            let g2 = e2.registry.head_graph(&head).expect("g2");
            assert_eq!(
                gantz_ca::graph_addr(g1),
                gantz_ca::graph_addr(g2),
                "graph addr for `{name}` must survive round-trip\n--- text2 ---\n{text2}",
            );
        }
    }

    /// The serializer's output is reader-valid Steel: Steel's own parser accepts
    /// every form. This is the property the whole format design rests on.
    #[test]
    fn output_is_valid_steel() {
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let text1 = "\
(graph g
  (n (number))
  (s (expr (values $x (* $x 2)) #:out 2))
  (b (branch (if $v (list 0 0) (list 1 0)) \"10\" \"01\"))
  (c (comment \"hello world\" 16 2))
  (l (log warn))
  (-> n (s 0)) (-> (s 1) (b 0)))";
        let export: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(text1, Duration::from_secs(0)).expect("from_str");
        let out = gantz_egui::format::to_string(&export).expect("to_string");
        steel::parser::parser::Parser::parse(&out)
            .unwrap_or_else(|e| panic!("output is not valid Steel: {e}\n--- output ---\n{out}"));
    }

    /// A `tick!` node compiles to valid, runnable Steel. `base.gantz` doesn't use
    /// `tick!`, so this is the only coverage of its constant-duration expr, its
    /// stateful accumulator slot, and the per-node push entrypoint registered by
    /// `tick_bang::entrypoints` (which `push_pull_entrypoints` does NOT discover,
    /// since `tick!` is driven externally rather than via `Node::push_eval`).
    #[test]
    fn tick_node_compiles() {
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let text = "\
(graph g
  (t (tick-bang #:rate 2))
  (l (log warn))
  (-> t (l 0)))";
        let export: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(text, Duration::from_secs(0)).expect("from_str");
        let head = gantz_ca::Head::Branch("g".into());
        let graph = export.registry.head_graph(&head).expect("g graph");

        let builtins = crate::builtin::Builtins::new();
        let reg_ref = gantz_egui::RegistryRef::new(&export.registry, &builtins, &export.demos);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_ref.node(ca);

        let entrypoints = bevy_gantz_egui::node::tick_bang::entrypoints(&get_node, graph);
        assert_eq!(
            entrypoints.len(),
            1,
            "tick! must register exactly one push entrypoint",
        );

        for config in [
            gantz_core::compile::Config::default(),
            gantz_core::compile::Config {
                validate_ir: true,
                emit_all_node_fns: true,
            },
        ] {
            gantz_core::vm::init(&get_node, graph, &entrypoints, &config).unwrap_or_else(|e| {
                panic!(
                    "tick! graph failed to compile:\n{}",
                    gantz_core::vm::error_chain(&e),
                )
            });
        }
    }

    /// Importing a commit whose parent is not present in the file records that
    /// commit as a root (the parent is cleared, with a warning).
    #[test]
    fn import_clears_absent_parent() {
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let text = "\
(graph g (e (expr 1)))
(commits (\"abcd1234\" (time 5 0) (parent \"deadbeef\") (graph g)))
(names (gname \"abcd1234\"))";
        let export: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(text, Duration::from_secs(0)).expect("import");
        let commit = export.registry.named_commit("gname").expect("commit");
        assert_eq!(commit.parent, None, "absent parent must be cleared to None");
    }

    /// The Export-level format (gantz_egui over gantz_format) round-trips
    /// `(layout ...)` view state: node positions and the camera survive
    /// text -> Export -> text -> Export.
    #[test]
    fn layout_roundtrips() {
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let now = Duration::from_secs(5);
        let text1 = "\
(graph mul
  (m (expr (* $l $r)))
  (l inlet) (r inlet) (out outlet)
  (-> l (m 0)) (-> r (m 1)) (-> m out))

(layout mul
  (m -10 20) (l 3.5 -4.5)
  (camera 25 -15 1.5))";

        let e1: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(text1, now).expect("from_str 1");
        let head = *e1.registry.names().get("mul").expect("mul name");
        let view = e1.views.get(&head).expect("view");
        // `m` is node index 0, `l` is 1.
        assert_eq!(
            view.layout.get(&egui_graph::NodeId(0)).map(|p| (p.x, p.y)),
            Some((-10.0, 20.0))
        );
        assert_eq!(
            view.layout.get(&egui_graph::NodeId(1)).map(|p| (p.x, p.y)),
            Some((3.5, -4.5))
        );
        assert_eq!((view.camera.center.x, view.camera.center.y), (25.0, -15.0));
        assert_eq!(view.camera.zoom, 1.5);

        let text2 = gantz_egui::format::to_string(&e1).expect("to_string");
        let e2: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(&text2, now).expect("from_str 2");
        let head2 = *e2.registry.names().get("mul").expect("mul name 2");
        let view2 = e2.views.get(&head2).expect("view 2");
        assert_eq!(view.layout.len(), view2.layout.len());
        assert_eq!(
            view2.layout.get(&egui_graph::NodeId(0)).map(|p| (p.x, p.y)),
            Some((-10.0, 20.0))
        );
        assert_eq!(view2.camera, view.camera);
    }

    /// The legacy `(scene min-x min-y max-x max-y)` view form (pre-camera) still
    /// parses: it maps to a camera centred on the rect at the default zoom.
    #[test]
    fn legacy_scene_form_parses_to_camera() {
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let now = Duration::from_secs(5);
        let text = "\
(graph mul
  (m (expr (* $l $r)))
  (l inlet) (r inlet) (out outlet)
  (-> l (m 0)) (-> r (m 1)) (-> m out))

(layout mul
  (m -10 20)
  (scene -50 -50 100 100))";

        let e: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(text, now).expect("from_str");
        let head = *e.registry.names().get("mul").expect("mul name");
        let view = e.views.get(&head).expect("view");
        // Centre of (-50,-50)..(100,100), default zoom.
        assert_eq!((view.camera.center.x, view.camera.center.y), (25.0, 25.0));
        assert_eq!(view.camera.zoom, 1.0);
    }

    /// A clipboard payload round-trips through the `.gantz` text format: the
    /// copied subgraph, its node positions and edges survive copy -> text ->
    /// paste, and the serialized payload is reader-valid Steel.
    #[test]
    fn clipboard_round_trips_through_gantz_text() {
        use bevy_egui::egui;
        use gantz_egui::export;
        use std::collections::{HashMap, HashSet};
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        fn node(tag: &str) -> Box<dyn Node> {
            gantz_format::from_datum(gantz_format::Datum::Map(vec![(
                "type".to_string(),
                gantz_format::Datum::Str(tag.to_string()),
            )]))
            .expect("node")
        }

        let mut graph: G = G::default();
        let a = graph.add_node(node("Identity"));
        let b = graph.add_node(node("Identity"));
        graph.add_edge(a, b, gantz_core::Edge::new(0.into(), 0.into()));

        let registry = gantz_ca::Registry::<G>::default();
        let mut layout = egui_graph::Layout::default();
        layout.insert(egui_graph::NodeId(0), egui::pos2(1.0, 2.0));
        layout.insert(egui_graph::NodeId(1), egui::pos2(3.0, 4.0));
        let selected: HashSet<gantz_core::node::graph::NodeIx> = [a, b].into_iter().collect();

        let copied = export::copy(&registry, &HashMap::new(), &graph, &selected, &layout);
        let text = export::copied_to_string(&copied).expect("copied to text");
        // The clipboard payload is itself reader-valid `.gantz` text.
        steel::parser::parser::Parser::parse(&text)
            .unwrap_or_else(|e| panic!("clipboard text is not valid Steel: {e}\n{text}"));

        let back: export::Copied<Box<dyn Node>> =
            export::copied_from_str(&text).expect("copied from text");
        assert_eq!(back.graph.node_count(), 2);
        assert_eq!(back.graph.edge_count(), 1);
        assert_eq!(
            back.positions
                .get(&egui_graph::NodeId(0))
                .map(|p| (p.x, p.y)),
            Some((1.0, 2.0)),
        );
        assert_eq!(
            back.positions
                .get(&egui_graph::NodeId(1))
                .map(|p| (p.x, p.y)),
            Some((3.0, 4.0)),
        );
    }

    /// Editing a nested child commits it to a new address; [`sync::resync`] must
    /// then propagate that up to its parent, recommitting the parent so its
    /// `NamedRef` references the child's new commit.
    #[test]
    fn resync_propagates_child_edit_to_parent() {
        use gantz_core::node::{Identity, Ref};
        use gantz_egui::node::NamedRef;
        use std::any::Any;
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let ts = Duration::from_secs(0);
        let mut registry = gantz_ca::Registry::<G>::default();

        // Child "p:1": a single node.
        let mut child = G::default();
        child.add_node(Box::new(Identity) as Box<dyn Node>);
        let child_old =
            registry.commit_graph_to_name(ts, gantz_ca::graph_addr(&child), || child, "p:1");

        // Parent "p": a sync-enabled NamedRef to "p:1".
        let mut parent = G::default();
        parent.add_node(Box::new(NamedRef::with_sync(
            "p:1".to_string(),
            Ref::new(child_old.into()),
        )) as Box<dyn Node>);
        let parent_old =
            registry.commit_graph_to_name(ts, gantz_ca::graph_addr(&parent), || parent, "p");

        // Edit the child: commit a different graph under "p:1".
        let mut child2 = G::default();
        child2.add_node(Box::new(Identity) as Box<dyn Node>);
        child2.add_node(Box::new(Identity) as Box<dyn Node>);
        let child_new =
            registry.commit_graph_to_name(ts, gantz_ca::graph_addr(&child2), || child2, "p:1");
        assert_ne!(child_old, child_new);

        // Resync: the parent must follow the child's new commit.
        let moves = gantz_egui::sync::resync(&mut registry, ts);
        assert!(
            moves.iter().any(|m| m.name == "p"),
            "parent should have recommitted: {moves:?}"
        );

        let parent_new = *registry.names().get("p").unwrap();
        assert_ne!(parent_old, parent_new, "parent commit must change");
        let p_graph = registry.commit_graph_ref(&parent_new).unwrap();
        let points_at_new_child = p_graph.node_weights().any(|n| {
            ((&**n) as &dyn Any)
                .downcast_ref::<NamedRef>()
                .map(|nr| nr.content_addr() == child_new.into())
                .unwrap_or(false)
        });
        assert!(
            points_at_new_child,
            "parent's NamedRef must reference the child's new commit"
        );
    }

    /// Forking a graph with a nested child gives the fork its *own* child:
    /// [`sync::fork_nested`] copies the `parent:*` subtree to the fork and
    /// rewrites its references, leaving the original's children untouched.
    #[test]
    fn fork_nested_gives_independent_children() {
        use gantz_core::node::{Identity, Ref};
        use gantz_egui::node::NamedRef;
        use std::any::Any;
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let ts = Duration::from_secs(0);
        let mut registry = gantz_ca::Registry::<G>::default();

        // Child "A:1" and parent "A" referencing it.
        let mut child = G::default();
        child.add_node(Box::new(Identity) as Box<dyn Node>);
        let child_ca =
            registry.commit_graph_to_name(ts, gantz_ca::graph_addr(&child), || child, "A:1");
        let mut parent = G::default();
        parent.add_node(Box::new(NamedRef::with_sync(
            "A:1".to_string(),
            Ref::new(child_ca.into()),
        )) as Box<dyn Node>);
        registry.commit_graph_to_name(ts, gantz_ca::graph_addr(&parent), || parent, "A");

        // Fork "A" -> "B": a fresh commit over A's graph (as `on_branch_head` does),
        // so "B" initially references A's child "A:1".
        let a_commit = *registry.names().get("A").unwrap();
        let a_graph = registry.commits()[&a_commit].graph;
        let b_commit = registry.commit_graph(ts, Some(a_commit), a_graph, || unreachable!());
        registry.insert_name("B".to_string(), b_commit);

        // Cascade: give "B" its own nested child "B:1".
        let moves = gantz_egui::sync::fork_nested(&mut registry, ts, "A", "B");
        assert!(
            moves.iter().any(|m| m.name == "B:1"),
            "B:1 should be created: {moves:?}"
        );
        assert!(
            moves.iter().any(|m| m.name == "B"),
            "B's root should be rewritten: {moves:?}"
        );

        // B references its own child B:1; A:1 is untouched.
        let b1: gantz_ca::ContentAddr = (*registry.names().get("B:1").unwrap()).into();
        let b_new = *registry.names().get("B").unwrap();
        let b_graph = registry.commit_graph_ref(&b_new).unwrap();
        let refs_b1 = b_graph.node_weights().any(|n| {
            ((&**n) as &dyn Any)
                .downcast_ref::<NamedRef>()
                .map(|nr| nr.name() == "B:1" && nr.content_addr() == b1)
                .unwrap_or(false)
        });
        assert!(refs_b1, "the fork's root must reference its own child B:1");
        assert!(
            registry.names().contains_key("A:1"),
            "the original child A:1 must remain"
        );
    }

    /// Copying a node that references a nested graph and pasting it must keep the
    /// reference. The format preserves only the head commit per graph, so an
    /// *edited* nested graph's head address heals on paste (its parent is
    /// dropped); the `NamedRef` must still resolve - by name - rather than
    /// vanish.
    #[test]
    fn clipboard_round_trips_nested_ref() {
        use gantz_core::node::{Identity, Ref};
        use gantz_egui::export;
        use gantz_egui::node::NamedRef;
        use std::any::Any;
        use std::collections::{HashMap, HashSet};
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let mut registry = gantz_ca::Registry::<G>::default();

        // Nested graph "A:1", committed twice so its head commit has a parent
        // (the format does not preserve it, so the head address heals on paste).
        let mut v1 = G::default();
        v1.add_node(Box::new(Identity) as Box<dyn Node>);
        registry.commit_graph_to_name(
            Duration::from_secs(1),
            gantz_ca::graph_addr(&v1),
            || v1,
            "A:1",
        );
        let mut v2 = G::default();
        v2.add_node(Box::new(Identity) as Box<dyn Node>);
        v2.add_node(Box::new(Identity) as Box<dyn Node>);
        let head = registry.commit_graph_to_name(
            Duration::from_secs(2),
            gantz_ca::graph_addr(&v2),
            || v2,
            "A:1",
        );

        // A graph holding a synced NamedRef to "A:1".
        let mut graph: G = G::default();
        let nref = graph.add_node(Box::new(NamedRef::with_sync(
            "A:1".to_string(),
            Ref::new(head.into()),
        )) as Box<dyn Node>);
        let selected: HashSet<_> = [nref].into_iter().collect();

        // Copy -> clipboard text -> paste.
        let copied = export::copy(
            &registry,
            &HashMap::new(),
            &graph,
            &selected,
            &egui_graph::Layout::default(),
        );
        let text = export::copied_to_string(&copied).expect("copied to text");
        let back: export::Copied<Box<dyn Node>> =
            export::copied_from_str(&text).expect("copied from text");

        assert_eq!(back.graph.node_count(), 1, "the nested-ref node must paste");
        let kept = back.graph.node_weights().any(|n| {
            ((&**n) as &dyn Any)
                .downcast_ref::<NamedRef>()
                .map(|nr| nr.name() == "A:1")
                .unwrap_or(false)
        });
        assert!(kept, "the pasted node must still be a NamedRef to A:1");
    }

    /// Renaming a nested graph to a root name promotes it: every reference in
    /// the parent (there may be several instances, each with its own state) is
    /// repointed to the new name, and the orphaned nested name is dropped.
    #[test]
    fn promote_nested_repoints_all_parent_instances() {
        use gantz_core::node::{Identity, Ref};
        use gantz_egui::node::NamedRef;
        use std::any::Any;
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let ts = Duration::from_secs(0);
        let mut registry = gantz_ca::Registry::<G>::default();

        // Nested child "A:1".
        let mut child = G::default();
        child.add_node(Box::new(Identity) as Box<dyn Node>);
        let a1 = registry.commit_graph_to_name(ts, gantz_ca::graph_addr(&child), || child, "A:1");

        // Parent "A" with THREE instances of the nested graph.
        let mut parent = G::default();
        for _ in 0..3 {
            parent.add_node(
                Box::new(NamedRef::with_sync("A:1".to_string(), Ref::new(a1.into())))
                    as Box<dyn Node>,
            );
        }
        registry.commit_graph_to_name(ts, gantz_ca::graph_addr(&parent), || parent, "A");

        // Simulate "rename A:1 -> B": a root "B" copy of A:1's graph (as the
        // fork does), then promote.
        let a1_graph = registry.commits()[&a1].graph;
        let b = registry.commit_graph(ts, Some(a1), a1_graph, || unreachable!());
        registry.insert_name("B".to_string(), b);
        let moves = gantz_egui::sync::promote_nested(&mut registry, ts, "A:1", "B");

        assert!(
            moves.iter().any(|m| m.name == "A"),
            "parent A must recommit"
        );
        assert!(
            !registry.names().contains_key("A:1"),
            "the orphaned nested name must be dropped"
        );

        // All three parent references now point at "B".
        let a_commit = *registry.names().get("A").unwrap();
        let a_graph = registry.commit_graph_ref(&a_commit).unwrap();
        let to_b = a_graph
            .node_weights()
            .filter(|n| {
                ((&***n) as &dyn Any)
                    .downcast_ref::<NamedRef>()
                    .map(|nr| nr.name() == "B")
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(to_b, 3, "all instances must be repointed to B");
    }

    /// Every named graph shipped in `base.gantz` - all primitives, the `demo-*`
    /// graphs, and the unconnected `demo-all` catalog - must compile to a valid
    /// Steel module under the same `Engine::new_base()` the runtime uses. This
    /// guards against authoring a graph that relies on a prelude-only binding
    /// (`map`, `and`, `cond`, `min`, ...) or otherwise emits invalid Steel,
    /// which the base engine (no prelude) rejects. Mirrors the live compile path
    /// in `bevy_gantz::vm` (`push_pull_entrypoints` + `vm::init`).
    ///
    /// Compiled under both configs: the default (node fns emitted on demand) and
    /// `emit_all_node_fns` (the app's "inspect every node's code" toggle, which
    /// emits each node's all-connected variant - the case that exercises the
    /// `demo-all` catalog's otherwise-unconnected `ref` nodes).
    #[test]
    fn base_graphs_all_compile() {
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let base: gantz_egui::export::Export<G> =
            gantz_egui::export::parse_export(gantz_base::BYTES).expect("parse base");
        let builtins = crate::builtin::Builtins::new();
        let reg_ref = gantz_egui::RegistryRef::new(&base.registry, &builtins, &base.demos);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_ref.node(ca);
        let configs = [
            gantz_core::compile::Config::default(),
            gantz_core::compile::Config {
                validate_ir: true,
                emit_all_node_fns: true,
            },
        ];

        assert!(
            !base.registry.names().is_empty(),
            "base.gantz registered no named graphs",
        );
        for name in base.registry.names().keys() {
            let head = gantz_ca::Head::Branch(name.clone());
            let graph = base
                .registry
                .head_graph(&head)
                .unwrap_or_else(|| panic!("`{name}` has no head graph"));
            let entrypoints = gantz_core::compile::push_pull_entrypoints(&get_node, graph);
            for config in &configs {
                gantz_core::vm::init(&get_node, graph, &entrypoints, config).unwrap_or_else(|e| {
                    panic!(
                        "base graph `{name}` failed to compile (emit_all_node_fns={}):\n{}",
                        config.emit_all_node_fns,
                        gantz_core::vm::error_chain(&e),
                    )
                });
            }
        }
    }

    /// Every `ref` in `base.gantz` is auto-syncing, so the demos track the latest
    /// primitive commits automatically. Verified through a load + re-serialize
    /// round-trip (the `update-base` export path): a loaded `NamedRef` whose
    /// `sync` was set re-emits `#:sync`, so the re-serialized text carries one
    /// `#:sync` per `ref`.
    #[test]
    fn base_refs_are_synced() {
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;
        let base: gantz_egui::export::Export<G> =
            gantz_egui::export::parse_export(gantz_base::BYTES).expect("parse base");
        let text = gantz_egui::format::to_string(&base).expect("to_string");
        let refs = text.matches("(ref ").count() + text.matches("(fn-ref ").count();
        let synced = text.matches("#:sync").count();
        assert!(refs > 0, "expected base to contain refs");
        assert_eq!(
            refs, synced,
            "every base ref must auto-sync (#:sync); got {synced}/{refs}\n--- text ---\n{text}",
        );
    }

    /// A `NamedRef`'s `sync` flag is part of its content address. This is what
    /// lets `base_refs_are_synced` hold in practice: toggling `sync` in the
    /// inspector must change the node's address so the edit rides the normal
    /// commit + export pipeline rather than being silently dropped by the
    /// registry's content-addressed dedup. Guards against re-adding
    /// `#[cahash(skip)]` to `NamedRef::sync`.
    #[test]
    fn named_ref_sync_affects_content_addr() {
        use gantz_egui::node::NamedRef;
        let ca = gantz_ca::ContentAddr::from([0u8; 32]);
        let ref_ = gantz_core::node::Ref::new(ca);
        let off = NamedRef::new("x".to_string(), ref_.clone());
        let on = NamedRef::with_sync("x".to_string(), ref_);
        assert_ne!(
            gantz_ca::content_addr(&off),
            gantz_ca::content_addr(&on),
            "toggling `sync` must change the content address, otherwise the \
             toggle can't trigger a commit and won't persist",
        );
    }

    /// Every base-primitive socket carries a hover doc (type + description), and
    /// those docs resolve through a `ref` to the referenced graph's inlet/outlet
    /// markers - exactly the path the GUI uses for a `NamedRef`'s socket tooltip.
    #[test]
    fn base_socket_docs() {
        use gantz_egui::{Registry as _, SocketKind};
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;
        let base: gantz_egui::export::Export<G> =
            gantz_egui::export::parse_export(gantz_base::BYTES).expect("parse base");

        // Completeness: no primitive socket serializes as a bare `inlet`/`outlet`.
        let text = gantz_egui::format::to_string(&base).expect("to_string");
        let bare = text.matches(" inlet)").count() + text.matches(" outlet)").count();
        assert_eq!(
            bare, 0,
            "every base socket must be documented\n--- text ---\n{text}"
        );

        // Resolution: a `ref add` exposes `add`'s socket docs.
        let builtins = crate::builtin::Builtins::new();
        let reg_ref = gantz_egui::RegistryRef::new(&base.registry, &builtins, &base.demos);
        let add = gantz_ca::ContentAddr::from(*base.registry.names().get("add").expect("add"));
        let doc = |kind, ix| reg_ref.socket_doc(&add, kind, ix);

        let l = doc(SocketKind::Input, 0).expect("add input 0 doc");
        assert_eq!(
            (l.ty.as_ref(), l.description.as_deref()),
            ("number", Some("left operand"))
        );
        let out = doc(SocketKind::Output, 0).expect("add output doc");
        assert_eq!(
            (out.ty.as_ref(), out.description.as_deref()),
            ("number", Some("sum"))
        );
    }

    /// End-to-end check of every `demo-*` graph: firing its `bang` must evaluate
    /// all ops without a runtime error *or panic*, with default inputs. The bang
    /// feeds every interactive input, so all of an op's inputs are active in one
    /// push (guarding the "single input active" failure). It also guards two
    /// integer-op gotchas: `number` outputs floats, so `list-ref`/`mod` coerce
    /// via `(exact (round ...))`, and `mod` must stay *total* - Steel's `modulo`
    /// panics (aborts) on a zero divisor, which `mod` avoids by returning the
    /// dividend, so firing `demo-arithmetic` with its default `0`/`0` inputs no
    /// longer crashes the process.
    #[test]
    fn demos_evaluate() {
        use gantz_core::compile::{EvalKind, entry_fn_name, push_pull_entrypoints};
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let base: gantz_egui::export::Export<G> =
            gantz_egui::export::parse_export(gantz_base::BYTES).expect("parse base");
        let builtins = crate::builtin::Builtins::new();
        let reg_ref = gantz_egui::RegistryRef::new(&base.registry, &builtins, &base.demos);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_ref.node(ca);
        let config = gantz_core::compile::Config::default();

        let demos = [
            "demo-arithmetic",
            "demo-comparison",
            "demo-logic",
            "demo-list",
            "demo-predicate",
        ];
        for name in demos {
            let head = gantz_ca::Head::Branch(name.to_string());
            let graph = base
                .registry
                .head_graph(&head)
                .unwrap_or_else(|| panic!("{name} graph"));

            // The single `bang` node drives every pipeline in the demo.
            let go = graph
                .node_indices()
                .find(|&ix| {
                    (&*graph[ix] as &dyn std::any::Any)
                        .downcast_ref::<gantz_std::Bang>()
                        .is_some()
                })
                .map(|ix| ix.index())
                .unwrap_or_else(|| panic!("{name} has a bang"));

            let eps = push_pull_entrypoints(&get_node, graph);
            let (mut vm, _compiled) = gantz_core::vm::init(&get_node, graph, &eps, &config)
                .unwrap_or_else(|e| panic!("init {name}: {}", gantz_core::vm::error_chain(&e)));

            let go_ep = eps
                .iter()
                .find(|ep| {
                    ep.0.iter()
                        .any(|s| s.kind == EvalKind::Push && s.path == [go])
                })
                .unwrap_or_else(|| panic!("{name} bang entrypoint"));
            vm.call_function_by_name_with_args(&entry_fn_name(&go_ep.id()), vec![])
                .unwrap_or_else(|e| panic!("firing {name} bang errored: {e}"));
        }
    }

    /// Resetting a demo re-parses the base and merges the demo's commit subset
    /// back in. Because the base's hand-authored graphs are stamped at a fixed
    /// [`bevy_gantz_egui::base::BASE_TIMESTAMP`], the re-parse reproduces the
    /// primitive commit addresses loaded at startup, so the reset demo's `ref`s
    /// still resolve and it recompiles. (With a wall-clock timestamp the
    /// re-parsed demo would reference fresh primitive commits absent from the
    /// registry, failing with "node has 0 outputs".)
    #[test]
    fn reset_then_reopen_demo_recompiles() {
        use gantz_core::compile::{Config, push_pull_entrypoints};
        use std::collections::HashSet;

        let ts = bevy_gantz_egui::base::BASE_TIMESTAMP;
        let parse = || {
            gantz_egui::export::parse_export_at::<Box<dyn Node>>(gantz_base::BYTES, ts)
                .expect("parse base")
        };

        // Parsing the base at the fixed timestamp is reproducible: every name
        // maps to the same commit both times - what lets a reset agree with the
        // registry loaded at startup.
        let startup = parse();
        let reparse = parse();
        assert_eq!(
            startup.registry.names(),
            reparse.registry.names(),
            "base commit addresses must be reproducible across parses",
        );

        // Simulate `on_reset_base_graph`: re-export the demo's commit subset
        // from a fresh parse and merge it into the startup registry.
        let mut registry = startup.registry;
        let name = "demo-arithmetic";
        let &demo_commit = reparse.registry.names().get(name).expect("demo name");
        let required: HashSet<_> =
            gantz_ca::ancestors(reparse.registry.commits(), demo_commit).collect();
        let mut subset = reparse.registry.export(&required);
        subset.insert_name(name.to_string(), demo_commit);
        registry.merge(subset);

        // Reopen: the reset demo must still compile, i.e. every `ref` resolves.
        let builtins = crate::builtin::Builtins::new();
        let reg_ref = gantz_egui::RegistryRef::new(&registry, &builtins, &startup.demos);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_ref.node(ca);
        let head = gantz_ca::Head::Branch(name.to_string());
        let graph = registry.head_graph(&head).expect("demo graph");
        let eps = push_pull_entrypoints(&get_node, graph);
        gantz_core::vm::init(&get_node, graph, &eps, &Config::default()).unwrap_or_else(|e| {
            panic!(
                "recompile after reset failed: {}",
                gantz_core::vm::error_chain(&e)
            )
        });
    }

    /// The inline-name base export (`format::to_string_named`) names every graph
    /// inline, drops the `(commits ...)`/`(names ...)` tables and the pinned ref
    /// addresses, and is *stable*: re-exporting an unchanged base produces byte
    /// -identical text (no churning addresses), which is the whole point - a
    /// cleaner, hand-editable `base.gantz`.
    #[test]
    fn base_named_export_is_stable() {
        use std::collections::BTreeSet;
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let base: gantz_egui::export::Export<G> =
            gantz_egui::export::parse_export(gantz_base::BYTES).expect("parse base");
        let text = gantz_egui::format::to_string_named(&base).expect("to_string_named");

        // Inline names, no tables, references by name.
        assert!(!text.contains("(commits"), "no commits table:\n{text}");
        assert!(!text.contains("(names"), "no names table:\n{text}");
        assert!(
            text.contains("(graph add\n"),
            "graphs named inline:\n{text}"
        );
        assert!(
            text.contains("(ref add #:sync)"),
            "refs resolve by name, no pinned address:\n{text}",
        );

        // Stable: reload the simplified text and re-serialize - byte-identical.
        let back: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(&text, Duration::from_secs(0)).expect("from_str");
        let text2 = gantz_egui::format::to_string_named(&back).expect("to_string_named 2");
        assert_eq!(text, text2, "inline-name export must be idempotent");

        // Names survive the round-trip.
        let n1: BTreeSet<_> = base.registry.names().keys().cloned().collect();
        let n2: BTreeSet<_> = back.registry.names().keys().cloned().collect();
        assert_eq!(n1, n2, "names preserved");
    }
}
