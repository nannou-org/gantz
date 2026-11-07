use crate::conf;
use gantz_cli::headless;

#[test]
fn fmt_check_passes_on_base_files() {
    let conf = conf();
    let sources = headless::base_sources(&conf);
    let output = gantz_cli::fmt(&conf, &sources, 0..sources.len(), true);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(output.writes.is_empty());
}

#[test]
fn check_passes_on_base_sources() {
    let conf = conf();
    let sources = headless::base_sources(&conf);
    let output = gantz_cli::check(&conf, &sources, 0..sources.len());
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(output.warnings.is_empty(), "{:?}", output.warnings);
}

/// No base graph keeps its node state, and no base file stores any.
#[test]
fn base_graphs_keep_no_state() {
    use gantz_core::node::state;
    let conf = conf();
    let sources = headless::base_sources(&conf);
    let ts = gantz_edit::base::BASE_TIMESTAMP;
    let registry = headless::load_sources(&sources, &[], ts, &conf.codec).registry;
    let kept: Vec<String> = registry
        .heads()
        .filter(|(name, _)| state::keeps(&registry, &gantz_ca::Head::Branch((*name).clone())))
        .map(|(name, _)| name.to_string())
        .collect();
    assert!(kept.is_empty(), "base graphs keep their state: {kept:?}");
    let stored = gantz_ca::section_iter::<state::Snapshots>(&registry).count();
    assert_eq!(stored, 0, "base files store node state");
}

/// What a program logs when the node `node` takes `args`, each a Steel expr
/// that a `main!` fires. `extra` holds graphs that the node refers to.
fn call(node: &str, args: &[&str], extra: &str) -> String {
    use std::borrow::Cow;
    let mut text = format!("{extra}\n(graph prog (m main!) (show log) (f {node})");
    for (i, arg) in args.iter().enumerate() {
        text.push_str(&format!(" (a{i} (expr (begin $m {arg})))"));
    }
    for i in 0..args.len() {
        text.push_str(&format!(" (-> m a{i}) (-> a{i} (f {i}))"));
    }
    text.push_str(" (-> f show))");
    let conf = conf();
    let mut sources = headless::base_sources(&conf);
    sources.push(headless::Source {
        label: "prog.gantz".to_string(),
        bytes: Cow::Owned(text.into_bytes()),
    });
    let target = sources.len() - 1;
    let output = gantz_cli::run_graph(&conf, &sources, target, Some("prog"), None, &[]);
    assert!(
        output.diagnostics.is_empty(),
        "{node}: {:?}",
        output.diagnostics
    );
    output.stdout.trim_end().to_string()
}

/// The math graphs, each with the value it logs.
#[test]
fn math_graphs() {
    let rows: &[(&str, &[&str], &str)] = &[
        ("floor", &["2.5"], "2.0"),
        ("ceil", &["2.5"], "3.0"),
        ("round", &["2.5"], "2.0"),
        ("round", &["3.5"], "4.0"),
        ("fract", &["2.25"], "0.25"),
        ("fract", &["-0.25"], "0.75"),
        ("min", &["3", "2"], "2"),
        ("max", &["3", "2"], "3"),
        ("pow", &["2", "10"], "1024"),
        ("pow", &["2", "-1"], "1/2"),
        ("pow", &["3/2", "20"], "3486784401/1048576"),
        ("pow", &["2/3", "-2"], "9/4"),
        ("pow", &["9/4", "0.5"], "1.5"),
        ("quotient", &["7", "2"], "3"),
        ("quotient", &["-7.0", "2"], "-3"),
        ("quotient", &["7", "0.2"], "0"),
        ("lerp", &["10", "20", "0.25"], "12.5"),
        ("ln", &["1"], "0.0"),
        ("ln", &["1/2"], "-0.6931471805599453"),
        ("ln", &["0"], "()"),
        ("ln", &["-1"], "()"),
    ];
    for (graph, args, expected) in rows {
        let got = call(&format!("(ref {graph})"), args, "");
        assert_eq!(got, *expected, "{graph} {args:?}");
    }
}

/// `until` steps until its predicate holds, and no further than its most
/// steps. A predicate or step that is not a function gives the initial value.
#[test]
fn until_graph() {
    let halve = ["(lambda (x) (< x 10))", "(lambda (x) (/ x 2))", "100", "10"];
    assert_eq!(call("(ref until)", &halve, ""), "25/4");
    let count = ["(lambda (x) #f)", "(lambda (x) (+ x 1))", "0", "5"];
    assert_eq!(call("(ref until)", &count, ""), "5");
    let done = ["(lambda (x) #t)", "(lambda (x) (+ x 1))", "0", "5"];
    assert_eq!(call("(ref until)", &done, ""), "0");
    let junk = ["'()", "(lambda (x) (+ x 1))", "7", "5"];
    assert_eq!(call("(ref until)", &junk, ""), "7");
}

/// `get` reads a key of a map, `set` writes one and `literal` emits its
/// datum, exactly.
#[test]
fn map_and_literal_nodes() {
    assert_eq!(call(r#"(get "w")"#, &["(hash 'w 3)"], ""), "3");
    assert_eq!(call(r#"(get "w")"#, &["(hash 'h 3)"], ""), "()");
    assert_eq!(call(r#"(get "w")"#, &["7"], ""), "()");
    let set = r#"(graph setw (m inlet) (v inlet) (o outlet) (s (set "w")) (g (get "w"))
      (-> m s) (-> v (s 1)) (-> s g) (-> g o))"#;
    assert_eq!(call("(ref setw)", &["(hash 'w 1)", "2"], set), "2");
    assert_eq!(call("(ref setw)", &["'()", "2"], set), "2");
    assert_eq!(
        call(r#"(literal "(1 9/8 5/4)")"#, &["'bang"], ""),
        "(1 9/8 5/4)"
    );
    assert_eq!(call(r#"(literal "1/3")"#, &["'bang"], ""), "1/3");
}

/// A `fn` binds the inputs of its graph that are connected, so `map` can
/// call a graph of two inputs with one.
#[test]
fn fn_binds_connected_inputs() {
    let scale = "(graph scale (x inlet) (n inlet) (o outlet) (e (expr (* $x $n)))
      (-> x e) (-> n (e 1)) (-> e o))";
    let prog = format!(
        "{scale}
(graph prog (m main!) (show log)
  (n (expr (begin $m 10))) (f (fn-ref scale)) (xs (expr (begin $m '(1 2 3))))
  (each (ref map))
  (-> m n) (-> m f) (-> m xs) (-> n (f 2)) (-> f each) (-> xs (each 1)) (-> each show))"
    );
    let conf = conf();
    let mut sources = headless::base_sources(&conf);
    sources.push(headless::Source {
        label: "prog.gantz".to_string(),
        bytes: std::borrow::Cow::Owned(prog.into_bytes()),
    });
    let target = sources.len() - 1;
    let output = gantz_cli::run_graph(&conf, &sources, target, Some("prog"), None, &[]);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(output.stdout, "(10 20 30)\n");
}
