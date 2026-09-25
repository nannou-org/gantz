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
    let output = match command {
        Command::Check(args) => match read_sources(&args.seed, &args.files) {
            Ok((sources, targets)) => check(&sources, targets),
            Err(diagnostics) => Output {
                diagnostics,
                ..Default::default()
            },
        },
    };
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
/// files. Returns the sources and the index range of the targets.
fn read_sources(
    seed: &SeedArgs,
    files: &[PathBuf],
) -> Result<(Vec<Source>, Range<usize>), Vec<String>> {
    let mut sources = if seed.no_base {
        vec![]
    } else {
        headless::base_sources()
    };
    let mut errors = vec![];
    for path in seed.deps.iter().chain(files) {
        match std::fs::read(path) {
            Ok(bytes) => sources.push(Source {
                label: path.display().to_string(),
                bytes: Cow::Owned(bytes),
            }),
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let targets = sources.len() - files.len()..sources.len();
    Ok((sources, targets))
}

/// Load every source to a fixpoint and reify the merged registry.
///
/// Parse and reify failures are returned as diagnostics. A target that
/// redefines a name an earlier source defined is a warning.
fn load(sources: &[Source], targets: Range<usize>) -> (Ready, Output) {
    let codec = crate::node::codec();
    let loaded = headless::load_sources(sources, BASE_TIMESTAMP, &codec);
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
    let (ready, mut output) = load(sources, targets.clone());
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
