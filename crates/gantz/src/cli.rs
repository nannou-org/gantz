//! The `gantz` command line.
//!
//! With no subcommand the binary boots the GUI. The subcommands work on
//! `.gantz` files headlessly, with no window, store or network, so any
//! editor or tool can validate and canonicalize graphs.
//!
//! Each subcommand is a pure core over in-memory [`Source`]s that returns an
//! [`Output`]. [`run`] does the file IO around it and maps the result to an
//! exit code. Names a file does not define resolve through the embedded base
//! sources unless `--no-base` is given, and through any `--dep` files.

use crate::headless::{self, Source};
use bevy_gantz_egui::base::BASE_TIMESTAMP;
use clap::{Args, Parser, Subcommand};
use gantz_egui::export::ParseExportError;
use std::borrow::Cow;
use std::ops::Range;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "gantz",
    version,
    about = "An environment for creative systems."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Rewrite .gantz files to their canonical form.
    ///
    /// The canonical form regenerates node labels, orders forms and drops
    /// comments. A file with commits and names tables keeps them.
    Fmt(FmtArgs),
    /// Parse and compile every named graph in .gantz files.
    ///
    /// Each graph's module is compiled and loaded into a fresh VM exactly as
    /// the app does on open, so its top-level forms are evaluated.
    Check(CheckArgs),
}

/// How names a file does not define are resolved.
#[derive(Args)]
struct SeedArgs {
    /// Do not resolve names through the embedded base sources.
    #[arg(long)]
    no_base: bool,
    /// Extra .gantz files parsed only for the names they define.
    #[arg(long = "dep", value_name = "FILE")]
    deps: Vec<PathBuf>,
}

#[derive(Args)]
pub struct FmtArgs {
    #[command(flatten)]
    seed: SeedArgs,
    /// Report files that are not canonical and write nothing.
    #[arg(long)]
    check: bool,
    /// The .gantz files to rewrite.
    #[arg(required = true)]
    files: Vec<PathBuf>,
}

#[derive(Args)]
pub struct CheckArgs {
    #[command(flatten)]
    seed: SeedArgs,
    /// The .gantz files to check.
    #[arg(required = true)]
    files: Vec<PathBuf>,
}

/// What a subcommand core produced.
#[derive(Default)]
struct Output {
    /// Text for stdout.
    stdout: String,
    /// Lines for stderr. Any means a non-zero exit.
    diagnostics: Vec<String>,
    /// Non-fatal lines for stderr.
    warnings: Vec<String>,
    /// Text to write back to a target, by source index.
    writes: Vec<(usize, String)>,
}

/// A loaded and reified source set, ready to compile.
struct Ready {
    codec: gantz_egui::node::NodeCodec,
    loaded: headless::Loaded,
    reified: headless::Reified,
    builtins: headless::Builtins,
}

impl Ready {
    fn env(&self) -> gantz_egui::Env<'_> {
        headless::env(
            &self.loaded.registry,
            &self.reified,
            &self.builtins,
            &self.codec,
        )
    }
}

/// The subcommand named on the command line, if any.
///
/// Usage errors and `--help` exit here, as clap does.
pub fn parse() -> Option<Command> {
    Cli::parse().command
}

/// Run a subcommand and return the process exit code.
pub fn run(command: Command) -> i32 {
    // Library warnings, such as an unrecognised form a rewrite would drop,
    // must reach the user. There is no Bevy log plugin here.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let (files, mut output) = match command {
        Command::Fmt(args) => {
            let output = with_sources(&args.seed, &args.files, |s, t| fmt(s, t, args.check));
            (args.files, output)
        }
        Command::Check(args) => {
            let output = with_sources(&args.seed, &args.files, check);
            (args.files, output)
        }
    };
    for (ix, text) in std::mem::take(&mut output.writes) {
        if let Err(e) = std::fs::write(&files[ix], text) {
            output
                .diagnostics
                .push(format!("{}: {e}", files[ix].display()));
        }
    }
    print!("{}", output.stdout);
    for line in &output.warnings {
        eprintln!("warning: {line}");
    }
    for line in &output.diagnostics {
        eprintln!("{line}");
    }
    if output.diagnostics.is_empty() { 0 } else { 1 }
}

/// Read the base sources unless omitted, then the deps, then the target
/// files, and run the core over them with the targets' index range.
///
/// Writes in the output are re-keyed from source index to target index.
/// A file that cannot be read is a diagnostic and the core does not run.
fn with_sources(
    seed: &SeedArgs,
    files: &[PathBuf],
    core: impl FnOnce(&[Source], Range<usize>) -> Output,
) -> Output {
    let mut sources = if seed.no_base {
        vec![]
    } else {
        headless::base_sources()
    };
    let mut diagnostics = vec![];
    for path in seed.deps.iter().chain(files) {
        match std::fs::read(path) {
            Ok(bytes) => sources.push(Source {
                label: path.display().to_string(),
                bytes: Cow::Owned(bytes),
            }),
            Err(e) => diagnostics.push(format!("{}: {e}", path.display())),
        }
    }
    if !diagnostics.is_empty() {
        return Output {
            diagnostics,
            ..Default::default()
        };
    }
    let start = sources.len() - files.len();
    let mut output = core(&sources, start..sources.len());
    for (ix, _) in &mut output.writes {
        *ix -= start;
    }
    output
}

/// Parse every source to a fixpoint.
///
/// Parse failures are returned as diagnostics. A target that redefines a
/// name an earlier source defined is a warning.
fn load(
    sources: &[Source],
    targets: Range<usize>,
    codec: &gantz_egui::node::NodeCodec,
) -> (headless::Loaded, Output) {
    let loaded = headless::load_sources(sources, BASE_TIMESTAMP, codec);
    let mut output = Output::default();
    for (source, parsed) in sources.iter().zip(&loaded.parsed) {
        if let Err(e) = parsed {
            output.diagnostics.push(parse_diagnostic(&source.label, e));
        }
    }
    for ix in targets {
        let Ok(parsed) = &loaded.parsed[ix] else {
            continue;
        };
        // Redefining a name with the same content, as checking a base file
        // does, is not a redefinition.
        for (name, ca) in parsed.heads() {
            let earlier = sources[..ix]
                .iter()
                .zip(&loaded.parsed[..ix])
                .filter_map(|(s, p)| p.as_ref().ok().map(|p| (s, p)))
                .find(|(_, p)| p.heads().any(|(n, c)| n == name && c != ca));
            if let Some((earlier, _)) = earlier {
                output.warnings.push(format!(
                    "{}: graph `{name}` redefines a name from {}",
                    sources[ix].label, earlier.label,
                ));
            }
        }
    }
    (loaded, output)
}

/// Load every source and reify the merged registry, ready to compile.
///
/// Reify failures join the parse diagnostics.
fn ready(sources: &[Source], targets: Range<usize>) -> (Ready, Output) {
    let codec = crate::node::codec();
    let (loaded, mut output) = load(sources, targets, &codec);
    let (reified, errs) = headless::reify_all(&loaded.registry, &codec);
    output
        .diagnostics
        .extend(errs.iter().map(|e| gantz_core::vm::error_chain(e)));
    let ready = Ready {
        codec,
        loaded,
        reified,
        builtins: headless::builtins_with_instances(),
    };
    (ready, output)
}

/// Whether the document carries `(commits ...)` or `(names ...)` tables, as
/// GUI exports do, rather than naming its graphs inline.
fn is_address_mode(text: &str) -> bool {
    use gantz_format::sexpr;
    sexpr::read(text).is_ok_and(|forms| {
        forms.iter().any(|form| {
            sexpr::list_args(form)
                .and_then(|args| args.first())
                .and_then(sexpr::as_symbol)
                .is_some_and(|head| head == "commits" || head == "names")
        })
    })
}

/// Rewrite each target to its canonical form, keeping the mode the file is
/// in. Under `check`, targets that differ are diagnostics and nothing is
/// written.
fn fmt(sources: &[Source], targets: Range<usize>, check: bool) -> Output {
    let codec = crate::node::codec();
    let (loaded, mut output) = load(sources, targets.clone(), &codec);
    for ix in targets {
        let Ok(parsed) = &loaded.parsed[ix] else {
            continue;
        };
        let label = &sources[ix].label;
        // The parse succeeded, so the bytes are UTF-8.
        let Ok(text) = std::str::from_utf8(&sources[ix].bytes) else {
            continue;
        };
        let canonical = if is_address_mode(text) {
            gantz_egui::format::to_string(parsed, &codec)
        } else {
            gantz_egui::format::to_string_named(parsed, &codec)
        };
        let canonical = match canonical {
            Ok(canonical) => canonical,
            Err(e) => {
                output.diagnostics.push(format!("{label}: {e}"));
                continue;
            }
        };
        if canonical == text {
            continue;
        }
        if check {
            output.diagnostics.push(format!("{label}: not formatted"));
        } else {
            output.writes.push((ix, canonical));
        }
    }
    output
}

/// Render a parse failure as `label:line:col: message` when the error has a
/// location, else `label: message`.
fn parse_diagnostic(label: &str, err: &ParseExportError) -> String {
    match err {
        ParseExportError::Format(e) => match (e.line, e.col) {
            (Some(line), Some(col)) => format!("{label}:{line}:{col}: {}", e.kind),
            _ => format!("{label}: {}", e.kind),
        },
        ParseExportError::Utf8(_) => format!("{label}: {err}"),
    }
}

/// Render a compile failure for the named graph, one line per diagnostic.
///
/// Steel spans are not rendered. They index the generated module, not the
/// file.
fn compile_diagnostics(
    label: &str,
    name: &gantz_ca::Name,
    err: &gantz_core::vm::CompileError,
) -> Vec<String> {
    let diags = gantz_core::diagnostic::from_compile_error(err);
    if diags.is_empty() {
        let chain = gantz_core::vm::error_chain(err);
        return vec![format!("{label}: graph `{name}`: {chain}")];
    }
    diags
        .iter()
        .map(|d| {
            let node = if d.path.is_empty() {
                String::new()
            } else {
                format!("node {}: ", path_text(&d.path))
            };
            format!("{label}: graph `{name}`: {node}{}", d.message)
        })
        .collect()
}

/// A node path as `/`-joined ids.
fn path_text(path: &[gantz_core::node::Id]) -> String {
    path.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("/")
}

/// Compile every named graph the targets define.
fn check(sources: &[Source], targets: Range<usize>) -> Output {
    let (ready, mut output) = ready(sources, targets.clone());
    let env = ready.env();
    let get_node = |ca: &gantz_ca::ContentAddr| env.node(ca);
    for ix in targets {
        let Ok(parsed) = &ready.loaded.parsed[ix] else {
            continue;
        };
        let label = &sources[ix].label;
        for (name, _) in parsed.heads() {
            let head = gantz_ca::Head::Branch(name.clone());
            let Some(graph) = headless::head_graph(&ready.reified, &ready.loaded.registry, &head)
            else {
                output
                    .diagnostics
                    .push(format!("{label}: graph `{name}`: no head graph"));
                continue;
            };
            if let Err(e) = headless::init(&get_node, graph) {
                output
                    .diagnostics
                    .extend(compile_diagnostics(label, name, &e));
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut sources = headless::base_sources();
        let start = sources.len();
        sources.extend(extra);
        let end = sources.len();
        (sources, start..end)
    }

    #[test]
    fn fmt_check_passes_on_base_files() {
        let sources = headless::base_sources();
        let output = fmt(&sources, 0..sources.len(), true);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        assert!(output.writes.is_empty());
    }

    #[test]
    fn fmt_rewrites_labels_and_is_idempotent() {
        let (sources, targets) = with_base(vec![source(
            "g.gantz",
            "(graph g (m (expr 1)) (n (expr 2)) (-> m n))",
        )]);
        let mut output = fmt(&sources, targets.clone(), false);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        let (ix, text) = output.writes.pop().expect("one rewrite");
        assert_eq!(ix, targets.start);
        assert!(text.contains("(expr0 (expr 1))"), "{text}");
        assert!(!is_address_mode(&text));

        let mut sources = sources;
        sources[ix].bytes = Cow::Owned(text.into_bytes());
        let output = fmt(&sources, targets, true);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    }

    #[test]
    fn fmt_keeps_address_mode() {
        let codec = crate::node::codec();
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
        let output = fmt(&sources, 0..1, true);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    }

    #[test]
    fn check_passes_on_base_sources() {
        let sources = headless::base_sources();
        let output = check(&sources, 0..sources.len());
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        assert!(output.warnings.is_empty(), "{:?}", output.warnings);
    }

    #[test]
    fn check_reports_parse_location() {
        let (sources, targets) = with_base(vec![source("bad.gantz", "(graph g (n bogus))")]);
        let output = check(&sources, targets);
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
        let output = check(&sources, 0..1);
        assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
        assert_eq!(
            output.diagnostics[0],
            "wrap-add.gantz: missing dependency `add`"
        );

        let (sources, targets) = with_base(vec![source("wrap-add.gantz", WRAP_ADD)]);
        let output = check(&sources, targets);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    }

    #[test]
    fn check_warns_on_redefined_name() {
        let (sources, targets) = with_base(vec![source("mine.gantz", "(graph add (a inlet))")]);
        let output = check(&sources, targets);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        assert_eq!(output.warnings.len(), 1, "{:?}", output.warnings);
        assert!(output.warnings[0].contains("`add` redefines"));
    }
}
