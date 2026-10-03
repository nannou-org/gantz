//! The gantz command line, as a library.
//!
//! The subcommands work on `.gantz` files headlessly, with no window, store
//! or network, so any editor or tool can validate, canonicalize and run
//! graphs. An app built on gantz passes its node set in a [`Conf`], so the
//! subcommands parse and compile exactly as the app does.
//!
//! Each subcommand is a pure core over in-memory [`Source`]s that returns an
//! [`Output`]. [`run`] does the file IO around it and maps the result to an
//! exit code. Names a file does not define resolve through the configured
//! base sources unless `--no-base` is given, and through any `--dep` files.
//!
//! An app puts [`Command`] in its own parser, so `--version` reports the
//! app's version:
//!
//! ```no_run
//! use clap::Parser;
//!
//! #[derive(Parser)]
//! #[command(version, about)]
//! struct Cli {
//!     #[command(subcommand)]
//!     command: Option<gantz_cli::Command>,
//! }
//!
//! fn conf() -> gantz_cli::Conf {
//!     todo!("the app's node set")
//! }
//!
//! fn main() {
//!     if let Some(command) = Cli::parse().command {
//!         std::process::exit(gantz_cli::run(command, &conf()));
//!     }
//!     // With no subcommand, start the app.
//! }
//! ```
//!
//! To add its own subcommands, an app puts `#[command(flatten)]` on a
//! [`Command`] variant of its own `clap::Subcommand` enum.

use crate::headless::Source;
use clap::{Args, Subcommand, ValueEnum};
use gantz_egui::base::BASE_TIMESTAMP;
use gantz_egui::export::ParseExportError;
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;

pub mod headless;
#[cfg(feature = "collab")]
mod join;
#[cfg(feature = "collab")]
mod mirror;
#[cfg(test)]
mod tests;
#[cfg(feature = "collab")]
mod vault;

/// The CLI configuration: the node set to parse and compile with, the names
/// that locate the default data directories, and the build.
pub struct Conf {
    /// The node set's codec. Its sugar reads and writes `.gantz` text.
    pub codec: gantz_egui::node::NodeCodec,
    /// The builtin node palette.
    pub builtins: gantz_core::Builtins,
    /// The Steel modules that every graph's VM registers beyond the core set.
    pub steel_modules: Vec<gantz_core::vm::SteelModule>,
    /// The embedded base sources, in load order.
    pub base_sources: Vec<gantz_egui::base::BaseSource>,
    /// Every entrypoint to compile for a graph.
    pub entrypoints: fn(
        gantz_core::node::GetNode<'_>,
        &gantz_core::node::graph::Graph<gantz_egui::node::DynNode>,
    ) -> Vec<gantz_core::compile::Entrypoint>,
    /// The organisation name that locates the default data directories.
    pub org: &'static str,
    /// The app name that locates the default data directories.
    pub app: &'static str,
    /// This build: the app and its version, such as `gantz 0.4.0`. Peers see
    /// it for display.
    pub build: &'static str,
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
    /// Run one named graph and print the values its log nodes emit.
    ///
    /// Fires the main! nodes at the graph's root together, or one push
    /// source with --push. Each log node firing prints its value on one line
    /// of stdout, in firing order. The graph compiles as the app compiles it.
    /// Nothing runs after the entry returns, so update!, tick!, audio and
    /// await do not run.
    Run(RunArgs),
    /// Join a collaborative session and mirror its graphs to a directory.
    ///
    /// Each top-level graph in the session becomes a .gantz file named after
    /// it in the directory, holding it and its nested graphs. Saved edits
    /// become commits announced to the session, and remote changes rewrite
    /// the files. Blocks until interrupted. Nothing keeps running after it
    /// exits.
    #[cfg(feature = "collab")]
    Join(JoinArgs),
    /// Run or manage a vault that syncs all your named graphs between your
    /// devices.
    #[cfg(feature = "collab")]
    Vault(VaultArgs),
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

#[cfg(feature = "collab")]
#[derive(Args)]
pub struct VaultArgs {
    #[command(subcommand)]
    pub command: VaultCommand,
}

#[cfg(feature = "collab")]
#[derive(Subcommand)]
pub enum VaultCommand {
    /// Serve the vault until interrupted.
    ///
    /// Prints the ticket that links a device. A device pairs on its first
    /// link, then links again on every start. The vault keeps every graph
    /// with its whole history, and writes each change to disk before the
    /// device hears it was accepted.
    Serve(ServeArgs),
    /// List the paired devices.
    Devices(DirArgs),
    /// Unpair a device and rotate the pairing secret, so the old ticket no
    /// longer pairs. Takes effect when the vault next starts.
    Revoke(RevokeArgs),
}

#[cfg(feature = "collab")]
#[derive(Args)]
pub struct DirArgs {
    /// The vault directory, created if absent. Defaults to `vault` under the
    /// app data directory.
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
}

#[cfg(feature = "collab")]
#[derive(Args)]
pub struct ServeArgs {
    #[command(flatten)]
    pub dir: DirArgs,
    /// A self-hosted relay URL instead of the default infrastructure.
    #[arg(long, value_name = "URL")]
    pub relay: Option<String>,
    /// The UDP port to bind. A fixed port keeps old tickets valid across
    /// restarts.
    #[arg(long, default_value_t = vault::DEFAULT_PORT)]
    pub port: u16,
}

#[cfg(feature = "collab")]
#[derive(Args)]
pub struct RevokeArgs {
    /// The device's id, as `gantz vault devices` prints it.
    pub peer: gantz_collab::PeerId,
    #[command(flatten)]
    pub dir: DirArgs,
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

#[derive(Args)]
pub struct RunArgs {
    #[command(flatten)]
    seed: SeedArgs,
    /// The .gantz file defining the graph.
    file: PathBuf,
    /// The graph to run. Defaults to the file's unique root graph, the one
    /// no other graph in the file references.
    #[arg(long, value_name = "NAME")]
    graph: Option<String>,
    /// Fire this push source instead of the main! nodes. A node label in the
    /// graph, or a `/`-joined path of node ids such as `2/1`.
    #[arg(long, value_name = "NODE")]
    push: Option<String>,
    /// Print the main! nodes and push sources, one per line, and run
    /// nothing. Each line holds the kind, the node path and the node label,
    /// or `-` for a node with no label.
    #[arg(long, conflicts_with = "push")]
    list: bool,
    /// The program arguments. Each main! node outputs them as a list of
    /// strings.
    #[arg(last = true, value_name = "ARGS")]
    args: Vec<String>,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Emit {
    /// The Steel module text.
    Steel,
    /// The source map: one line per definition and identifier occurrence,
    /// with line:col positions into the Steel module text and the node path
    /// each refers to.
    SourceMap,
}

/// What a subcommand core produced.
#[derive(Default)]
pub struct Output {
    /// Text for stdout.
    pub stdout: String,
    /// Lines for stderr. Any means a non-zero exit.
    pub diagnostics: Vec<String>,
    /// Non-fatal lines for stderr.
    pub warnings: Vec<String>,
    /// Text to write back to a target, by source index.
    pub writes: Vec<(usize, String)>,
}

/// A loaded and reified source set, ready to compile.
struct Ready {
    codec: gantz_egui::node::NodeCodec,
    loaded: headless::Loaded,
    reified: headless::Reified,
    builtins: headless::Builtins,
}

/// The one graph that a `compile` or `run` target names.
struct Target<'a> {
    /// The target source's label.
    label: &'a str,
    /// The graph's name.
    name: gantz_ca::Name,
    /// The node index of each node label in the graph, if the source labels
    /// its nodes.
    labels: Option<&'a HashMap<String, usize>>,
    /// The typed graph.
    graph: &'a gantz_core::node::graph::Graph<gantz_egui::node::DynNode>,
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

/// Run a subcommand and return the process exit code.
pub fn run(command: Command, conf: &Conf) -> i32 {
    init_logs(&command);
    let (files, mut output) = match command {
        #[cfg(feature = "collab")]
        Command::Join(args) => return crate::join::run(args, conf),
        #[cfg(feature = "collab")]
        Command::Vault(args) => return crate::vault::run(args, conf),
        Command::Fmt(args) => {
            let output = with_sources(conf, &args.seed, &args.files, |s, t| {
                fmt(conf, s, t, args.check)
            });
            (args.files, output)
        }
        Command::Check(args) => {
            let output = with_sources(conf, &args.seed, &args.files, |s, t| check(conf, s, t));
            (args.files, output)
        }
        Command::Compile(args) => {
            let files = vec![args.file];
            let output = with_sources(conf, &args.seed, &files, |s, t| {
                compile(conf, s, t.start, args.graph.as_deref(), args.emit)
            });
            (files, output)
        }
        Command::Run(args) => {
            let files = vec![args.file];
            let output = with_sources(conf, &args.seed, &files, |s, t| {
                let name = args.graph.as_deref();
                if args.list {
                    list(conf, s, t.start, name)
                } else {
                    run_graph(conf, s, t.start, name, args.push.as_deref(), &args.args)
                }
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

/// Log to stderr.
///
/// Library warnings, such as an unrecognised form a rewrite would drop,
/// must reach the user. Libraries log through `log` and `tracing`, and the
/// subscriber bridges both. `RUST_LOG` overrides the default. A host that
/// installed its own subscriber keeps it.
fn init_logs(command: &Command) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,gantz_cli=info"));
    let logs = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr);
    // A vault serves for days, so its lines carry the time.
    let _ = match command {
        #[cfg(feature = "collab")]
        Command::Vault(_) => logs.try_init(),
        _ => logs.without_time().try_init(),
    };
}

/// Read the base sources unless omitted, then the deps, then the target
/// files, and run the core over them with the targets' index range.
///
/// Writes in the output are re-keyed from source index to target index.
/// A file that cannot be read is a diagnostic and the core does not run.
fn with_sources(
    conf: &Conf,
    seed: &SeedArgs,
    files: &[PathBuf],
    core: impl FnOnce(&[Source], Range<usize>) -> Output,
) -> Output {
    let mut sources = if seed.no_base {
        vec![]
    } else {
        headless::base_sources(conf)
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
        for (name, ca) in parsed.registry.heads() {
            let earlier = sources[..ix]
                .iter()
                .zip(&loaded.parsed[..ix])
                .filter_map(|(s, p)| p.as_ref().ok().map(|p| (s, &p.registry)))
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
fn ready(conf: &Conf, sources: &[Source], targets: Range<usize>) -> (Ready, Output) {
    let codec = conf.codec;
    let (loaded, mut output) = load(sources, targets, &codec);
    let (reified, errs) = headless::reify_all(&loaded.registry, &codec);
    output
        .diagnostics
        .extend(errs.iter().map(|e| gantz_core::vm::error_chain(e)));
    let ready = Ready {
        codec,
        loaded,
        reified,
        builtins: headless::builtins_with_instances(conf),
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
pub fn fmt(conf: &Conf, sources: &[Source], targets: Range<usize>, check: bool) -> Output {
    let codec = conf.codec;
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
            gantz_egui::format::to_string(&parsed.registry, &codec)
        } else {
            gantz_egui::format::to_string_named(&parsed.registry, &codec)
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
        .map(|d| diagnostic_line(label, name, d))
        .collect()
}

/// Render a diagnostic for the named graph as one line, prefixed by the
/// path of the node it concerns, if any.
fn diagnostic_line(label: &str, name: &gantz_ca::Name, d: &gantz_core::Diagnostic) -> String {
    let node = if d.path.is_empty() {
        String::new()
    } else {
        format!("node {}: ", path_text(&d.path))
    };
    format!("{label}: graph `{name}`: {node}{}", d.message)
}

/// A node path as `/`-joined ids.
fn path_text(path: &[gantz_core::node::Id]) -> String {
    path.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("/")
}

/// Compile every named graph the targets define.
pub fn check(conf: &Conf, sources: &[Source], targets: Range<usize>) -> Output {
    let (ready, mut output) = ready(conf, sources, targets.clone());
    let env = ready.env();
    let get_node = |ca: &gantz_ca::ContentAddr| env.node(ca);
    for ix in targets {
        let Ok(parsed) = &ready.loaded.parsed[ix] else {
            continue;
        };
        let label = &sources[ix].label;
        for (name, _) in parsed.registry.heads() {
            let head = gantz_ca::Head::Branch(name.clone());
            let Some(graph) = headless::head_graph(&ready.reified, &ready.loaded.registry, &head)
            else {
                output
                    .diagnostics
                    .push(format!("{label}: graph `{name}`: no head graph"));
                continue;
            };
            if let Err(e) = headless::init(conf, &get_node, graph) {
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
pub fn compile(
    conf: &Conf,
    sources: &[Source],
    target: usize,
    name: Option<&str>,
    emit: Emit,
) -> Output {
    let (ready, mut output) = ready(conf, sources, target..target + 1);
    let Target {
        label, name, graph, ..
    } = match target_graph(&ready, sources, target, name) {
        Ok(target) => target,
        Err(diagnostic) => {
            output.diagnostics.extend(diagnostic);
            return output;
        }
    };
    let env = ready.env();
    let get_node = |ca: &gantz_ca::ContentAddr| env.node(ca);
    let entrypoints = (conf.entrypoints)(&get_node, graph);
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

/// Print the root `main!` nodes and the push sources of the graph `name`
/// names, or the target's unique root graph. One line per node holds its
/// kind, its path and its label, or `-` for a node with no label.
pub fn list(conf: &Conf, sources: &[Source], target: usize, name: Option<&str>) -> Output {
    let (ready, mut output) = ready(conf, sources, target..target + 1);
    let Target { labels, graph, .. } = match target_graph(&ready, sources, target, name) {
        Ok(target) => target,
        Err(diagnostic) => {
            output.diagnostics.extend(diagnostic);
            return output;
        }
    };
    let env = ready.env();
    let get_node = |ca: &gantz_ca::ContentAddr| env.node(ca);
    let line = |kind: &str, path: &[gantz_core::node::Id]| {
        let node_label = node_label(labels, path).unwrap_or("-");
        format!("{kind} {} {node_label}\n", path_text(path))
    };
    let main = gantz_io::main_bang::entrypoint(graph);
    let mains = main
        .iter()
        .flat_map(|ep| &ep.0)
        .map(|s| line("main!", &s.path));
    let pushes = push_sources(&get_node, graph);
    let pushes = pushes.iter().map(|(path, _)| line("push", path));
    output.stdout = mains.chain(pushes).collect();
    output
}

/// Run the graph `name` names, or the target's unique root graph.
///
/// Fires the root `main!` nodes together, each with `args` as its output.
/// With `push`, fires that push source instead. Each value that the graph's
/// `log` nodes emit becomes one line of stdout, in firing order.
pub fn run_graph(
    conf: &Conf,
    sources: &[Source],
    target: usize,
    name: Option<&str>,
    push: Option<&str>,
    args: &[String],
) -> Output {
    let (ready, mut output) = ready(conf, sources, target..target + 1);
    let Target {
        label,
        name,
        labels,
        graph,
    } = match target_graph(&ready, sources, target, name) {
        Ok(target) => target,
        Err(diagnostic) => {
            output.diagnostics.extend(diagnostic);
            return output;
        }
    };
    let env = ready.env();
    let get_node = |ca: &gantz_ca::ContentAddr| env.node(ca);
    let main = gantz_io::main_bang::entrypoint(graph);
    let pushes = push_sources(&get_node, graph);
    let entry = match (push, &main) {
        (Some(node), _) => {
            let path = node_path(labels, node);
            match pushes.iter().find(|(p, _)| Some(p) == path.as_ref()) {
                Some((_, ep)) => ep.clone(),
                None => {
                    output.diagnostics.push(format!(
                        "{label}: graph `{name}`: no push source `{node}`. Push sources: {}",
                        push_sources_text(&pushes, labels),
                    ));
                    return output;
                }
            }
        }
        (None, Some(main)) => main.clone(),
        (None, None) => {
            output.diagnostics.push(format!(
                "{label}: graph `{name}`: no main! node, pass --push <NODE>. Push sources: {}",
                push_sources_text(&pushes, labels),
            ));
            return output;
        }
    };

    // Compile as the app does, plus the entry if the app does not compile it.
    let mut entrypoints = (conf.entrypoints)(&get_node, graph);
    if !entrypoints.contains(&entry) {
        entrypoints.push(entry.clone());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let mut vm = gantz_core::vm::new_engine(&conf.steel_modules);
    gantz_io::log::register_sink(&mut vm, move |_level, _path, val| {
        let _ = tx.send(val.to_string());
    });
    gantz_core::graph::register(&get_node, graph, &[], &mut vm);
    let config = gantz_core::compile::Config::default();
    let compiled = match gantz_core::vm::compile(&get_node, graph, &mut vm, &entrypoints, &config) {
        Ok(compiled) => compiled,
        Err(e) => {
            output
                .diagnostics
                .extend(compile_diagnostics(label, &name, &e));
            return output;
        }
    };

    for source in main.iter().flat_map(|ep| &ep.0) {
        if let Err(e) = gantz_core::node::state::update(&mut vm, &source.path, args.to_vec()) {
            let node = path_text(&source.path);
            output
                .diagnostics
                .push(format!("{label}: graph `{name}`: node {node}: {e}"));
            return output;
        }
    }
    let entry_fn = gantz_core::compile::entry_fn_name(&entry.id());
    let result = vm.call_function_by_name_with_args(&entry_fn, vec![]);
    output.stdout = rx.try_iter().map(|line| line + "\n").collect();
    if let Err(e) = result {
        let diagnostic = gantz_core::diagnostic::from_eval_error(&e, &vm, &compiled);
        output
            .diagnostics
            .push(diagnostic_line(label, &name, &diagnostic));
    }
    output
}

/// The graph `name` names in the target, or else the target's unique root
/// graph.
///
/// An `Err` holds the diagnostic, or `None` when the target failed to parse.
/// [`ready`] reports that failure.
fn target_graph<'a>(
    ready: &'a Ready,
    sources: &'a [Source],
    target: usize,
    name: Option<&str>,
) -> Result<Target<'a>, Option<String>> {
    let label = &sources[target].label;
    let Ok(parsed) = &ready.loaded.parsed[target] else {
        return Err(None);
    };
    let registry = &parsed.registry;
    let name = match name {
        Some(name) => {
            let name: gantz_ca::Name = name.parse().expect("infallible");
            if !registry.heads().any(|(n, _)| *n == name) {
                return Err(Some(format!("{label}: no graph named `{name}`")));
            }
            name
        }
        None => gantz_egui::export::unique_root_name(registry).ok_or_else(|| {
            let names: Vec<String> = registry.heads().map(|(n, _)| n.to_string()).collect();
            Some(format!(
                "{label}: no unique root graph, pass --graph <NAME>. Graphs: {}",
                names.join(", "),
            ))
        })?,
    };
    let head = gantz_ca::Head::Branch(name.clone());
    let Some(graph) = headless::head_graph(&ready.reified, &ready.loaded.registry, &head) else {
        return Err(Some(format!("{label}: graph `{name}`: no head graph")));
    };
    let labels = parsed.labels.get(&name);
    Ok(Target {
        label,
        name,
        labels,
        graph,
    })
}

/// The push sources of the graph and its nested graphs, as each node's path
/// and push entrypoint, in path order.
fn push_sources(
    get_node: gantz_core::node::GetNode<'_>,
    graph: &gantz_core::node::graph::Graph<gantz_egui::node::DynNode>,
) -> Vec<(Vec<gantz_core::node::Id>, gantz_core::compile::Entrypoint)> {
    let push = gantz_core::compile::EvalKind::Push;
    let mut sources: Vec<_> = gantz_core::compile::push_pull_entrypoints(get_node, graph)
        .into_iter()
        .filter_map(|ep| {
            let source = ep.0.first().filter(|s| s.kind == push)?;
            Some((source.path.clone(), ep))
        })
        .collect();
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    sources.dedup_by(|a, b| a.0 == b.0);
    sources
}

/// The path of the node that `node` names. That is a node label at the
/// graph's root, or else a `/`-joined path of node ids.
fn node_path(
    labels: Option<&HashMap<String, usize>>,
    node: &str,
) -> Option<Vec<gantz_core::node::Id>> {
    match labels.and_then(|labels| labels.get(node)) {
        Some(&ix) => Some(vec![ix]),
        None => node.split('/').map(|id| id.parse().ok()).collect(),
    }
}

/// The label of the node at `path`, if the node is at the graph's root and
/// the file labels it.
fn node_label<'a>(
    labels: Option<&'a HashMap<String, usize>>,
    path: &[gantz_core::node::Id],
) -> Option<&'a str> {
    let [id] = path else {
        return None;
    };
    let (label, _) = labels?.iter().find(|(_, ix)| *ix == id)?;
    Some(label)
}

/// The push sources as `--push` accepts them, by label or else by path.
fn push_sources_text(
    pushes: &[(Vec<gantz_core::node::Id>, gantz_core::compile::Entrypoint)],
    labels: Option<&HashMap<String, usize>>,
) -> String {
    let names: Vec<String> = pushes
        .iter()
        .map(|(path, _)| node_label(labels, path).map_or_else(|| path_text(path), str::to_string))
        .collect();
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
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
