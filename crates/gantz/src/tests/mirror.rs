use crate::headless;
use crate::mirror::{Applied, FileState, Mirror, apply_text, filename, render, write_set};
use gantz_ca as ca;
use gantz_egui::base::BASE_TIMESTAMP;
use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

fn name(s: &str) -> ca::Name {
    s.parse().expect("infallible")
}

/// A registry holding the embedded base sources, as a peer starts with.
fn base_registry() -> ca::Registry {
    headless::load_sources(
        &headless::base_sources(&crate::conf()),
        BASE_TIMESTAMP,
        &crate::node::codec(),
    )
    .registry
}

fn apply(registry: &mut ca::Registry, file: &mut FileState, text: &str, secs: u64) -> Applied {
    apply_text(
        registry,
        file,
        text.as_bytes(),
        Duration::from_secs(secs),
        &crate::node::codec(),
    )
    .unwrap_or_else(|e| panic!("apply failed: {e}"))
}

const G1: &str = "\
(graph g
  (b bang)
  (e (expr (begin $push 1)))
  (-> b e))

(layout g
  (b 0 0)
  (e 100 50)
  (camera 0 0 1))";

const G2: &str = "\
(graph g
  (b bang)
  (e (expr (begin $push 2)))
  (-> b e))

(layout g
  (b 0 0)
  (e 100 50)
  (camera 0 0 1))";

#[test]
fn write_set_groups_nested_under_root() {
    let mut registry = base_registry();
    let mut file = FileState::default();
    apply(
        &mut registry,
        &mut file,
        "(graph a (b bang))\n(graph a:b (c bang))\n(graph c (d bang))",
        1,
    );
    let scope: BTreeSet<ca::Name> = [name("a"), name("a:b"), name("c"), name("missing")]
        .into_iter()
        .collect();
    let set = write_set(&registry, &scope);
    assert_eq!(
        set,
        [
            (name("a"), vec![name("a"), name("a:b")]),
            (name("c"), vec![name("c")]),
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(filename(&name("a")), Some("a.gantz".to_string()));
    assert_eq!(filename(&name("../x")), None);
    assert_eq!(filename(&name(".hidden")), None);
}

#[test]
fn apply_text_commits_an_edit_with_the_previous_head_as_parent() {
    let mut registry = base_registry();
    let mut file = FileState::default();
    let first = apply(&mut registry, &mut file, G1, 1);
    assert_eq!(first.committed.len(), 1);
    let (n, c1) = &first.committed[0];
    assert_eq!(*n, name("g"));
    assert_eq!(registry.head(&name("g")), Some(*c1));
    assert!(
        gantz_egui::section::view(&registry, c1).is_some(),
        "layout attached"
    );

    let second = apply(&mut registry, &mut file, G2, 2);
    let (_, c2) = &second.committed[0];
    assert_eq!(registry.commits()[c2].parent, Some(*c1));
    assert!(gantz_egui::section::view(&registry, c2).is_some());
    assert_eq!(file.heads[&name("g")], *c2);
}

/// Re-applying rendered text changes nothing. Nested names render inline
/// in the named format.
#[test]
fn reapplying_rendered_text_is_a_noop() {
    let rows: [(&str, &str, &[&str]); 2] = [
        (
            "nested names",
            "(graph a (b bang))\n(graph a:b (c bang))",
            &["a", "a:b"],
        ),
        ("layout", G1, &["g"]),
    ];
    for (case, src, names) in rows {
        let mut registry = base_registry();
        let mut file = FileState::default();
        apply(&mut registry, &mut file, src, 1);
        let names: Vec<ca::Name> = names.iter().map(|n| name(n)).collect();
        let heads: Vec<_> = names.iter().map(|n| registry.head(n)).collect();
        let text = render(&registry, &names, &crate::node::codec()).unwrap();
        for n in &names {
            assert!(text.contains(&format!("(graph {n}")), "{case}: {text}");
        }
        let applied = apply(&mut registry, &mut file, &text, 2);
        assert!(applied.is_empty(), "{case}: {applied:?}");
        let after: Vec<_> = names.iter().map(|n| registry.head(n)).collect();
        assert_eq!(after, heads, "{case}: heads unchanged");
    }
}

#[test]
fn layout_only_edit_mints_a_layout_commit() {
    let mut registry = base_registry();
    let mut file = FileState::default();
    apply(&mut registry, &mut file, G1, 1);
    let c1 = registry.head(&name("g")).unwrap();
    let moved = G1.replace("(e 100 50)", "(e 200 50)");
    let applied = apply(&mut registry, &mut file, &moved, 2);
    assert!(applied.committed.is_empty());
    assert_eq!(applied.layout_only.len(), 1);
    let c2 = registry.head(&name("g")).unwrap();
    assert_ne!(c1, c2);
    assert_eq!(registry.commits()[&c1].graph, registry.commits()[&c2].graph);
    let view = gantz_egui::section::view(&registry, &c2).unwrap();
    assert!(view.layout.iter().any(|(_, p)| p.x == 200.0), "{view:?}");
}

#[test]
fn non_sync_pins_survive_a_round_trip_and_sync_refs_follow() {
    let mut registry = base_registry();
    let add_before = gantz_egui::reg::head_graph_addr(&registry, &name("add")).unwrap();
    let mut pinned = FileState::default();
    apply(
        &mut registry,
        &mut pinned,
        "(graph pinned (a inlet) (b inlet) (r (ref add)) (-> a (r 0)) (-> b (r 1)))",
        1,
    );
    let mut following = FileState::default();
    apply(
        &mut registry,
        &mut following,
        "(graph following (a inlet) (b inlet) (r (ref add #:sync)) (-> a (r 0)) (-> b (r 1)))",
        2,
    );
    assert_eq!(pinned.pins.get("add"), Some(&add_before));
    assert!(following.pins.is_empty());

    // Move `add` on. The sync referrer follows on the resync.
    let mut add_file = FileState::default();
    let applied = apply(
        &mut registry,
        &mut add_file,
        "(graph add (a inlet) (b inlet) (out outlet) (e (expr (+ $a $b 0))) (-> a (e 0)) (-> b (e 1)) (-> e out))",
        3,
    );
    let add_after = gantz_egui::reg::head_graph_addr(&registry, &name("add")).unwrap();
    assert_ne!(add_before, add_after);
    assert!(
        applied.moved.iter().any(|m| m.name == name("following")),
        "{applied:?}"
    );
    assert!(!applied.moved.iter().any(|m| m.name == name("pinned")));

    // Re-reading the pinned file keeps its pin. Re-reading the following
    // file resolves to the new `add`.
    let text = render(&registry, &[name("pinned")], &crate::node::codec()).unwrap();
    let applied = apply(&mut registry, &mut pinned, &text, 4);
    assert!(applied.is_empty(), "{applied:?}");
    let text = render(&registry, &[name("following")], &crate::node::codec()).unwrap();
    let applied = apply(&mut registry, &mut following, &text, 5);
    assert!(applied.is_empty(), "{applied:?}");
    let graph = registry
        .head_graph(&ca::Head::Branch(name("following")))
        .unwrap();
    let refs: Vec<_> = gantz_egui::sync::named_refs(graph).collect();
    assert_eq!(refs, vec![(name("add"), add_after, true)]);
}

#[test]
fn mirror_writes_reads_and_ignores_own_writes() {
    let dir = std::env::temp_dir().join(format!(
        "gantz-mirror-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut registry = base_registry();
    let mut seed_file = FileState::default();
    apply(&mut registry, &mut seed_file, G1, 1);
    let scope: BTreeSet<ca::Name> = [name("g")].into_iter().collect();
    let mut mirror = Mirror::new(dir.clone(), crate::node::codec());

    let written = mirror.write_all(&registry, &scope).unwrap();
    assert_eq!(written, vec![dir.join("g.gantz")]);
    assert!(
        mirror.write_all(&registry, &scope).unwrap().is_empty(),
        "unchanged"
    );
    // Own write, then no change: nothing to read, even after settling.
    for _ in 0..3 {
        let results = mirror
            .poll(&mut registry, Duration::from_secs(2), false)
            .unwrap();
        assert!(results.is_empty(), "{results:?}");
    }

    // An edit is read once it has held still for one poll.
    let path = dir.join("g.gantz");
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("$push 1", "$push 3");
    std::fs::write(&path, &edited).unwrap();
    let head = registry.head(&name("g")).unwrap();
    assert!(
        mirror
            .poll(&mut registry, Duration::from_secs(3), false)
            .unwrap()
            .is_empty()
    );
    let results = mirror
        .poll(&mut registry, Duration::from_secs(3), false)
        .unwrap();
    assert_eq!(results.len(), 1);
    let (p, applied) = &results[0];
    assert_eq!(p, &path);
    assert_eq!(applied.as_ref().unwrap().committed.len(), 1);
    assert_ne!(registry.head(&name("g")), Some(head));
    // The registry now agrees with the file, so nothing is rewritten.
    assert!(mirror.write_all(&registry, &scope).unwrap().is_empty());

    // A broken file is reported and left alone.
    std::fs::write(&path, "(graph g (b bogus))").unwrap();
    mirror
        .poll(&mut registry, Duration::from_secs(4), false)
        .unwrap();
    let results = mirror
        .poll(&mut registry, Duration::from_secs(4), false)
        .unwrap();
    assert!(matches!(&results[..], [(_, Err(_))]), "{results:?}");
    assert!(mirror.write_all(&registry, &scope).unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "(graph g (b bogus))"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

/// A file defining a root with nested graphs and a base reference. Every
/// reference resolves to the graph at its name's head, the session scope
/// covers all of them, and the served closure holds their graphs.
#[test]
fn nested_refs_are_in_scope_and_served() {
    let mut registry = base_registry();
    let mut file = FileState::default();
    apply(
        &mut registry,
        &mut file,
        "\
(graph root
  (number0 number)
  (add1 (ref add #:sync))
  (half2 (ref root:half #:sync))
  (gain3 (ref root:gain #:sync))
  (number4 number)
  (inspect5 inspect)
  (-> number0 add1) (-> number0 (add1 1)) (-> add1 half2)
  (-> half2 gain3) (-> number4 (gain3 1)) (-> gain3 inspect5))
(graph root:half
  (inlet0 (inlet \"number\" \"value\"))
  (expr1 (expr (/ $v 2)))
  (outlet2 (outlet \"number\" \"half\"))
  (-> inlet0 expr1) (-> expr1 outlet2))
(graph root:gain
  (inlet0 (inlet \"number\" \"value\"))
  (inlet1 (inlet \"number\" \"gain\"))
  (expr2 (expr (* $v $g)))
  (outlet3 (outlet \"number\" \"scaled\"))
  (-> inlet0 expr2) (-> inlet1 (expr2 1)) (-> expr2 outlet3))",
        1,
    );
    let root = name("root");
    let graph = registry
        .head_graph(&ca::Head::Branch(root.clone()))
        .unwrap();
    let refs: Vec<_> = gantz_egui::sync::named_refs(graph).collect();
    assert_eq!(refs.len(), 3);
    for (n, ga, sync) in &refs {
        assert!(sync);
        assert_eq!(gantz_egui::reg::head_graph_addr(&registry, n), Some(*ga));
    }
    let scope = gantz_egui::sync::session_scope(&registry, &root);
    let expected: BTreeSet<ca::Name> = ["add", "root", "root:gain", "root:half"]
        .into_iter()
        .map(name)
        .collect();
    assert_eq!(scope, expected);
    let live = ca::closure_from(&registry, scope.iter().filter_map(|n| registry.head(n)));
    for (_, ga, _) in &refs {
        assert!(live.graphs.contains(ga));
    }
}
