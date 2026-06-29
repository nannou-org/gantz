use dyn_clone::DynClone;
use dyn_hash::DynHash;
use std::any::Any;

/// A top-level blanket trait providing trait object cloning, hashing, and serialization.
#[typetag::serde(tag = "type")]
pub trait Node:
    Any + DynClone + DynHash + gantz_ca::CaHash + gantz_core::Node + gantz_egui::NodeUi + Send + Sync
{
}

dyn_clone::clone_trait_object!(Node);
dyn_hash::hash_trait_object!(Node);

#[typetag::serde]
impl Node for gantz_core::node::Apply {}
#[typetag::serde]
impl Node for gantz_core::node::Branch {}
#[typetag::serde]
impl Node for gantz_core::node::Delay {}
#[typetag::serde]
impl Node for gantz_core::node::Expr {}
#[typetag::serde]
impl Node for gantz_core::node::Identity {}
#[typetag::serde]
impl Node for gantz_core::node::graph::Inlet {}
#[typetag::serde]
impl Node for gantz_core::node::graph::Outlet {}

#[typetag::serde]
impl Node for gantz_std::Bang {}
#[typetag::serde]
impl Node for gantz_std::Log {}
#[typetag::serde]
impl Node for gantz_std::Number {}

#[typetag::serde]
impl Node for gantz_egui::node::FnNamedRef {}
#[typetag::serde]
impl Node for gantz_egui::node::NamedRef {}

#[typetag::serde]
impl Node for gantz_egui::node::Comment {}
#[typetag::serde]
impl Node for bevy_gantz_egui::node::UpdateBang {}
#[typetag::serde]
impl Node for bevy_gantz_egui::node::TickBang {}
#[typetag::serde]
impl Node for gantz_egui::node::Inspect {}
#[typetag::serde]
impl Node for gantz_egui::node::Plot {}

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

#[typetag::serde]
impl Node for Box<dyn Node> {}

#[cfg(test)]
mod tests {
    use super::Node;

    /// Gate test for the `.gantz` text format: confirm `Box<dyn Node>`
    /// (typetag-dispatched) round-trips through the self-describing
    /// `gantz_format::Datum` codec. The format bridges node specs to/from
    /// typetag via this codec rather than hand-writing a parser per node type,
    /// so the mechanism must hold for every registered node.
    #[test]
    fn typetag_roundtrips_through_datum() {
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
        ];
        for value in cases {
            let node: Box<dyn Node> = from_datum(value.clone())
                .unwrap_or_else(|e| panic!("from_datum failed for {value:?}: {e}"));
            let back = to_datum(&node).unwrap_or_else(|e| panic!("to_datum failed: {e}"));
            // The re-serialized form must itself round-trip identically, proving
            // both directions of the typetag <-> Datum bridge are stable.
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
        let mut required = HashSet::new();
        let mut ca = demo_commit;
        loop {
            required.insert(ca);
            match reparse.registry.commits().get(&ca).and_then(|c| c.parent) {
                Some(parent) => ca = parent,
                None => break,
            }
        }
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
