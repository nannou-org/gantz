//! Shared VM utilities for initializing and compiling gantz graphs.
//!
//! This module provides common functionality for working with the Steel VM
//! that is shared between different gantz frontends (Bevy app, pure egui demo, etc.).

use crate::{
    Edge, Node,
    compile::{ModuleError, SourceMap},
    node,
};
use petgraph::visit::{Data, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, Visitable};
use steel::{
    SteelErr, SteelVal,
    parser::{ast::ExprKind, span::Span},
    steel_vm::engine::Engine,
};

/// A compiled gantz module.
#[derive(Clone, Debug)]
pub struct Compiled {
    /// The module's top-level expressions.
    pub exprs: Vec<ExprKind>,
    /// The module source: exactly the text executed in the VM, so steel
    /// error spans and [`Compiled::map`] offsets index into it directly.
    pub src: String,
    /// Byte-offset map from [`Compiled::src`] back to graph node paths.
    pub map: SourceMap,
}

/// Errors that can occur during VM compilation.
#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    /// Error generating the Steel module from the graph.
    #[error("module generation failed")]
    Module(#[from] ModuleError),
    /// Steel rejected or errored running the module.
    #[error("module evaluation failed")]
    Eval {
        /// The underlying steel error; its span (if any) indexes into the
        /// carried module's source.
        #[source]
        err: SteelErr,
        /// The module that failed to evaluate, so frontends can still
        /// display its source and resolve the error span.
        module: Box<Compiled>,
    },
}

/// A named Steel source module that can be registered with an [`Engine`].
///
/// Registration is cheap: the engine stores the source text and only
/// compiles the module when a program first `(require ...)`s it by name,
/// caching the result for the engine's lifetime. Graphs that never
/// require a module never pay for it.
///
/// Modules must be registered via [`new_engine`], which installs a
/// minimal prelude string first. Steel prepends its prelude string to a
/// module's source *at registration time*, and the default prelude would
/// drag the entire steel stdlib into the module's first `(require ...)`.
#[derive(Clone, Copy, Debug)]
pub struct SteelModule {
    /// The name used to `(require ...)` the module.
    pub name: &'static str,
    /// The module's Steel source.
    pub src: &'static str,
}

/// The Steel modules provided by `gantz_core` itself.
///
/// Always registered by [`new_engine`], ahead of any domain modules.
const CORE_MODULES: &[SteelModule] = &[SteelModule {
    name: "gantz/option",
    src: include_str!("vm/option.scm"),
}];

impl CompileError {
    /// The generated module, when compilation got far enough to produce one
    /// (steel rejecting the module still yields the artifact, so its source
    /// remains displayable and error spans resolvable).
    pub fn into_module(self) -> Option<Compiled> {
        match self {
            Self::Module(_) => None,
            Self::Eval { module, .. } => Some(*module),
        }
    }
}

/// The Steel modules provided by `gantz_core` itself.
///
/// [`new_engine`] registers these on every engine, so their bindings are
/// available to any graph via `(require ...)` regardless of which domains
/// are present.
pub fn modules() -> &'static [SteelModule] {
    CORE_MODULES
}

/// Create a new base [`Engine`] with gantz's core Steel [`modules`] and
/// the given domain modules registered, plus the root state and args
/// globals.
///
/// The prelude string is reduced to `(require-builtin steel/base)` before
/// any module is registered (see [`SteelModule`]): module sources get the
/// base primitives and must `(require-builtin ...)` anything further
/// themselves.
pub fn new_engine(extra_modules: &[SteelModule]) -> Engine {
    let mut vm = Engine::new_base();
    vm.set_prelude_string(std::borrow::Cow::Borrowed("(require-builtin steel/base)\n"));
    for m in modules().iter().chain(extra_modules) {
        vm.register_steel_module(m.name.to_string(), m.src.to_string());
    }
    vm.register_value(crate::ROOT_STATE, SteelVal::empty_hashmap());
    vm.register_value(crate::ARGS, crate::args::default());
    vm
}

/// Initialize a new VM with root state and register the given graph.
///
/// The VM is created via [`new_engine`] with no domain modules, so
/// gantz_core's own [`modules`] are available. To register additional
/// domain modules, use [`init_with_modules`].
///
/// Returns the initialized VM and the compiled module.
pub fn init<'a, G>(
    get_node: node::GetNode<'a>,
    graph: G,
    entrypoints: &[crate::compile::Entrypoint],
    config: &crate::compile::Config,
) -> Result<(Engine, Compiled), CompileError>
where
    G: Data<EdgeWeight = Edge>
        + IntoEdgesDirected
        + IntoNodeReferences
        + NodeIndexable
        + Visitable
        + Copy,
    G::NodeWeight: Node,
{
    init_with_modules(get_node, graph, entrypoints, config, &[])
}

/// The same as [`init`], but with additional domain [`SteelModule`]s
/// registered on the freshly created engine (see [`new_engine`]).
pub fn init_with_modules<'a, G>(
    get_node: node::GetNode<'a>,
    graph: G,
    entrypoints: &[crate::compile::Entrypoint],
    config: &crate::compile::Config,
    extra_modules: &[SteelModule],
) -> Result<(Engine, Compiled), CompileError>
where
    G: Data<EdgeWeight = Edge>
        + IntoEdgesDirected
        + IntoNodeReferences
        + NodeIndexable
        + Visitable
        + Copy,
    G::NodeWeight: Node,
{
    let mut vm = new_engine(extra_modules);
    crate::graph::register(get_node, graph, &[], &mut vm);
    let compiled = compile(get_node, graph, &mut vm, entrypoints, config)?;
    Ok((vm, compiled))
}

/// Compile the graph into a Steel module and run it in the VM.
///
/// The module runs as a *single* program so that the engine registers
/// [`Compiled::src`] verbatim as one source: subsequent steel errors then
/// carry spans whose offsets index into it directly (see
/// [`steel_err_node`]).
pub fn compile<'a, G>(
    get_node: node::GetNode<'a>,
    graph: G,
    vm: &mut Engine,
    entrypoints: &[crate::compile::Entrypoint],
    config: &crate::compile::Config,
) -> Result<Compiled, CompileError>
where
    G: Data<EdgeWeight = Edge>
        + IntoEdgesDirected
        + IntoNodeReferences
        + NodeIndexable
        + Visitable
        + Copy,
    G::NodeWeight: Node,
{
    let module_start = web_time::Instant::now();
    let exprs = crate::compile::module(get_node, graph, entrypoints, config)?;
    log::debug!("Generated steel module ({:?})", module_start.elapsed());

    let src = fmt_module(&exprs);
    let map = SourceMap::parse(&src);
    let compiled = Compiled { exprs, src, map };

    let run_start = web_time::Instant::now();
    let result = vm.run(compiled.src.clone());
    log::debug!("Compiled steel ({:?})", run_start.elapsed());
    match result {
        Ok(_) => Ok(compiled),
        Err(err) => Err(CompileError::Eval {
            err,
            module: Box::new(compiled),
        }),
    }
}

/// Format a compiled module as a human-readable string.
///
/// Each expression is pretty-printed with a width of 80 characters
/// and separated by blank lines.
pub fn fmt_module(module: &[ExprKind]) -> String {
    module
        .iter()
        .map(|expr| expr.to_pretty(80))
        .collect::<Vec<String>>()
        .join("\n\n")
}

/// The byte range into [`Compiled::src`] best attributed to a steel error.
///
/// Uses the error's own span when it points into the compiled module's
/// source, otherwise the innermost stack-trace frame that does. A span
/// belongs to the module when its source text (looked up in the engine by
/// the span's source id) is exactly [`Compiled::src`] - so spans from other
/// sources (e.g. snippets run by node UIs, or modules from before a
/// recompile) and span-less errors yield `None`.
pub fn steel_err_span(
    err: &SteelErr,
    vm: &Engine,
    compiled: &Compiled,
) -> Option<std::ops::Range<usize>> {
    let in_module = |span: &Span| {
        span.source_id()
            .and_then(|id| vm.get_source(&id))
            .is_some_and(|text| text.as_ref().as_ref() == compiled.src)
    };
    steel_err_spans(err)
        .find(in_module)
        .map(|span| span.usize_range())
}

/// The first span attached to a steel error, *without* verifying which
/// source it points into.
///
/// Only sound when the error's provenance is already known - e.g. an error
/// returned by [`compile`] itself, whose spans can only index the module
/// just run.
pub fn steel_err_raw_span(err: &SteelErr) -> Option<std::ops::Range<usize>> {
    steel_err_spans(err).next().map(|span| span.usize_range())
}

/// The full path of the node best attributed to a steel error (see
/// [`steel_err_span`]).
pub fn steel_err_node(err: &SteelErr, vm: &Engine, compiled: &Compiled) -> Option<Vec<node::Id>> {
    compiled.map.node_at(steel_err_span(err, vm, compiled)?)
}

/// Format an error together with its full [`std::error::Error::source`] chain.
///
/// `Display` renders only the outermost message, so a wrapper like
/// [`CompileError`] -> [`crate::compile::ModuleError`] -> the underlying cause
/// otherwise hides what actually went wrong (e.g. a bare "module generation
/// failed"). This walks the `source()` chain and appends each level on its own
/// `caused by:` line.
pub fn error_chain(err: &dyn std::error::Error) -> String {
    use std::fmt::Write;
    let mut s = err.to_string();
    let mut source = err.source();
    while let Some(e) = source {
        write!(s, "\ncaused by: {e}").expect("writing to a String never fails");
        source = e.source();
    }
    s
}

/// The spans attached to a steel error: its own span first, then its stack
/// trace frames innermost-first (frames are pushed caller-first).
fn steel_err_spans(err: &SteelErr) -> impl Iterator<Item = Span> + '_ {
    err.span().into_iter().chain(
        err.stack_trace()
            .iter()
            .flat_map(|trace| trace.trace().iter().rev().filter_map(|frame| *frame.span())),
    )
}
