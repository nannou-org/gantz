/// The `.gantz` keyword sugar carrier for the app's node set: the
/// `gantz_core`, `gantz_std`, `gantz_egui`, `bevy_gantz_egui` and
/// `gantz_plyphon` node sugars composed.
pub struct NodeSet;

impl gantz_format::NodeSugar for NodeSet {
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

/// The value-level codec for the app's node set: typed nodes to/from the
/// registry's erased `NodeData` form, plus the set's `.gantz` sugar.
///
/// This list is the app's wire-format manifest: adding a node type to the
/// app is one line here (the `codec_covers_every_node_set_case` and
/// `node_set_addr_pins` gate tests enforce it).
pub fn codec() -> gantz_egui::node::NodeCodec {
    gantz_egui::ui_node_codec! {
        NodeSet {
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
            gantz_plyphon::Sum,
            gantz_plyphon::Unpack,
            gantz_plyphon::Bus,
            gantz_plyphon::PlayBuf,
        }
    }
}

/// The app's full builtin node set: every domain's builtin specs composed.
pub fn builtins() -> gantz_core::Builtins {
    gantz_core::Builtins::from_specs(
        gantz_core::node::builtins()
            .into_iter()
            .chain(gantz_std::builtins())
            .chain(gantz_egui::builtins())
            .chain(bevy_gantz_egui::builtins())
            .chain(gantz_plyphon::builtins()),
    )
}

#[cfg(test)]
mod tests {
    use gantz_egui::node::DynNode;

    /// The data registry: graphs stored erased.
    type DataReg = gantz_ca::Registry;
    /// The typed cache serving the registry's graphs as the app's node set.
    type Reified = gantz_core::data::ReifiedGraphs<DynNode>;

    fn name(s: &str) -> gantz_ca::Name {
        s.parse().expect("infallible")
    }

    /// The `NamedRef` within an erased app node, if it is one.
    #[allow(clippy::borrowed_box)]
    fn as_named_ref(node: &DynNode) -> Option<&gantz_egui::node::NamedRef> {
        let n: &dyn gantz_core::Node = &**node;
        (n as &dyn std::any::Any).downcast_ref()
    }

    /// Reify the whole registry column into a typed cache through the codec.
    fn reify_all(reg: &DataReg) -> Reified {
        let mut reified = Reified::new();
        let codec = super::codec();
        let errs = reified.ensure_all_with(reg, |nd| codec.reify_ui(nd).map(|inst| inst.node));
        assert!(errs.is_empty(), "{errs:?}");
        reified
    }

    /// The composed builtin palette plus one reified instance per builtin.
    fn builtins_with_instances() -> (gantz_core::Builtins, gantz_egui::node::UiBuiltins) {
        let builtins = super::builtins();
        let (instances, errs) = gantz_egui::node::UiBuiltins::reify(&builtins, &super::codec());
        assert!(errs.is_empty(), "{errs:?}");
        (builtins, instances)
    }

    /// The [`gantz_egui::Env`] over the given borrowed parts.
    fn env<'a>(
        registry: &'a DataReg,
        reified: &'a Reified,
        builtins: &'a (gantz_core::Builtins, gantz_egui::node::UiBuiltins),
        codec: &'a gantz_egui::node::NodeCodec,
    ) -> gantz_egui::Env<'a> {
        gantz_egui::Env {
            registry,
            builtins: &builtins.0,
            codec,
            graphs: reified,
            instances: &builtins.1,
        }
    }

    /// The typed graph at the given head's tip, if reified.
    fn head_graph<'a>(
        reified: &'a Reified,
        reg: &DataReg,
        head: &gantz_ca::Head,
    ) -> Option<&'a gantz_core::node::graph::Graph<DynNode>> {
        reified.get(&reg.head_commit(head)?.graph)
    }

    /// Erase a typed node to its stored data form via its own tag + serde.
    fn erased<T>(node: &T) -> gantz_ca::NodeData
    where
        T: gantz_nodetag::NodeTag + serde::Serialize + gantz_core::Node,
    {
        gantz_core::data::erase_node_typed(node).expect("erase")
    }

    /// A data graph over the given stored node weights (no edges).
    fn data_graph(nodes: impl IntoIterator<Item = gantz_ca::NodeData>) -> gantz_ca::DataGraph {
        let mut g = gantz_ca::DataGraph::default();
        for nd in nodes {
            g.add_node(nd);
        }
        g
    }

    /// Commit `graph` under `name`, returning the new commit and the graph's
    /// address (the registry's identity for the graph).
    fn commit_to_name(
        reg: &mut DataReg,
        ts: std::time::Duration,
        graph: gantz_ca::DataGraph,
        name: &gantz_ca::Name,
    ) -> (gantz_ca::CommitAddr, gantz_ca::GraphAddr) {
        let ga = gantz_ca::graph_addr(&graph);
        let ca = reg.commit_graph_to_name(ts, ga, || graph, name);
        (ca, ga)
    }

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

    /// Gate test for the app's builtin palette: the set composed from the
    /// per-domain `builtins()` lists must match the full expected name set.
    /// A builtin dropped from (or added to) any domain list fails here.
    #[test]
    fn builtins_match_expected_name_set() {
        let expected = vec![
            "apply",
            "bang",
            "branch",
            "comment",
            "delay",
            "expr",
            "fn",
            "id",
            "inlet",
            "inspect",
            "log",
            "number",
            "outlet",
            "plot",
            "tick!",
            "update!",
            "~bus",
            "~lag",
            "~out",
            "~pack",
            "~playbuf",
            "~scopeout",
            "~sinosc",
            "~sum",
            "~unpack",
        ];
        let builtins = super::builtins();
        let names: Vec<_> = builtins.names().collect();
        assert_eq!(names, expected);
    }

    /// Gate test for the builtin data <-> typed instance seam: every composed
    /// builtin's stored `NodeData` reifies through the app codec and
    /// re-erases to the identical `NodeData` (same canonical form, same
    /// content address).
    #[test]
    fn builtins_round_trip_through_codec() {
        let codec = super::codec();
        let builtins = super::builtins();
        for name in builtins.names() {
            let nd = builtins.node_data(name).expect("named builtin");
            let inst = codec
                .reify_ui(nd)
                .unwrap_or_else(|e| panic!("builtin `{name}` failed to reify: {e}"));
            let back = inst
                .erase()
                .unwrap_or_else(|e| panic!("builtin `{name}` failed to re-erase: {e}"));
            assert_eq!(*nd, back, "builtin `{name}`: typed round-trip diverges");
            assert_eq!(nd.content_addr(), back.content_addr());
        }
    }

    /// A `"type"`-tagged node wire datum, as the `.gantz` format node specs
    /// parse to.
    fn node_datum(tag: &str, fields: Vec<(&str, gantz_format::Datum)>) -> gantz_format::Datum {
        use gantz_format::Datum;
        let mut entries = vec![("type".to_string(), Datum::Str(tag.to_string()))];
        entries.extend(fields.into_iter().map(|(k, v)| (k.to_string(), v)));
        Datum::Map(entries)
    }

    /// Known-valid wire datums covering the node set, shared by the
    /// round-trip and erasure gate tests.
    fn node_set_cases() -> Vec<gantz_format::Datum> {
        use gantz_format::Datum;
        vec![
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
            node_datum("Sum", vec![]),
            node_datum("Sum", vec![("count", Datum::U64(4))]),
            node_datum("Unpack", vec![]),
            node_datum("Unpack", vec![("count", Datum::U64(4))]),
            node_datum("Bus", vec![]),
        ]
    }

    /// The stored form of one hand-authored wire datum: split the `"type"`
    /// tag out and round-trip the fields through the codec's typed node,
    /// recomputing the canonical form and the refs/blobs columns (mirrors
    /// the `.gantz` parse path's normalization).
    fn node_data_of(datum: gantz_format::Datum) -> gantz_ca::NodeData {
        let gantz_format::Datum::Map(mut entries) = datum else {
            panic!("node datum is not a map");
        };
        let ix = entries
            .iter()
            .position(|(k, _)| k == "type")
            .expect("node datum has no `type` tag");
        let (_, gantz_format::Datum::Str(tag)) = entries.remove(ix) else {
            panic!("node `type` tag is not a string");
        };
        let nd = gantz_ca::NodeData::new(tag, gantz_format::Datum::Map(entries));
        super::codec()
            .normalize(&nd)
            .unwrap_or_else(|e| panic!("`{}` failed to normalize: {e}", nd.tag))
    }

    /// Every manifest case decodes through the codec: a type authored in the
    /// wire cases but missing from `ui_node_codec!` fails here.
    #[test]
    fn codec_covers_every_node_set_case() {
        let codec = super::codec();
        for value in node_set_cases() {
            let nd = node_data_of(value.clone());
            codec
                .reify_ui(&nd)
                .unwrap_or_else(|e| panic!("tag `{}` missing from the codec: {e}", nd.tag));
        }
    }

    /// The stored instances the erased-representation gate runs over: every
    /// wire case above, plus types without hand-authored cases.
    fn node_set_data() -> Vec<gantz_ca::NodeData> {
        let mut nodes: Vec<gantz_ca::NodeData> =
            node_set_cases().into_iter().map(node_data_of).collect();
        nodes.push(erased(&gantz_std::Log::default()));
        nodes.push(erased(&gantz_core::node::Fn(
            gantz_egui::node::NamedRef::new(
                name("mul"),
                gantz_core::node::Ref::new(gantz_ca::ContentAddr([1; 32])),
            ),
        )));
        nodes.push(erased(&gantz_plyphon::PlayBuf::new(
            gantz_ca::ContentAddr([2; 32]),
            2,
            48_000.0,
        )));
        nodes
    }

    /// Gate test for the registry's erased representation: every node in the
    /// set must erase to canonical `NodeData` whose typed round-trip through
    /// the codec is a fixpoint (a stable content address), with structural
    /// refs matching the graph-level reachability reporting. A node type with
    /// order- or shape-unstable serde fails here.
    #[test]
    fn node_set_erases_canonically() {
        fn no_node(_: &gantz_ca::ContentAddr) -> Option<&'static dyn gantz_core::Node> {
            None
        }

        let codec = super::codec();
        for (i, nd) in node_set_data().into_iter().enumerate() {
            assert!(
                nd.is_canonical(),
                "case {i} (`{}`): non-canonical erasure: {nd:?}",
                nd.tag,
            );
            let inst = codec
                .reify_ui(&nd)
                .unwrap_or_else(|e| panic!("case {i}: reify failed: {e}"));
            let nd2 = inst.erase().expect("re-erase");
            let back = inst.node;
            assert_eq!(
                nd, nd2,
                "case {i} (`{}`): typed round-trip shifts the node's data",
                nd.tag,
            );

            // Structural refs match the graph-level reachability reporting.
            let mut g: gantz_core::node::graph::Graph<DynNode> = Default::default();
            g.add_node(back);
            let out = gantz_core::graph::out_refs(&no_node, &g);
            let refs: Vec<_> = nd
                .refs
                .iter()
                .copied()
                .map(gantz_ca::GraphAddr::from)
                .collect();
            assert_eq!(out.graphs, refs, "case {i} (`{}`): refs parity", nd.tag);
            assert_eq!(out.blobs, nd.blobs, "case {i} (`{}`): blobs parity", nd.tag);
        }
    }

    /// Pin every node type's canonical content address (the first
    /// `node_set_instances` case per tag): erased node addresses are
    /// wire-stability-critical, so any serde change that shifts one - e.g.
    /// emitting a defaulted field, renaming a field, reordering an enum -
    /// fails here loudly and must be a deliberate decision.
    #[test]
    fn node_set_addr_pins() {
        let mut seen = std::collections::BTreeMap::new();
        for nd in node_set_data() {
            seen.entry(nd.tag.clone())
                .or_insert_with(|| nd.content_addr().to_string());
        }
        let actual: Vec<(String, String)> = seen.into_iter().collect();
        let expected: Vec<(String, String)> = [
            (
                "Apply",
                "7efc434b814bf22f86e51c95a525c335b050cefe38a37598dd45bce0f1ebfde4",
            ),
            (
                "Bang",
                "ac2f4e3d47c7b69a188da461568e9c9c013d1458f68a259ad3c64c3e4055c825",
            ),
            (
                "Branch",
                "dfd6ba15af40df9e11f89d8b89e895154ef8dc52249797399b326430c4f2f4e2",
            ),
            (
                "Bus",
                "10ac84d365f318b5116af46dec6c5b400ebcbd3bff1751113096e43e766d791b",
            ),
            (
                "Comment",
                "ee0eb63753a269f48a387c812b4d96a2a8ef78c441c791bc9ea445613d7e514b",
            ),
            (
                "Delay",
                "45b031845977348820dd7e4b21fc53238355d94c9d682f604c1b77f199cc2940",
            ),
            (
                "Expr",
                "95d237d9e0f63806d770c79214c7a7a4f7bf92ad6c850401db772044627b9645",
            ),
            (
                "FnNamedRef",
                "71b52c706fd6459c6cced0cfd3a58035c5e0a10d1862881a37672bd1a8f9366b",
            ),
            (
                "Identity",
                "01266e1286fc7d37d4f8c4c3c1e4e99ceca5837f9094e5f628a096ffb27217b1",
            ),
            (
                "Inlet",
                "e67bf1330241ba58f249f768fd63e701f90eeed94990a113cd494368bd1e2572",
            ),
            (
                "Inspect",
                "f33771875aa2d1e58ba6d0b78508d3fbefb67bb9cea450a58c661f0c1f8c94ad",
            ),
            (
                "Lag",
                "25b2a58df0f09fbfbac24f66e392b3bbc37dac93f6be3a4615c2564ec93bc98b",
            ),
            (
                "Log",
                "e342bc3f0fbb7f89223b045a6083a84de3e099074d969aec9b5a5c9f27169c09",
            ),
            (
                "NamedRef",
                "207f946ce3dfee38b4da6616a775f7b575e4f850b4ee945c8c4d061737314cc0",
            ),
            (
                "Number",
                "d22fb1ac39321f2aefb1ab72947d22f1a1ffecaeaacab0d3b3d42bd0466837d0",
            ),
            (
                "Out",
                "5eaa263391e9171e0a7835c8bf75e64ea88f07e7b77d6ab3f8c6760d38c37be6",
            ),
            (
                "Outlet",
                "ffa3e274559c73dbc70ddf73efafb9dae72e8e3d56a84818d16e201ee543f4b6",
            ),
            (
                "Pack",
                "36a4fded818932c8bbfad7cba2748c3ea369d697398a3c1e506e46f4e10ecb42",
            ),
            (
                "PlayBuf",
                "80f30968c0021ac5a72c7c136c2d946f22768831414028672580cc629649ff99",
            ),
            (
                "Plot",
                "deb280956a42f29de5d9515537c19b57a8ccb1575e2620dd68ab2d66aaae4484",
            ),
            (
                "ScopeOut",
                "f52e55d37ad94f3c6bd37c78f55282f8b8e934c8f26e075cd1afb32a5eee133c",
            ),
            (
                "SinOsc",
                "8855fa0969785b3f32a540a882d4993aec164be346317fa0ad4563e25a36a25e",
            ),
            (
                "Sum",
                "80658975450345544a9d54870972ef3ffe60d100778adbee6113455963254389",
            ),
            (
                "TickBang",
                "5b3327a3738cae95967f9b7969d8ddf07eedf1e681e3e1a45147614fa75a1ea1",
            ),
            (
                "Unpack",
                "ad01b8b16a5f537d6054fd6fdbc3bc03dfa9b818ab3558178b2fcf51b96515d3",
            ),
            (
                "UpdateBang",
                "674ba08a023408bb47aadfd667ffb1bb6866ff4a5bdf0907b3c1620daf9dfd26",
            ),
        ]
        .into_iter()
        .map(|(t, a)| (t.to_string(), a.to_string()))
        .collect();
        assert_eq!(
            actual, expected,
            "node content addresses changed - if deliberate, repin",
        );
    }

    /// `PlyphonSugar` is composed into the app's `NodeSugar`, so the DSP nodes
    /// serialize as their `~`-prefixed keyword forms (not the generic
    /// `(node "SinOsc" ...)` fallback). Guards that the sugar stays wired in.
    #[test]
    fn dsp_nodes_use_plyphon_keyword_sugar() {
        use gantz_format::Sugar;

        /// The node's wire datum with its `type` tag re-inserted, as the
        /// sugar writer receives it.
        fn tagged_datum(nd: &gantz_ca::NodeData) -> gantz_format::Datum {
            let gantz_ca::Datum::Map(fields) = nd.data.clone() else {
                panic!("node data is not a map");
            };
            gantz_ca::Datum::tagged(&nd.tag, fields)
        }

        let sugar = super::codec().sugars();
        let cases: [(gantz_ca::NodeData, &str, &str); 8] = [
            (
                erased(&gantz_plyphon::SinOsc::default()),
                "SinOsc",
                "~sinosc",
            ),
            (erased(&gantz_plyphon::Out::default()), "Out", "~out"),
            (erased(&gantz_plyphon::Lag::default()), "Lag", "~lag"),
            (
                erased(&gantz_plyphon::ScopeOut::default()),
                "ScopeOut",
                "~scopeout",
            ),
            (erased(&gantz_plyphon::Pack::default()), "Pack", "~pack"),
            (erased(&gantz_plyphon::Sum::default()), "Sum", "~sum"),
            (
                erased(&gantz_plyphon::Unpack::default()),
                "Unpack",
                "~unpack",
            ),
            (erased(&gantz_plyphon::Bus::default()), "Bus", "~bus"),
        ];
        for (nd, tag, expected) in cases {
            let datum = tagged_datum(&nd);
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
        type G = Graph<DynNode>;

        // Two sines packed into one 2-wide edge, across a `~bus`, unpacked,
        // channel 1 to the out - covering the whole dsp node set including the
        // routing pair and the boundary (which the single-def `derive_synthdef`
        // fuses to a plain wire).
        let mut g: G = Graph::default();
        let s0 = g.add_node(Box::new(gantz_plyphon::SinOsc::default()) as DynNode);
        let s1 = g.add_node(Box::new(gantz_plyphon::SinOsc::default()) as DynNode);
        let pk = g.add_node(Box::new(gantz_plyphon::Pack::default()) as DynNode);
        let bus = g.add_node(Box::new(gantz_plyphon::Bus::default()) as DynNode);
        let up = g.add_node(Box::new(gantz_plyphon::Unpack::default()) as DynNode);
        let o = g.add_node(Box::new(gantz_plyphon::Out::default()) as DynNode);
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

    /// A head graph with `~sinosc -> ~out` plus unconnected `inlet`/`outlet`
    /// nodes flattens correctly (root-level boundaries stay as inert non-DSP
    /// markers) and derives a sounding synthdef. Regression guard for the
    /// GUI's `flatten_from_registry` path with `DynNode`.
    #[test]
    fn head_graph_with_unconnected_inlets_derives_sound() {
        use std::time::Duration;

        let text = "\
(graph head
  (s ~sinosc) (out ~out) (i inlet) (o outlet)
  (-> s (out 0)))";
        let registry: DataReg =
            gantz_egui::format::from_str(text, Duration::from_secs(0), &super::codec())
                .expect("from_str");
        let reified = reify_all(&registry);
        let head = gantz_ca::Head::Branch(name("head"));
        let graph = head_graph(&reified, &registry, &head).expect("head graph");

        let flat = gantz_plyphon::flatten_from_registry(graph, &reified).expect("flatten");
        // Root inlet/outlet survive as markers (the head graph's interface);
        // they are non-DSP, so derivation ignores them.
        assert_eq!(
            flat.node_count(),
            4,
            "sin + out + the two root boundary markers"
        );
        assert_eq!(flat.edge_count(), 1, "the sin -> out edge survives");
        let markers = flat
            .node_indices()
            .filter(|&n| {
                matches!(
                    flat[n],
                    gantz_plyphon::Flat::Inlet { .. } | gantz_plyphon::Flat::Outlet { .. }
                )
            })
            .count();
        assert_eq!(markers, 2, "the boundaries are kept as markers");

        let regions = gantz_plyphon::derive_synthdefs(&flat, 1, "head").expect("derive");
        assert_eq!(regions.len(), 1, "one region");
        let names: Vec<&str> = regions[0]
            .derived
            .def
            .units
            .iter()
            .map(|u| u.name.as_str())
            .collect();
        assert_eq!(names, vec!["SinOsc", "BinaryOpUGen", "BinaryOpUGen", "Out"]);
        assert_eq!(
            regions[0].derived.gains.len(),
            1,
            "the out carries a fade gain"
        );
    }

    /// A nested graph of DSP nodes lowers through the instancing pass: the
    /// ref stays an instance marker, the child's `~lag` derives into the
    /// shared child def, and the resolved part's binding carries the lag's
    /// ABSOLUTE nested path - which reaches the node's live param state in
    /// the VM, exactly the contract the audio driver's param sync relies on.
    /// Guards the whole nested-DSP pipeline (registry ref resolution,
    /// flattening, template derivation, VM state bridge) end to end with the
    /// app's real node type.
    #[test]
    fn nested_dsp_graph_flattens_derives_and_bridges_state() {
        use gantz_plyphon::ToNodeDsp;
        use std::time::Duration;

        // child `env:1`: inlet -> ~lag -> outlet, nested into
        // parent `env`: ~sinosc -> ref -> ~out.
        let text = "\
(graph env:1
  (i inlet) (l ~lag) (o outlet)
  (-> i (l 0)) (-> l o))

(graph env
  (s ~sinosc) (sub (ref env:1)) (out ~out)
  (-> s (sub 0)) (-> sub (out 0)))";
        let registry: DataReg =
            gantz_egui::format::from_str(text, Duration::from_secs(0), &super::codec())
                .expect("from_str");
        let reified = reify_all(&registry);
        let parent_head = gantz_ca::Head::Branch(name("env"));
        let child_head = gantz_ca::Head::Branch(name("env:1"));
        let parent = head_graph(&reified, &registry, &parent_head).expect("env graph");
        let child = head_graph(&reified, &registry, &child_head).expect("env:1 graph");

        // The indices the flattened path must carry: the ref within the parent,
        // the lag within the child (its only dsp node).
        let ref_ix = parent
            .node_indices()
            .find(|&n| as_named_ref(&parent[n]).is_some())
            .expect("ref node")
            .index();
        let lag_ix = child
            .node_indices()
            .find(|&n| child[n].to_node_dsp().is_some())
            .expect("lag node")
            .index();

        let flat = gantz_plyphon::flatten_from_registry(parent, &reified).expect("flatten");
        // The DSP-bearing child lowers as an instance marker by default.
        let markers = flat
            .node_indices()
            .filter(|&n| matches!(flat[n], gantz_plyphon::Flat::Instance { .. }))
            .count();
        assert_eq!(markers, 1, "the ref stays an instance marker");
        let children = gantz_plyphon::flatten_instance_children(&flat, &reified).expect("children");
        let resolve = |ca: &gantz_ca::ContentAddr| children.get(ca);
        let mut cache = gantz_plyphon::DefCache::new();
        let template =
            gantz_plyphon::derive_template(&flat, 1, &resolve, &mut cache).expect("derive");
        let parts = gantz_plyphon::instantiate(&template, &cache);
        let binding = parts
            .iter()
            .flat_map(|p| p.params.iter())
            .find(|b| b.node_path == [ref_ix, lag_ix])
            .expect("a param binding keyed by the lag's nested path");

        // The binding's path reaches the nested lag's live param state in a VM
        // compiled from the same (un-flattened) graph.
        let builtins = builtins_with_instances();
        let codec = super::codec();
        let reg_env = env(&registry, &reified, &builtins, &codec);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_env.node(ca);
        let config = gantz_core::compile::Config::default();
        let (mut vm, _compiled) =
            gantz_core::vm::init(&get_node, parent, &[], &config).expect("vm init");
        let (value, pending) = gantz_plyphon::param::drain_param(&mut vm, &binding.node_path)
            .expect("nested lag param state");
        assert_eq!(value, f64::from(gantz_plyphon::Lag::DEFAULT_DUR));
        assert!(pending.is_empty());
    }

    /// Two references to one DSP child share a single derived variant: one
    /// `DefCache` entry, both resolved parts naming the same content-hashed
    /// def, with per-instance absolute binding paths. Guards the install-once
    /// spawn-many contract end to end with the app's real node type.
    #[test]
    fn instanced_refs_share_one_variant() {
        use std::time::Duration;

        let text = "\
(graph voice
  (s ~sinosc) (out ~out)
  (-> s (out 0)))

(graph env
  (a (ref voice)) (b (ref voice)))";
        let registry: DataReg =
            gantz_egui::format::from_str(text, Duration::from_secs(0), &super::codec())
                .expect("from_str");
        let reified = reify_all(&registry);
        let head = gantz_ca::Head::Branch(name("env"));
        let parent = head_graph(&reified, &registry, &head).expect("env graph");

        let flat = gantz_plyphon::flatten_from_registry(parent, &reified).expect("flatten");
        let children = gantz_plyphon::flatten_instance_children(&flat, &reified).expect("children");
        let resolve = |ca: &gantz_ca::ContentAddr| children.get(ca);
        let mut cache = gantz_plyphon::DefCache::new();
        let template =
            gantz_plyphon::derive_template(&flat, 1, &resolve, &mut cache).expect("derive");
        assert_eq!(cache.len(), 1, "both refs share one variant");
        let parts = gantz_plyphon::instantiate(&template, &cache);
        assert_eq!(parts.len(), 2, "one spawn per instance");
        assert_eq!(parts[0].def.name, parts[1].def.name, "one shared def");
        assert_ne!(parts[0].key, parts[1].key, "distinct identities");
        let mut prefixes: Vec<usize> = parts.iter().map(|p| p.params[0].node_path[0]).collect();
        prefixes.sort_unstable();
        prefixes.dedup();
        assert_eq!(prefixes.len(), 2, "bindings carry per-instance prefixes");
    }

    /// `~unpack`'s placeholder expr honours the multi-output contract for any
    /// `count`: a single value for one output, a list of values otherwise. A
    /// wrong shape (e.g. `(list 0)` for count 1) fails `vm::init`'s compile.
    #[test]
    fn unpack_expr_is_steel_inert_for_any_count() {
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;
        type G = Graph<DynNode>;

        for count in [1usize, 2, 3] {
            let mut unpack = gantz_plyphon::Unpack::default();
            unpack.set_count(count);
            let mut g: G = Graph::default();
            let s = g.add_node(Box::new(gantz_plyphon::SinOsc::default()) as DynNode);
            let up = g.add_node(Box::new(unpack) as DynNode);
            let insp = g.add_node(Box::new(gantz_egui::node::Inspect::default()) as DynNode);
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
        type G = Graph<DynNode>;

        // number (a push source) -> ~sinosc.freq (control input at index 0).
        let mut g: G = Graph::default();
        let num = g.add_node(Box::new(gantz_std::Number::default()) as DynNode);
        let sine = g.add_node(Box::new(gantz_plyphon::SinOsc::default()) as DynNode);
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

    /// The hybrid side of `~sinosc`'s freq input: a *dsp* source pushed through
    /// it must NOT touch the param state - dsp placeholder outputs are
    /// non-numeric by contract, so the `number?` guard skips the write. (The
    /// derived def reads the wire instead, so a queued update would have no
    /// param to drain into and `pending` would grow unboundedly.)
    #[test]
    fn dsp_wire_into_freq_leaves_param_state_untouched() {
        use gantz_core::compile::push_pull_entrypoints;
        use gantz_core::edge::Edge;
        use gantz_core::node::graph::Graph;
        use gantz_core::steel::SteelVal;
        type G = Graph<DynNode>;

        // number -> ~lag (dsp input) -> ~sinosc.freq: pushing the number fires
        // the whole chain, so the lag's placeholder reaches the freq input.
        let mut g: G = Graph::default();
        let num = g.add_node(Box::new(gantz_std::Number::default()) as DynNode);
        let lag = g.add_node(Box::new(gantz_plyphon::Lag::default()) as DynNode);
        let sine = g.add_node(Box::new(gantz_plyphon::SinOsc::default()) as DynNode);
        g.add_edge(num, lag, Edge::new(0.into(), 0.into()));
        g.add_edge(lag, sine, Edge::new(0.into(), 0.into()));

        let get_node = |_: &gantz_ca::ContentAddr| -> Option<&dyn gantz_core::Node> { None };
        let config = gantz_core::compile::Config::default();
        let eps = push_pull_entrypoints(&get_node, &g);
        let (mut vm, _compiled) = gantz_core::vm::init(&get_node, &g, &eps, &config)
            .expect("compile number -> ~lag -> ~sinosc");

        vm.update_value(gantz_core::ARGS, gantz_core::args::time(1.25));
        gantz_core::node::state::update_value(&mut vm, &[num.index()], SteelVal::NumV(440.0))
            .expect("set number state");
        fire_push(&mut vm, &eps, num.index());

        let (value, pending) = gantz_plyphon::param::drain_param(&mut vm, &[sine.index()])
            .expect("~sinosc param state present");
        assert_eq!(
            value,
            f64::from(gantz_plyphon::SinOsc::DEFAULT_FREQ),
            "a dsp wire must not overwrite the param value",
        );
        assert!(
            pending.is_empty(),
            "a dsp wire must not queue param updates",
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
        type G = Graph<DynNode>;

        // number -> ~scopeout.trigger (input 1, after the dsp input); output 0 ->
        // inspect_samples, output 1 -> inspect_channels.
        let mut g: G = Graph::default();
        let num = g.add_node(Box::new(gantz_std::Number::default()) as DynNode);
        let tap = g.add_node(Box::new(gantz_plyphon::ScopeOut::default()) as DynNode);
        let samples = g.add_node(Box::new(gantz_egui::node::Inspect::default()) as DynNode);
        let chans = g.add_node(Box::new(gantz_egui::node::Inspect::default()) as DynNode);
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
        type G = Graph<DynNode>;

        // number -> ~scopeout.dsp (input 0); output 0 -> inspect. Firing the number
        // pushes the dsp input, leaving the trigger (input 1) inactive.
        let mut g: G = Graph::default();
        let num = g.add_node(Box::new(gantz_std::Number::default()) as DynNode);
        let tap = g.add_node(Box::new(gantz_plyphon::ScopeOut::default()) as DynNode);
        let inspect = g.add_node(Box::new(gantz_egui::node::Inspect::default()) as DynNode);
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

        let base: DataReg = gantz_egui::export::parse_export(gantz_base::BYTES, &super::codec())
            .expect("parse base");
        let base_head = gantz_ca::Head::Branch(name("mul"));
        let base_graph = base.head_graph(&base_head).expect("base mul graph");
        let base_addr = gantz_ca::ContentAddr::from(gantz_ca::graph_addr(base_graph)).to_string();

        let text = "\
(graph mul
  (m (expr (* $l $r)))
  (l (inlet \"number\" \"left operand\")) (r (inlet \"number\" \"right operand\")) (out (outlet \"number\" \"product\"))
  (-> l (m 0)) (-> r (m 1)) (-> m out))";
        let mine: DataReg =
            gantz_egui::format::from_str(text, Duration::from_secs(0), &super::codec())
                .expect("lower");
        let head = gantz_ca::Head::Branch(name("mul"));
        let graph = mine.head_graph(&head).expect("mul graph");
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

        let export1: DataReg =
            gantz_egui::format::from_str(text1, now, &super::codec()).expect("from_str 1");
        let text2 = gantz_egui::format::to_string(&export1, &super::codec()).expect("to_string");
        let export2: DataReg =
            gantz_egui::format::from_str(&text2, Duration::from_secs(7), &super::codec())
                .expect("from_str 2");

        let names1: BTreeSet<_> = export1.heads().map(|(n, _)| n.clone()).collect();
        let names2: BTreeSet<_> = export2.heads().map(|(n, _)| n.clone()).collect();
        assert_eq!(names1, names2, "names must match\n--- text2 ---\n{text2}");

        for (name, head1) in export1.heads() {
            let head2 = export2.head(name).expect("name present");
            assert_eq!(
                head1, head2,
                "commit addr for `{name}`\n--- text2 ---\n{text2}"
            );
            let g1 = export1.commit_graph_ref(&head1).expect("g1");
            let g2 = export2.commit_graph_ref(&head2).expect("g2");
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

        let base: DataReg = gantz_egui::export::parse_export(gantz_base::BYTES, &super::codec())
            .expect("parse base");
        let text = gantz_egui::format::to_string(&base, &super::codec()).expect("to_string");
        let back: DataReg =
            gantz_egui::format::from_str(&text, Duration::from_secs(0), &super::codec())
                .expect("from_str");

        let base_names: BTreeSet<_> = base.heads().map(|(n, _)| n.clone()).collect();
        let back_names: BTreeSet<_> = back.heads().map(|(n, _)| n.clone()).collect();
        assert_eq!(
            base_names, back_names,
            "names preserved\n--- text ---\n{text}"
        );

        // base.gantz is consistent: addresses survive the round-trip exactly.
        for (name, head) in base.heads() {
            assert_eq!(
                Some(head),
                back.head(name),
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
        let e1: DataReg =
            gantz_egui::format::from_str(text1, now, &super::codec()).expect("from_str 1");
        let text2 = gantz_egui::format::to_string(&e1, &super::codec()).expect("to_string");
        let e2: DataReg =
            gantz_egui::format::from_str(&text2, now, &super::codec()).expect("from_str 2");

        for n in ["env", "env:1"] {
            let head = gantz_ca::Head::Branch(name(n));
            let g1 = e1.head_graph(&head).expect("g1");
            let g2 = e2.head_graph(&head).expect("g2");
            assert_eq!(
                gantz_ca::graph_addr(g1),
                gantz_ca::graph_addr(g2),
                "graph addr for `{n}` must survive round-trip\n--- text2 ---\n{text2}",
            );
        }
    }

    /// The serializer's output is reader-valid Steel: Steel's own parser accepts
    /// every form. This is the property the whole format design rests on.
    #[test]
    fn output_is_valid_steel() {
        use std::time::Duration;

        let text1 = "\
(graph g
  (n (number))
  (s (expr (values $x (* $x 2)) #:out 2))
  (b (branch (if $v (list 0 0) (list 1 0)) \"10\" \"01\"))
  (c (comment \"hello world\" 16 2))
  (l (log warn))
  (-> n (s 0)) (-> (s 1) (b 0)))";
        let registry: DataReg =
            gantz_egui::format::from_str(text1, Duration::from_secs(0), &super::codec())
                .expect("from_str");
        let out = gantz_egui::format::to_string(&registry, &super::codec()).expect("to_string");
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

        let text = "\
(graph g
  (t (tick-bang #:rate 2))
  (l (log warn))
  (-> t (l 0)))";
        let registry: DataReg =
            gantz_egui::format::from_str(text, Duration::from_secs(0), &super::codec())
                .expect("from_str");
        let reified = reify_all(&registry);
        let head = gantz_ca::Head::Branch(name("g"));
        let graph = head_graph(&reified, &registry, &head).expect("g graph");

        let builtins = builtins_with_instances();
        let codec = super::codec();
        let reg_env = env(&registry, &reified, &builtins, &codec);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_env.node(ca);

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

        let text = "\
(graph g (e (expr 1)))
(commits (\"abcd1234\" (time 5 0) (parent \"deadbeef\") (graph g)))
(names (gname \"abcd1234\"))";
        let registry: DataReg =
            gantz_egui::format::from_str(text, Duration::from_secs(0), &super::codec())
                .expect("import");
        let commit = registry.named_commit(&name("gname")).expect("commit");
        assert_eq!(commit.parent, None, "absent parent must be cleared to None");
    }

    /// The Export-level format (gantz_egui over gantz_format) round-trips
    /// `(layout ...)` view state: node positions and the camera survive
    /// text -> Export -> text -> Export.
    #[test]
    fn layout_roundtrips() {
        use std::time::Duration;

        let now = Duration::from_secs(5);
        let text1 = "\
(graph mul
  (m (expr (* $l $r)))
  (l inlet) (r inlet) (out outlet)
  (-> l (m 0)) (-> r (m 1)) (-> m out))

(layout mul
  (m -10 20) (l 3.5 -4.5)
  (camera 25 -15 1.5))";

        let e1: DataReg =
            gantz_egui::format::from_str(text1, now, &super::codec()).expect("from_str 1");
        let head = e1.head(&name("mul")).expect("mul name");
        let view = gantz_egui::section::view(&e1, &head).expect("view");
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

        let text2 = gantz_egui::format::to_string(&e1, &super::codec()).expect("to_string");
        let e2: DataReg =
            gantz_egui::format::from_str(&text2, now, &super::codec()).expect("from_str 2");
        let head2 = e2.head(&name("mul")).expect("mul name 2");
        let view2 = gantz_egui::section::view(&e2, &head2).expect("view 2");
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

        let now = Duration::from_secs(5);
        let text = "\
(graph mul
  (m (expr (* $l $r)))
  (l inlet) (r inlet) (out outlet)
  (-> l (m 0)) (-> r (m 1)) (-> m out))

(layout mul
  (m -10 20)
  (scene -50 -50 100 100))";

        let e: DataReg =
            gantz_egui::format::from_str(text, now, &super::codec()).expect("from_str");
        let head = e.head(&name("mul")).expect("mul name");
        let view = gantz_egui::section::view(&e, &head).expect("view");
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
        use std::collections::HashSet;

        // The clipboard operates on the working graph's data form.
        let mut graph = data_graph([
            erased(&gantz_core::node::Identity),
            erased(&gantz_core::node::Identity),
        ]);
        let (a, b) = (0.into(), 1.into());
        graph.add_edge(a, b, gantz_core::Edge::new(0.into(), 0.into()));

        let registry = DataReg::default();
        let mut layout = egui_graph::Layout::default();
        layout.insert(egui_graph::NodeId(0), egui::pos2(1.0, 2.0));
        layout.insert(egui_graph::NodeId(1), egui::pos2(3.0, 4.0));
        let selected: HashSet<gantz_core::node::graph::NodeIx> = [a, b].into_iter().collect();

        let copied = export::copy(&registry, &graph, &selected, &layout);
        let text = export::copied_to_string(&copied, &super::codec()).expect("copied to text");
        // The clipboard payload is itself reader-valid `.gantz` text.
        steel::parser::parser::Parser::parse(&text)
            .unwrap_or_else(|e| panic!("clipboard text is not valid Steel: {e}\n{text}"));

        let back: export::Copied =
            export::copied_from_str(&text, &super::codec()).expect("copied from text");
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
    /// `NamedRef` references the child's new graph.
    #[test]
    fn resync_propagates_child_edit_to_parent() {
        use gantz_core::node::{Identity, Ref};
        use gantz_egui::node::NamedRef;
        use std::time::Duration;

        let ts = Duration::from_secs(0);
        let mut registry = DataReg::default();

        // Child "p:1": a single node.
        let child = data_graph([erased(&Identity)]);
        let (child_old, child_addr) = commit_to_name(&mut registry, ts, child, &name("p:1"));

        // Parent "p": a sync-enabled NamedRef to "p:1"'s head graph.
        let parent = data_graph([erased(&NamedRef::with_sync(
            name("p:1"),
            Ref::new(child_addr.into()),
        ))]);
        let (parent_old, _) = commit_to_name(&mut registry, ts, parent, &name("p"));

        // Edit the child: commit a different graph under "p:1".
        let child2 = data_graph([erased(&Identity), erased(&Identity)]);
        let (child_new, child2_addr) = commit_to_name(&mut registry, ts, child2, &name("p:1"));
        assert_ne!(child_old, child_new);

        // Resync: the parent must follow the child's new head graph.
        let moves = gantz_egui::sync::resync(&mut registry, ts);
        assert!(
            moves.iter().any(|m| m.name == name("p")),
            "parent should have recommitted: {moves:?}"
        );

        let parent_new = registry.head(&name("p")).unwrap();
        assert_ne!(parent_old, parent_new, "parent commit must change");
        let reified = reify_all(&registry);
        let p_graph = reified.get(&registry.commits()[&parent_new].graph).unwrap();
        let points_at_new_child = p_graph.node_weights().any(|n| {
            as_named_ref(n)
                .map(|nr| nr.content_addr() == child2_addr.into())
                .unwrap_or(false)
        });
        assert!(
            points_at_new_child,
            "parent's NamedRef must reference the child's new graph"
        );
    }

    /// Forking a graph with a nested child gives the fork its *own* child:
    /// [`sync::fork_nested`] copies the `parent:*` subtree to the fork and
    /// rewrites its references, leaving the original's children untouched.
    #[test]
    fn fork_nested_gives_independent_children() {
        use gantz_core::node::{Identity, Ref};
        use gantz_egui::node::NamedRef;
        use std::time::Duration;

        let ts = Duration::from_secs(0);
        let mut registry = DataReg::default();

        // Child "A:1" and parent "A" referencing it.
        let child = data_graph([erased(&Identity)]);
        let (_, child_addr) = commit_to_name(&mut registry, ts, child, &name("A:1"));
        let parent = data_graph([erased(&NamedRef::with_sync(
            name("A:1"),
            Ref::new(child_addr.into()),
        ))]);
        commit_to_name(&mut registry, ts, parent, &name("A"));

        // Fork "A" -> "B": a fresh commit over A's graph (as `on_branch_head` does),
        // so "B" initially references A's child "A:1".
        let a_commit = registry.head(&name("A")).unwrap();
        let a_graph = registry.commits()[&a_commit].graph;
        let b_commit = registry.commit_graph(ts, Some(a_commit), a_graph, || unreachable!());
        registry.set_head(name("B"), b_commit);

        // Cascade: give "B" its own nested child "B:1".
        let moves = gantz_egui::sync::fork_nested(&mut registry, ts, &name("A"), &name("B"));
        assert!(
            moves.iter().any(|m| m.name == name("B:1")),
            "B:1 should be created: {moves:?}"
        );
        assert!(
            moves.iter().any(|m| m.name == name("B")),
            "B's root should be rewritten: {moves:?}"
        );

        // B references its own child B:1's graph; A:1 is untouched.
        let b1: gantz_ca::ContentAddr = registry.named_commit(&name("B:1")).unwrap().graph.into();
        let b_new = registry.head(&name("B")).unwrap();
        let reified = reify_all(&registry);
        let b_graph = reified.get(&registry.commits()[&b_new].graph).unwrap();
        let refs_b1 = b_graph.node_weights().any(|n| {
            as_named_ref(n)
                .map(|nr| nr.name() == &name("B:1") && nr.content_addr() == b1)
                .unwrap_or(false)
        });
        assert!(refs_b1, "the fork's root must reference its own child B:1");
        assert!(
            registry.head(&name("A:1")).is_some(),
            "the original child A:1 must remain"
        );
    }

    /// Copying a node that references a nested graph and pasting it must keep
    /// the reference. The reference pins the nested graph's content (graph
    /// address), which the clipboard payload carries along with the naming
    /// head, so the pasted `NamedRef` must still resolve rather than vanish.
    #[test]
    fn clipboard_round_trips_nested_ref() {
        use gantz_core::node::{Identity, Ref};
        use gantz_egui::export;
        use gantz_egui::node::NamedRef;
        use std::collections::HashSet;
        use std::time::Duration;

        let mut registry = DataReg::default();

        // Nested graph "A:1", committed twice so its head commit has a parent
        // (the format does not preserve the parent chain).
        let v1 = data_graph([erased(&Identity)]);
        commit_to_name(&mut registry, Duration::from_secs(1), v1, &name("A:1"));
        let v2 = data_graph([erased(&Identity), erased(&Identity)]);
        let (_, v2_addr) = commit_to_name(&mut registry, Duration::from_secs(2), v2, &name("A:1"));

        // A graph (in data form) holding a synced NamedRef to "A:1"'s head
        // graph.
        let mut graph = gantz_ca::DataGraph::default();
        let named = NamedRef::with_sync(name("A:1"), Ref::new(v2_addr.into()));
        let nref = graph.add_node(gantz_core::data::erase_node_typed(&named).expect("erase"));
        let selected: HashSet<_> = [nref].into_iter().collect();

        // Copy -> clipboard text -> paste.
        let copied = export::copy(&registry, &graph, &selected, &egui_graph::Layout::default());
        let text = export::copied_to_string(&copied, &super::codec()).expect("copied to text");
        let back: export::Copied =
            export::copied_from_str(&text, &super::codec()).expect("copied from text");

        assert_eq!(back.graph.node_count(), 1, "the nested-ref node must paste");
        let kept = back.graph.node_weights().any(|n| {
            gantz_core::data::reify_node_concrete::<NamedRef>(n)
                .map(|nr| nr.name() == &name("A:1"))
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
        use std::time::Duration;

        let ts = Duration::from_secs(0);
        let mut registry = DataReg::default();

        // Nested child "A:1".
        let child = data_graph([erased(&Identity)]);
        let (a1, child_addr) = commit_to_name(&mut registry, ts, child, &name("A:1"));

        // Parent "A" with THREE instances of the nested graph.
        let named = NamedRef::with_sync(name("A:1"), Ref::new(child_addr.into()));
        let parent = data_graph(std::iter::repeat_n(erased(&named), 3));
        commit_to_name(&mut registry, ts, parent, &name("A"));

        // Simulate "rename A:1 -> B": a root "B" copy of A:1's graph (as the
        // fork does), then promote.
        let a1_graph = registry.commits()[&a1].graph;
        let b = registry.commit_graph(ts, Some(a1), a1_graph, || unreachable!());
        registry.set_head(name("B"), b);
        let moves = gantz_egui::sync::promote_nested(&mut registry, ts, &name("A:1"), &name("B"));

        assert!(
            moves.iter().any(|m| m.name == name("A")),
            "parent A must recommit"
        );
        assert!(
            registry.head(&name("A:1")).is_none(),
            "the orphaned nested name must be dropped"
        );

        // All three parent references now point at "B".
        let a_commit = registry.head(&name("A")).unwrap();
        let reified = reify_all(&registry);
        let a_graph = reified.get(&registry.commits()[&a_commit].graph).unwrap();
        let to_b = a_graph
            .node_weights()
            .filter(|n| {
                as_named_ref(n)
                    .map(|nr| nr.name() == &name("B"))
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
        let base: DataReg = gantz_egui::export::parse_export(gantz_base::BYTES, &super::codec())
            .expect("parse base");
        let reified = reify_all(&base);
        let builtins = builtins_with_instances();
        let codec = super::codec();
        let reg_env = env(&base, &reified, &builtins, &codec);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_env.node(ca);
        let configs = [
            gantz_core::compile::Config::default(),
            gantz_core::compile::Config {
                validate_ir: true,
                emit_all_node_fns: true,
            },
        ];

        assert!(
            base.heads().next().is_some(),
            "base.gantz registered no named graphs",
        );
        for (name, _) in base.heads() {
            let head = gantz_ca::Head::Branch(name.clone());
            let graph = head_graph(&reified, &base, &head)
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
        let base: DataReg = gantz_egui::export::parse_export(gantz_base::BYTES, &super::codec())
            .expect("parse base");
        let text = gantz_egui::format::to_string(&base, &super::codec()).expect("to_string");
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
    /// `#[serde(skip)]` to `NamedRef::sync`.
    #[test]
    fn named_ref_sync_affects_content_addr() {
        use gantz_egui::node::NamedRef;
        let ca = gantz_ca::ContentAddr::from([0u8; 32]);
        let ref_ = gantz_core::node::Ref::new(ca);
        let off = NamedRef::new(name("x"), ref_.clone());
        let on = NamedRef::with_sync(name("x"), ref_);
        assert_ne!(
            erased_addr(&off),
            erased_addr(&on),
            "toggling `sync` must change the content address, otherwise the \
             toggle can't trigger a commit and won't persist",
        );
    }

    /// A node's erased (data-layer) content address.
    fn erased_addr(named: &gantz_egui::node::NamedRef) -> gantz_ca::ContentAddr {
        gantz_core::data::erase_node_typed(named)
            .unwrap()
            .content_addr()
    }

    /// The ext-free `NamedRef` erased address must never change: it is the
    /// address every existing graph's references already hash to.
    #[test]
    fn named_ref_ext_free_content_addr_is_pinned() {
        use gantz_egui::node::NamedRef;
        let ca = gantz_ca::ContentAddr::from([0u8; 32]);
        let named = NamedRef::new(name("mul"), gantz_core::node::Ref::new(ca));
        assert_eq!(
            erased_addr(&named).to_string(),
            "207f946ce3dfee38b4da6616a775f7b575e4f850b4ee945c8c4d061737314cc0",
            "ext-free NamedRef CA changed - this breaks every existing graph address",
        );
    }

    /// Ref ext data participates in the `NamedRef` address, and survives every
    /// repointing operation: rename cascades, resync, and node forking (which
    /// deliberately still resets `sync`).
    #[test]
    fn named_ref_ext_survives_repointing_and_fork() {
        use gantz_egui::node::NamedRef;

        #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
        struct TestExt {
            inline: bool,
        }
        let ext = TestExt { inline: true };
        let key = "test.ext";

        let ca = gantz_ca::ContentAddr::from([0u8; 32]);
        let mut named = NamedRef::new(name("mul"), gantz_core::node::Ref::new(ca));
        let plain_ca = erased_addr(&named);
        named.set_ext(key, &ext).unwrap();
        assert_ne!(erased_addr(&named), plain_ca);

        // Rename cascade repoints - ext rides.
        named.rename(name("mul2"), gantz_ca::ContentAddr::from([1u8; 32]));
        assert_eq!(named.ext_as::<TestExt>(key), Some(TestExt { inline: true }));

        // Resync repoints - ext rides.
        let mut synced = NamedRef::with_sync(name("mul"), gantz_core::node::Ref::new(ca));
        synced.set_ext(key, &ext).unwrap();
        let latest = gantz_ca::ContentAddr::from([2u8; 32]);
        assert!(synced.resync(|_| Some(latest)));
        assert_eq!(synced.content_addr(), latest);
        assert_eq!(
            synced.ext_as::<TestExt>(key),
            Some(TestExt { inline: true })
        );

        // Forking via branch_node replaces the node but carries ext over.
        let mut registry = DataReg::default();
        let now = std::time::Duration::from_secs(1);
        let child = gantz_ca::DataGraph::default();
        let (_, child_addr) = commit_to_name(&mut registry, now, child, &name("child"));

        // The working graph is data: the ext-carrying `NamedRef` erases in.
        let mut graph = gantz_ca::DataGraph::default();
        let mut named =
            NamedRef::with_sync(name("child"), gantz_core::node::Ref::new(child_addr.into()));
        named.set_ext(key, &ext).unwrap();
        let ix = graph.add_node(gantz_core::data::erase_node_typed(&named).unwrap());

        gantz_egui::ops::branch_node(
            &mut registry,
            now,
            &mut graph,
            "fork".to_string(),
            child_addr.into(),
            &[ix.index()],
        );
        let forked: NamedRef =
            gantz_core::data::reify_node_concrete(&graph[ix]).expect("named ref");
        assert_eq!(forked.name(), &name("fork"));
        assert_eq!(
            forked.ext_as::<TestExt>(key),
            Some(TestExt { inline: true }),
            "fork must carry ext over - the forked content is identical",
        );
        // Whole-node identity: name, sync reset (a fork pins) and ext all
        // land as an ext-carrying `NamedRef::new` of the fork's graph.
        assert!(registry.head(&name("fork")).is_some(), "fork name");
        let mut expected =
            NamedRef::new(name("fork"), gantz_core::node::Ref::new(child_addr.into()));
        expected.set_ext(key, &ext).unwrap();
        assert_eq!(
            graph[ix],
            gantz_core::data::erase_node_typed(&expected).unwrap(),
        );
    }

    /// An ext-carrying reference round-trips through the `.gantz` text format:
    /// the `#:ext` tail survives, and so does the graph's commit address
    /// (ext is CA-relevant, so a lossy round-trip would heal to a different
    /// address).
    #[test]
    fn ext_text_roundtrip_preserves_addr_and_ext() {
        use std::time::Duration;

        let now = Duration::from_secs(1_000_000);
        let text1 = "\
(graph mul
  (m (expr (* $l $r)))
  (l inlet) (r inlet) (out outlet)
  (-> l (m 0)) (-> r (m 1)) (-> m out))

(graph use-mul
  (a inlet) (b inlet) (out outlet)
  (mref (ref mul #:ext ((\"test.ext\" ((inline #t))))))
  (-> a (mref 0)) (-> b (mref 1)) (-> mref out))";

        let export1: DataReg =
            gantz_egui::format::from_str(text1, now, &super::codec()).expect("from_str 1");
        let text2 = gantz_egui::format::to_string(&export1, &super::codec()).expect("to_string");
        assert!(text2.contains("#:ext"), "ext tail must survive\n{text2}");
        let export2: DataReg =
            gantz_egui::format::from_str(&text2, Duration::from_secs(7), &super::codec())
                .expect("from_str 2");

        for (name, head1) in export1.heads() {
            let head2 = export2.head(name).expect("name present");
            assert_eq!(
                head1, head2,
                "commit addr for `{name}` (ext is CA-relevant)\n--- text2 ---\n{text2}"
            );
        }

        // The ext data itself survives on the re-parsed node.
        #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
        struct TestExt {
            inline: bool,
        }
        let head = export2.head(&name("use-mul")).expect("use-mul");
        let reified = reify_all(&export2);
        let g = reified.get(&export2.commits()[&head].graph).expect("graph");
        let named = g
            .node_indices()
            .find_map(|ix| as_named_ref(&g[ix]))
            .expect("a named ref in use-mul");
        assert_eq!(
            named.ext_as::<TestExt>("test.ext"),
            Some(TestExt { inline: true })
        );
    }

    /// An ext-carrying `NamedRef` round-trips through the node codec with its
    /// stored form (and thus its content address) intact - ext-free output is
    /// unchanged, as the addr pins verify.
    #[test]
    fn ext_roundtrips_through_codec() {
        use gantz_egui::node::NamedRef;

        #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
        struct TestExt {
            inline: bool,
        }
        let ca = gantz_ca::ContentAddr::from([0u8; 32]);
        let mut named = NamedRef::new(name("mul"), gantz_core::node::Ref::new(ca));
        named
            .set_ext("test.ext", &TestExt { inline: true })
            .unwrap();
        let nd = erased(&named);

        let inst = super::codec().reify_ui(&nd).expect("reify");
        assert_eq!(inst.erase().expect("erase"), nd, "codec round-trip");
        let named = as_named_ref(&inst.node).expect("named");
        assert_eq!(
            named.ext_as::<TestExt>("test.ext"),
            Some(TestExt { inline: true })
        );
    }

    /// Every base-primitive socket carries a hover doc (type + description), and
    /// those docs resolve through a `ref` to the referenced graph's inlet/outlet
    /// markers - exactly the path the GUI uses for a `NamedRef`'s socket tooltip.
    #[test]
    fn base_socket_docs() {
        use gantz_egui::SocketKind;
        let base: DataReg = gantz_egui::export::parse_export(gantz_base::BYTES, &super::codec())
            .expect("parse base");

        // Completeness: no primitive socket serializes as a bare `inlet`/`outlet`.
        let text = gantz_egui::format::to_string(&base, &super::codec()).expect("to_string");
        let bare = text.matches(" inlet)").count() + text.matches(" outlet)").count();
        assert_eq!(
            bare, 0,
            "every base socket must be documented\n--- text ---\n{text}"
        );

        // Resolution: a `ref add` exposes `add`'s socket docs.
        let builtins = builtins_with_instances();
        let reified = reify_all(&base);
        let codec = super::codec();
        let reg_env = env(&base, &reified, &builtins, &codec);
        let add: gantz_ca::ContentAddr = gantz_egui::reg::head_graph_addr(&base, &name("add"))
            .expect("add")
            .into();
        let doc = |kind, ix| reg_env.socket_doc(&add, kind, ix);

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

        let base: DataReg = gantz_egui::export::parse_export(gantz_base::BYTES, &super::codec())
            .expect("parse base");
        let reified = reify_all(&base);
        let builtins = builtins_with_instances();
        let codec = super::codec();
        let reg_env = env(&base, &reified, &builtins, &codec);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_env.node(ca);
        let config = gantz_core::compile::Config::default();

        let demos = [
            "demo-arithmetic",
            "demo-comparison",
            "demo-logic",
            "demo-list",
            "demo-predicate",
        ];
        for demo in demos {
            let head = gantz_ca::Head::Branch(name(demo));
            let graph =
                head_graph(&reified, &base, &head).unwrap_or_else(|| panic!("{demo} graph"));

            // The single `bang` node drives every pipeline in the demo.
            let go = graph
                .node_indices()
                .find(|&ix| {
                    (&*graph[ix] as &dyn std::any::Any)
                        .downcast_ref::<gantz_std::Bang>()
                        .is_some()
                })
                .map(|ix| ix.index())
                .unwrap_or_else(|| panic!("{demo} has a bang"));

            let eps = push_pull_entrypoints(&get_node, graph);
            let (mut vm, _compiled) = gantz_core::vm::init(&get_node, graph, &eps, &config)
                .unwrap_or_else(|e| panic!("init {demo}: {}", gantz_core::vm::error_chain(&e)));

            let go_ep = eps
                .iter()
                .find(|ep| {
                    ep.0.iter()
                        .any(|s| s.kind == EvalKind::Push && s.path == [go])
                })
                .unwrap_or_else(|| panic!("{demo} bang entrypoint"));
            vm.call_function_by_name_with_args(&entry_fn_name(&go_ep.id()), vec![])
                .unwrap_or_else(|e| panic!("firing {demo} bang errored: {e}"));
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
        use std::collections::BTreeMap;

        let ts = bevy_gantz_egui::base::BASE_TIMESTAMP;
        let parse = || -> DataReg {
            gantz_egui::export::parse_export_at(gantz_base::BYTES, ts, &super::codec())
                .expect("parse base")
        };
        let heads = |reg: &DataReg| -> BTreeMap<_, _> {
            reg.heads().map(|(n, ca)| (n.clone(), ca)).collect()
        };

        // Parsing the base at the fixed timestamp is reproducible: every name
        // maps to the same commit both times - what lets a reset agree with the
        // registry loaded at startup.
        let startup = parse();
        let reparse = parse();
        assert_eq!(
            heads(&startup),
            heads(&reparse),
            "base commit addresses must be reproducible across parses",
        );

        // Simulate `on_reset_base_graph`: re-export the demo's reachable
        // subset from a fresh parse and merge it into the startup registry.
        let mut registry = startup;
        let demo = name("demo-arithmetic");
        let demo_commit = reparse.head(&demo).expect("demo name");
        let live = gantz_ca::closure_from(&reparse, [demo_commit]);
        let mut subset = gantz_ca::export(&reparse, &live);
        subset.set_head(demo.clone(), demo_commit);
        registry.merge(subset);

        // Reopen: the reset demo must still compile, i.e. every `ref` resolves.
        let builtins = builtins_with_instances();
        let reified = reify_all(&registry);
        let codec = super::codec();
        let reg_env = env(&registry, &reified, &builtins, &codec);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_env.node(ca);
        let head = gantz_ca::Head::Branch(demo);
        let graph = head_graph(&reified, &registry, &head).expect("demo graph");
        let eps = push_pull_entrypoints(&get_node, graph);
        gantz_core::vm::init(&get_node, graph, &eps, &Config::default()).unwrap_or_else(|e| {
            panic!(
                "recompile after reset failed: {}",
                gantz_core::vm::error_chain(&e)
            )
        });
    }

    /// A `~playbuf`'s buffer reference wires audio blobs into reachability:
    /// prune and export keep exactly the buffers live graphs reference, and
    /// drop the rest.
    #[test]
    fn playbuf_buffers_ride_reachability() {
        use std::time::Duration;

        let mut registry = DataReg::default();
        let used = gantz_plyphon::AudioAsset::from_interleaved(vec![0.5; 8], 1, 48_000.0);
        let unused = gantz_plyphon::AudioAsset::from_interleaved(vec![-0.5; 8], 1, 48_000.0);
        let used_addr = gantz_plyphon::add_audio_asset(&mut registry, &used);
        let unused_addr = gantz_plyphon::add_audio_asset(&mut registry, &unused);

        let dg = data_graph([erased(&gantz_plyphon::PlayBuf::new(used_addr, 1, 48_000.0))]);
        let g_addr = gantz_ca::graph_addr(&dg);
        registry.add_graph(dg);
        let commit = registry.commit_graph(Duration::from_secs(1), None, g_addr, || {
            unreachable!("graph already added")
        });
        registry.set_head(name("sampler"), commit);

        // Heads are a Root-liveness section, so the closure seeds from them.
        let live = gantz_ca::closure(&registry, [] as [gantz_ca::CommitAddr; 0]);
        assert!(live.blob_live(gantz_plyphon::BUFFER_SECTION, &used_addr));
        assert!(!live.blob_live(gantz_plyphon::BUFFER_SECTION, &unused_addr));

        // Export carries only the referenced buffer.
        let exported = gantz_ca::export(&registry, &live);
        assert!(gantz_plyphon::audio_asset(&exported, &used_addr).is_some());
        assert!(gantz_plyphon::audio_asset(&exported, &unused_addr).is_none());

        // Prune drops the unreferenced one in place.
        gantz_ca::prune(&mut registry, &live);
        assert_eq!(
            gantz_plyphon::audio_asset(&registry, &used_addr),
            Some(used)
        );
        assert!(gantz_plyphon::audio_asset(&registry, &unused_addr).is_none());
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

        let base: DataReg = gantz_egui::export::parse_export(gantz_base::BYTES, &super::codec())
            .expect("parse base");
        let text =
            gantz_egui::format::to_string_named(&base, &super::codec()).expect("to_string_named");

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
        let back: DataReg =
            gantz_egui::format::from_str(&text, Duration::from_secs(0), &super::codec())
                .expect("from_str");
        let text2 =
            gantz_egui::format::to_string_named(&back, &super::codec()).expect("to_string_named 2");
        assert_eq!(text, text2, "inline-name export must be idempotent");

        // Names survive the round-trip.
        let n1: BTreeSet<_> = base.heads().map(|(n, _)| n.clone()).collect();
        let n2: BTreeSet<_> = back.heads().map(|(n, _)| n.clone()).collect();
        assert_eq!(n1, n2, "names preserved");
    }

    /// The plyphon base source is exactly the writer's canonical form: the
    /// file re-exports byte-identically, so `update-base` write-backs never
    /// churn it.
    #[test]
    fn plyphon_base_export_is_stable() {
        let text1 = std::str::from_utf8(gantz_plyphon::BASE_BYTES).expect("utf8");
        let base: DataReg =
            gantz_egui::export::parse_export(gantz_plyphon::BASE_BYTES, &super::codec())
                .expect("parse base");
        let text2 =
            gantz_egui::format::to_string_named(&base, &super::codec()).expect("to_string_named");
        assert_eq!(
            text1, text2,
            "the plyphon base file must match the writer's canonical form",
        );
    }

    /// Every graph across ALL base sources compiles in the merged registry -
    /// the registry every app assembles at startup. DSP graphs are
    /// Steel-inert, so they compile like any other graph.
    #[test]
    fn merged_base_sources_all_compile() {
        let mut merged = DataReg::default();
        for bytes in [gantz_base::BYTES, gantz_plyphon::BASE_BYTES] {
            let export: DataReg = gantz_egui::export::parse_export_at(
                bytes,
                bevy_gantz_egui::base::BASE_TIMESTAMP,
                &super::codec(),
            )
            .expect("parse source");
            merged.merge(export);
        }
        let builtins = builtins_with_instances();
        let reified = reify_all(&merged);
        let codec = super::codec();
        let reg_env = env(&merged, &reified, &builtins, &codec);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_env.node(ca);
        let names: Vec<gantz_ca::Name> = merged.heads().map(|(n, _)| n.clone()).collect();
        assert!(names.contains(&name("demo-sine")), "plyphon demo loaded");
        for n in names {
            let head = gantz_ca::Head::Branch(n.clone());
            let graph = head_graph(&reified, &merged, &head)
                .unwrap_or_else(|| panic!("`{n}` has no head graph"));
            let entrypoints = gantz_core::compile::push_pull_entrypoints(&get_node, graph);
            let config = gantz_core::compile::Config::default();
            gantz_core::vm::init(&get_node, graph, &entrypoints, &config).unwrap_or_else(|e| {
                panic!(
                    "merged base graph `{n}` failed to compile:\n{}",
                    gantz_core::vm::error_chain(&e),
                )
            });
        }
    }

    /// The plyphon source parses reproducibly at BASE_TIMESTAMP: startup and
    /// demo-reset parses agree on the demo's commit address (the invariant
    /// reset relies on, per source).
    #[test]
    fn plyphon_base_parses_reproducibly() {
        let parse = || -> DataReg {
            gantz_egui::export::parse_export_at(
                gantz_plyphon::BASE_BYTES,
                bevy_gantz_egui::base::BASE_TIMESTAMP,
                &super::codec(),
            )
            .expect("parse")
        };
        let a = parse();
        let b = parse();
        let ca_a = a.head(&name("demo-sine")).expect("demo-sine");
        let ca_b = b.head(&name("demo-sine")).expect("demo-sine");
        assert_eq!(ca_a, ca_b, "reset must resolve the startup commit address");
    }

    /// A domain base source can reference another source's graphs: the parse
    /// fails unseeded, resolves when seeded with the other source's names,
    /// the merged registry compiles, and the source's own export keeps the
    /// foreign ref by name WITHOUT embedding the foreign graph.
    #[test]
    fn cross_source_base_refs_resolve_via_seed() {
        use std::collections::BTreeMap;
        let ts = bevy_gantz_egui::base::BASE_TIMESTAMP;

        let core: DataReg =
            gantz_egui::export::parse_export_at(gantz_base::BYTES, ts, &super::codec())
                .expect("parse core");
        // Externally-known name -> head graph associations, the form the
        // seeded parse resolves foreign refs through.
        let seed: BTreeMap<String, gantz_ca::GraphAddr> = core
            .heads()
            .filter_map(|(n, ca)| Some((n.to_string(), core.commits().get(&ca)?.graph)))
            .collect();

        // A synthetic domain source wrapping the core `add` graph.
        let text = "\
(graph wrap-add
  (a inlet) (b inlet) (out outlet)
  (add0 (ref add #:sync))
  (-> a (add0 0)) (-> b (add0 1)) (-> add0 out))";

        // Unseeded: the foreign name cannot resolve.
        match gantz_egui::export::parse_export_at(text.as_bytes(), ts, &super::codec()) {
            Err(gantz_egui::export::ParseExportError::Format(e)) => assert!(
                matches!(&e.kind, gantz_format::ErrorKind::MissingDependency(n) if n == "add"),
                "unexpected error kind: {e:?}",
            ),
            Err(other) => panic!("unexpected error: {other:?}"),
            Ok(_) => panic!("must not resolve unseeded"),
        }

        // Seeded with the core source's names: resolves to the core content.
        let domain: DataReg =
            gantz_egui::export::parse_export_seeded_at(text.as_bytes(), ts, &seed, &super::codec())
                .expect("seeded parse");
        let mut merged = core;
        merged.merge(domain);

        // The merged registry compiles the wrapper.
        let builtins = builtins_with_instances();
        let reified = reify_all(&merged);
        let codec = super::codec();
        let reg_env = env(&merged, &reified, &builtins, &codec);
        let get_node = |ca: &gantz_ca::ContentAddr| reg_env.node(ca);
        let head = gantz_ca::Head::Branch(name("wrap-add"));
        let graph = head_graph(&reified, &merged, &head).expect("wrap-add graph");
        let entrypoints = gantz_core::compile::push_pull_entrypoints(&get_node, graph);
        let config = gantz_core::compile::Config::default();
        gantz_core::vm::init(&get_node, graph, &entrypoints, &config).unwrap_or_else(|e| {
            panic!(
                "wrap-add failed to compile:\n{}",
                gantz_core::vm::error_chain(&e),
            )
        });

        // The domain source's own export keeps `add` by name only.
        let out =
            gantz_egui::export::export_names_sexpr_named(&merged, ["wrap-add"], &super::codec())
                .expect("per-source export");
        assert!(out.contains("(graph wrap-add"), "own graph present:\n{out}");
        assert!(out.contains("(ref add"), "foreign ref by name:\n{out}");
        assert!(
            !out.contains("(graph add"),
            "foreign graph must not be embedded:\n{out}",
        );
    }
}
