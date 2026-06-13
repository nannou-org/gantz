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
impl Node for gantz_core::node::GraphNode<Box<dyn Node>> {}
#[typetag::serde]
impl Node for gantz_core::node::Identity {}
#[typetag::serde]
impl Node for gantz_core::node::graph::Inlet {}
#[typetag::serde]
impl Node for gantz_core::node::graph::Outlet {}

#[typetag::serde]
impl Node for gantz_std::ops::Add {}
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
impl Node for bevy_gantz_egui::node::FrameBang {}
#[typetag::serde]
impl Node for gantz_egui::node::Inspect {}

impl From<gantz_egui::node::NamedRef> for Box<dyn Node> {
    fn from(named: gantz_egui::node::NamedRef) -> Self {
        Box::new(named)
    }
}

impl bevy_gantz_egui::node::ToFrameBang for Box<dyn Node> {
    fn to_frame_bang(&self) -> Option<&bevy_gantz_egui::node::FrameBang> {
        let any: &dyn std::any::Any = &**self;
        any.downcast_ref()
    }
}

#[typetag::serde]
impl Node for Box<dyn Node> {}

// To allow for navigating between nested graphs in a graph scene, we need to be
// able to downcast a node to a graph node.
impl gantz_egui::widget::graph_scene::ToGraphMut for Box<dyn Node> {
    type Node = Self;
    fn to_graph_mut(&mut self) -> Option<&mut gantz_core::node::graph::Graph<Self::Node>> {
        ((&mut **self) as &mut dyn Any)
            .downcast_mut::<gantz_core::node::GraphNode<Self::Node>>()
            .map(|node| &mut node.graph)
    }
}

#[cfg(test)]
mod tests {
    use super::Node;

    /// Gate test for the new `.gantz` text format: confirm `Box<dyn Node>`
    /// (typetag-dispatched) round-trips through a self-describing
    /// `serde_json::Value`. The format bridges node specs to/from typetag via
    /// `serde_json::Value` rather than hand-writing a parser per node type, so
    /// this mechanism must hold for every registered node.
    #[test]
    fn typetag_roundtrips_through_serde_json_value() {
        let cases = [
            serde_json::json!({ "type": "Inlet" }),
            serde_json::json!({ "type": "Outlet" }),
            serde_json::json!({ "type": "Apply" }),
            serde_json::json!({ "type": "Delay" }),
            serde_json::json!({ "type": "Identity" }),
            serde_json::json!({ "type": "Bang" }),
            serde_json::json!({ "type": "Add" }),
            serde_json::json!({ "type": "Inspect" }),
            serde_json::json!({ "type": "FrameBang" }),
            serde_json::json!({ "type": "Number" }),
            serde_json::json!({ "type": "Expr", "src": "(* $l $r)" }),
            serde_json::json!({ "type": "Comment", "text": "hi", "size": [100, 40] }),
            serde_json::json!({ "type": "Branch", "src": "(if $x (list 0 0) (list 1 0))", "branches": ["10", "01"] }),
            serde_json::json!({
                "type": "NamedRef",
                "ref_": "0000000000000000000000000000000000000000000000000000000000000000",
                "name": "mul",
            }),
        ];
        for value in cases {
            let node: Box<dyn Node> = serde_json::from_value(value.clone())
                .unwrap_or_else(|e| panic!("from_value failed for {value}: {e}"));
            let back =
                serde_json::to_value(&node).unwrap_or_else(|e| panic!("to_value failed: {e}"));
            // The re-serialized form must itself round-trip identically, proving
            // both directions of the typetag <-> Value bridge are stable.
            let node2: Box<dyn Node> = serde_json::from_value(back.clone())
                .unwrap_or_else(|e| panic!("re-deserialize failed for {back}: {e}"));
            let back2 =
                serde_json::to_value(&node2).unwrap_or_else(|e| panic!("re-serialize failed: {e}"));
            assert_eq!(back, back2, "round-trip not stable for {value}");
            assert_eq!(
                back.get("type").and_then(|t| t.as_str()),
                value.get("type").and_then(|t| t.as_str()),
                "type tag changed for {value}",
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
  (l inlet) (r inlet) (out outlet)
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

    /// A nested `(graph ...)` node (a `GraphNode`) round-trips: the outer graph
    /// address is preserved through text -> Export -> text -> Export.
    #[test]
    fn nested_graph_roundtrips() {
        use std::time::Duration;
        type G = gantz_core::node::graph::Graph<Box<dyn Node>>;

        let now = Duration::from_secs(42);
        let text1 = "\
(graph env
  (in inlet) (out outlet)
  (sub (graph
         (i inlet) (o outlet)
         (e (expr (+ $x 1)))
         (-> i (e 0)) (-> e o)))
  (-> in (sub 0)) (-> sub out))";
        let e1: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(text1, now).expect("from_str 1");
        let text2 = gantz_egui::format::to_string(&e1).expect("to_string");
        let e2: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(&text2, now).expect("from_str 2");

        let head1 = gantz_ca::Head::Branch("env".to_string());
        let g1 = e1.registry.head_graph(&head1).expect("g1");
        let g2 = e2.registry.head_graph(&head1).expect("g2");
        assert_eq!(
            gantz_ca::graph_addr(g1),
            gantz_ca::graph_addr(g2),
            "nested graph addr must survive round-trip\n--- text2 ---\n{text2}",
        );
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
    /// `(layout ...)` view state: node positions and the scene rect survive
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
  (scene -50 -50 100 100))";

        let e1: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(text1, now).expect("from_str 1");
        let head = *e1.registry.names().get("mul").expect("mul name");
        let view = e1
            .views
            .get(&head)
            .and_then(|gv| gv.get(&Vec::new()))
            .expect("view");
        // `m` is node index 0, `l` is 1.
        assert_eq!(
            view.layout.get(&egui_graph::NodeId(0)).map(|p| (p.x, p.y)),
            Some((-10.0, 20.0))
        );
        assert_eq!(
            view.layout.get(&egui_graph::NodeId(1)).map(|p| (p.x, p.y)),
            Some((3.5, -4.5))
        );
        assert_eq!(view.scene_rect.min.x, -50.0);
        assert_eq!(view.scene_rect.max.y, 100.0);

        let text2 = gantz_egui::format::to_string(&e1).expect("to_string");
        let e2: gantz_egui::export::Export<G> =
            gantz_egui::format::from_str(&text2, now).expect("from_str 2");
        let head2 = *e2.registry.names().get("mul").expect("mul name 2");
        let view2 = e2
            .views
            .get(&head2)
            .and_then(|gv| gv.get(&Vec::new()))
            .expect("view 2");
        assert_eq!(view.layout.len(), view2.layout.len());
        assert_eq!(
            view2.layout.get(&egui_graph::NodeId(0)).map(|p| (p.x, p.y)),
            Some((-10.0, 20.0))
        );
        assert_eq!(view2.scene_rect, view.scene_rect);
    }
}
