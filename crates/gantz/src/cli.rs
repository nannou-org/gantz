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
use clap::{Args, Parser, Subcommand, ValueEnum};
use gantz_egui::base::BASE_TIMESTAMP;
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
    /// Print the Steel module compiled from one named graph.
    Compile(CompileArgs),
    /// Join a collaborative session and mirror its graphs to a directory.
    ///
    /// Each top-level graph in the session becomes a .gantz file named after
    /// it in the directory, holding it and its nested graphs. Saved edits
    /// become commits announced to the session, and remote changes rewrite
    /// the files. Blocks until interrupted. Nothing keeps running after it
    /// exits.
    #[cfg(feature = "collab")]
    Join(JoinArgs),
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

#[cfg(feature = "collab")]
#[derive(Args)]
pub struct JoinArgs {
    /// The session invite ticket.
    pub ticket: String,
    /// The directory to mirror the session into, created if absent. Defaults
    /// to a directory named after the session under the app data directory.
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// The peer identity file: 32 secret key bytes, created if absent. Gives
    /// the peer a stable id across runs. Without it the identity is new
    /// each run.
    #[arg(long, value_name = "FILE")]
    pub identity: Option<PathBuf>,
    /// A self-hosted relay URL instead of the default infrastructure.
    #[arg(long, value_name = "URL")]
    pub relay: Option<String>,
}

#[derive(Args)]
pub struct CompileArgs {
    #[command(flatten)]
    seed: SeedArgs,
    /// The .gantz file defining the graph.
    file: PathBuf,
    /// The graph to compile. Defaults to the file's unique root graph, the
    /// one no other graph in the file references.
    #[arg(long, value_name = "NAME")]
    graph: Option<String>,
    /// What to print.
    #[arg(long, value_enum, default_value_t = Emit::Steel)]
    emit: Emit,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Emit {
    /// The Steel module text.
    Steel,
    /// The source map: one line per definition and identifier occurrence,
    /// with line:col positions into the Steel module text and the node path
    /// each refers to.
    SourceMap,
}

/// What a subcommand core produced.
#[derive(Default)]
pub(crate) struct Output {
    /// Text for stdout.
    pub(crate) stdout: String,
    /// Lines for stderr. Any means a non-zero exit.
    pub(crate) diagnostics: Vec<String>,
    /// Non-fatal lines for stderr.
    pub(crate) warnings: Vec<String>,
    /// Text to write back to a target, by source index.
    pub(crate) writes: Vec<(usize, String)>,
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
    // must reach the user. Libraries log through `log` and `tracing`, and the
    // subscriber bridges both. `RUST_LOG` overrides the default.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,gantz=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .with_writer(std::io::stderr)
        .init();
    let (files, mut output) = match command {
        #[cfg(feature = "collab")]
        Command::Join(args) => return crate::join::run(args),
        Command::Fmt(args) => {
            let output = with_sources(&args.seed, &args.files, |s, t| fmt(s, t, args.check));
            (args.files, output)
        }
        Command::Check(args) => {
            let output = with_sources(&args.seed, &args.files, check);
            (args.files, output)
        }
        Command::Compile(args) => {
            let files = vec![args.file];
            let output = with_sources(&args.seed, &files, |s, t| {
                compile(s, t.start, args.graph.as_deref(), args.emit)
            });
            (files, output)
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
pub(crate) fn is_address_mode(text: &str) -> bool {
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
pub(crate) fn fmt(sources: &[Source], targets: Range<usize>, check: bool) -> Output {
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
pub(crate) fn parse_diagnostic(label: &str, err: &ParseExportError) -> String {
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
pub(crate) fn check(sources: &[Source], targets: Range<usize>) -> Output {
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

/// Compile the graph `name` names, or the target's unique root graph, and
/// emit the module text or its source map.
pub(crate) fn compile(sources: &[Source], target: usize, name: Option<&str>, emit: Emit) -> Output {
    let (ready, mut output) = ready(sources, target..target + 1);
    let label = &sources[target].label;
    let Ok(parsed) = &ready.loaded.parsed[target] else {
        return output;
    };
    let name = match name {
        Some(name) => {
            let name: gantz_ca::Name = name.parse().expect("infallible");
            if !parsed.heads().any(|(n, _)| *n == name) {
                output
                    .diagnostics
                    .push(format!("{label}: no graph named `{name}`"));
                return output;
            }
            name
        }
        None => match gantz_egui::export::unique_root_name(parsed) {
            Some(name) => name,
            None => {
                let names: Vec<String> = parsed.heads().map(|(n, _)| n.to_string()).collect();
                output.diagnostics.push(format!(
                    "{label}: no unique root graph, pass --graph <NAME>. Graphs: {}",
                    names.join(", "),
                ));
                return output;
            }
        },
    };
    let env = ready.env();
    let get_node = |ca: &gantz_ca::ContentAddr| env.node(ca);
    let head = gantz_ca::Head::Branch(name.clone());
    let Some(graph) = headless::head_graph(&ready.reified, &ready.loaded.registry, &head) else {
        output
            .diagnostics
            .push(format!("{label}: graph `{name}`: no head graph"));
        return output;
    };
    let entrypoints = bevy_gantz_egui::entrypoints(&get_node, graph);
    let config = gantz_core::compile::Config::default();
    let exprs = match gantz_core::compile::module(&get_node, graph, &entrypoints, &config) {
        Ok(exprs) => exprs,
        Err(e) => {
            let e = gantz_core::vm::CompileError::Module(e);
            output
                .diagnostics
                .extend(compile_diagnostics(label, &name, &e));
            return output;
        }
    };
    let mut src = gantz_core::vm::fmt_module(&exprs);
    if !src.ends_with('\n') {
        src.push('\n');
    }
    output.stdout = match emit {
        Emit::Steel => src,
        Emit::SourceMap => source_map_text(&src),
    };
    output
}

/// The source map as text. One `def` line per top-level form and one `ref`
/// line per identifier occurrence, each with its `line:col-line:col` range
/// into `src` and the `/`-joined path of the node it refers to, or `-`.
fn source_map_text(src: &str) -> String {
    let map = gantz_core::compile::SourceMap::parse(src);
    let pos = |range: &Range<usize>| {
        let (l1, c1) = gantz_format::line_col(src, range.start);
        let (l2, c2) = gantz_format::line_col(src, range.end);
        format!("{l1}:{c1}-{l2}:{c2}")
    };
    let path = |range: &Range<usize>| match map.node_at(range.clone()) {
        Some(path) if !path.is_empty() => path_text(&path),
        _ => "-".to_string(),
    };
    let defs = map
        .defs()
        .iter()
        .map(|d| format!("def {} {}\n", pos(&d.range), path(&d.range)));
    let refs = map
        .occs()
        .iter()
        .map(|o| format!("ref {} {}\n", pos(&o.range), path(&o.range)));
    defs.chain(refs).collect()
}
