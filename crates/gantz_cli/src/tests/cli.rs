use super::conf;
use crate::headless::{self, Source};
use crate::{Emit, check, compile, fmt, is_address_mode, list, run_graph};
use gantz_egui::base::BASE_TIMESTAMP;
use std::borrow::Cow;
use std::ops::Range;

/// A root graph with a push entrypoint, so its module has definitions.
const ROOT: &str = "\
(graph root
  (b bang)
  (e (expr (begin $push 42)))
  (-> b e))";

/// A domain-style source wrapping the core `add` graph.
const WRAP_ADD: &str = "\
(graph wrap-add
  (a inlet) (b inlet) (out outlet)
  (add0 (ref add #:sync))
  (-> a (add0 0)) (-> b (add0 1)) (-> add0 out))";

fn source(label: &str, text: &'static str) -> Source {
    Source {
        label: label.to_string(),
        bytes: Cow::Borrowed(text.as_bytes()),
    }
}

/// The base sources followed by `extra`, with the target range covering
/// only `extra`.
fn with_base(extra: Vec<Source>) -> (Vec<Source>, Range<usize>) {
    let mut sources = headless::base_sources(&conf());
    let start = sources.len();
    sources.extend(extra);
    let end = sources.len();
    (sources, start..end)
}

#[test]
fn fmt_rewrites_labels_and_is_idempotent() {
    let (sources, targets) = with_base(vec![source(
        "g.gantz",
        "(graph g (m (expr 1)) (n (expr 2)) (-> m n))",
    )]);
    let mut output = fmt(&conf(), &sources, targets.clone(), false);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    let (ix, text) = output.writes.pop().expect("one rewrite");
    assert_eq!(ix, targets.start);
    assert!(text.contains("(expr0 (expr 1))"), "{text}");
    assert!(!is_address_mode(&text));

    let mut sources = sources;
    sources[ix].bytes = Cow::Owned(text.into_bytes());
    let output = fmt(&conf(), &sources, targets, true);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
}

#[test]
fn fmt_keeps_address_mode() {
    let codec = conf().codec;
    let base = gantz_egui::export::parse_export_at(gantz_base::BYTES, BASE_TIMESTAMP, &codec)
        .expect("parse base");
    let heads: Vec<_> = base
        .heads()
        .map(|(n, _)| gantz_ca::Head::Branch(n.clone()))
        .collect();
    let text = gantz_egui::export::export_heads_sexpr(&base, &heads, &codec).expect("export");
    assert!(is_address_mode(&text));

    let sources = vec![Source {
        label: "export.gantz".to_string(),
        bytes: Cow::Owned(text.into_bytes()),
    }];
    let output = fmt(&conf(), &sources, 0..1, true);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
}

#[test]
fn compile_emits_steel_for_root_graph() {
    let (sources, targets) = with_base(vec![source("root.gantz", ROOT)]);
    let output = compile(&conf(), &sources, targets.start, None, Emit::Steel);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(output.stdout.contains("(define"), "{}", output.stdout);
    assert!(output.stdout.ends_with('\n'));
}

#[test]
fn compile_rejects_ambiguous_root() {
    let sources = headless::base_sources(&conf());
    let output = compile(&conf(), &sources, 0, None, Emit::Steel);
    assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
    assert!(output.diagnostics[0].contains("no unique root graph"));
    assert!(output.stdout.is_empty());
}

#[test]
fn compile_by_name_and_source_map() {
    let (sources, targets) = with_base(vec![source("root.gantz", ROOT)]);
    let output = compile(
        &conf(),
        &sources,
        targets.start,
        Some("root"),
        Emit::SourceMap,
    );
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(
        output.stdout.lines().any(|l| l.starts_with("def ")),
        "{}",
        output.stdout
    );

    let output = compile(&conf(), &sources, targets.start, Some("nope"), Emit::Steel);
    assert_eq!(output.diagnostics, ["root.gantz: no graph named `nope`"]);
}

#[test]
fn check_reports_parse_location() {
    let (sources, targets) = with_base(vec![source("bad.gantz", "(graph g (n bogus))")]);
    let output = check(&conf(), &sources, targets);
    assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
    assert!(
        output.diagnostics[0].starts_with("bad.gantz:1:"),
        "{}",
        output.diagnostics[0]
    );
}

#[test]
fn check_no_base_reports_missing_dependency() {
    let sources = vec![source("wrap-add.gantz", WRAP_ADD)];
    let output = check(&conf(), &sources, 0..1);
    assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
    assert_eq!(
        output.diagnostics[0],
        "wrap-add.gantz: missing dependency `add`"
    );

    let (sources, targets) = with_base(vec![source("wrap-add.gantz", WRAP_ADD)]);
    let output = check(&conf(), &sources, targets);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
}

#[test]
fn check_warns_on_redefined_name() {
    let (sources, targets) = with_base(vec![source("mine.gantz", "(graph add (a inlet))")]);
    let output = check(&conf(), &sources, targets);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(output.warnings.len(), 1, "{:?}", output.warnings);
    assert!(output.warnings[0].contains("`add` redefines"));
}

/// A program that logs its arguments, with a push source at the root and
/// one in a nested graph. The nested `main!` is not an entry.
const PROG: &str = "\
(graph inner
  (m main!) (b bang) (l log)
  (-> m l) (-> b l))

(graph prog
  (args main!) (show log)
  (go bang) (answer (expr (begin $go 42))) (say log)
  (i (ref inner))
  (-> args show) (-> go answer) (-> answer say))";

fn run_prog(push: Option<&str>, args: &[&str]) -> crate::Output {
    let (sources, targets) = with_base(vec![source("prog.gantz", PROG)]);
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    run_graph(&conf(), &sources, targets.start, None, push, &args)
}

#[test]
fn run_fires_root_main_with_args() {
    let output = run_prog(None, &["a", "b c"]);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(output.stdout, "(\"a\" \"b c\")\n");

    let output = run_prog(None, &[]);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(output.stdout, "()\n");
}

#[test]
fn run_push_by_label_or_path() {
    for node in ["go", "2"] {
        let output = run_prog(Some(node), &[]);
        assert!(
            output.diagnostics.is_empty(),
            "{node}: {:?}",
            output.diagnostics
        );
        assert_eq!(output.stdout, "42\n", "{node}");
    }
    let output = run_prog(Some("5/1"), &[]);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(output.stdout, "()\n");

    let output = run_prog(Some("show"), &[]);
    assert_eq!(
        output.diagnostics,
        ["prog.gantz: graph `prog`: no push source `show`. Push sources: go, 5/1"]
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn run_without_main_names_push_sources() {
    let (sources, targets) = with_base(vec![source("root.gantz", ROOT)]);
    let output = run_graph(&conf(), &sources, targets.start, None, None, &[]);
    assert_eq!(
        output.diagnostics,
        ["root.gantz: graph `root`: no main! node, pass --push <NODE>. Push sources: b"]
    );
}

#[test]
fn run_reports_runtime_error_at_node() {
    let text = "(graph boom (m main!) (e (expr (car $m))) (-> m e))";
    let (sources, targets) = with_base(vec![source("boom.gantz", text)]);
    let output = run_graph(&conf(), &sources, targets.start, None, None, &[]);
    assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
    assert!(
        output.diagnostics[0].starts_with("boom.gantz: graph `boom`: node 1: "),
        "{}",
        output.diagnostics[0]
    );
}

#[test]
fn list_prints_main_and_push_sources() {
    let (sources, targets) = with_base(vec![source("prog.gantz", PROG)]);
    let output = list(&conf(), &sources, targets.start, None);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(output.stdout, "main! 0 args\npush 2 go\npush 5/1 -\n");
}
