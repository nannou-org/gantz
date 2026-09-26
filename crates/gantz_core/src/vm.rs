//! Shared VM utilities for initializing and compiling gantz graphs.
//!
//! This module provides common functionality for working with the Steel VM
//! that is shared between the gantz frontends.

use crate::{
    Edge, Node,
    compile::{ModuleError, SourceMap},
    node,
};
use petgraph::visit::{Data, IntoEdgesDirected, IntoNodeReferences, NodeIndexable, Visitable};
use steel::{
    SteelErr, SteelVal,
    parser::{ast::ExprKind, span::Span},
    steel_vm::{builtin::BuiltInModule, engine::Engine},
};

/// A compiled gantz module.
#[derive(Clone, Debug)]
pub struct Compiled {
    /// The module's top-level expressions.
    pub exprs: Vec<ExprKind>,
    /// The module source. Exactly the text executed in the VM, so steel
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
        /// The underlying steel error. Its span, if any, indexes into the
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
/// Registration is cheap. The engine stores the source text and only
/// compiles the module when a program first `(require ...)`s it by name.
/// It caches the result for the engine's lifetime. Graphs that never
/// require a module never pay for it.
///
/// Modules must be registered via [`new_engine`], which installs a
/// minimal prelude string first. Steel prepends its prelude string to a
/// module's source at registration time. The default prelude would drag
/// the entire steel stdlib into the module's first `(require ...)`.
///
/// A module can also carry a Rust [`BuiltInModule`], registered with the
/// source. The source then `(require-builtin ...)`s it by its name and
/// `provide`s the Rust fns that it wants to expose.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct SteelModule {
    /// The name used to `(require ...)` the module.
    pub name: &'static str,
    /// The module's Steel source.
    pub src: &'static str,
    /// Constructs the Rust builtin module that the source requires, if any.
    pub builtin: Option<fn() -> BuiltInModule>,
}

/// The Steel modules provided by `gantz_core` itself.
///
/// Always registered by [`new_engine`], ahead of any domain modules.
const CORE_MODULES: &[SteelModule] = &[
    SteelModule::new("gantz/option", include_str!("vm/option.scm")),
    SteelModule::new("gantz/list", include_str!("vm/list.scm")),
];

impl SteelModule {
    /// A source module with the given name.
    pub const fn new(name: &'static str, src: &'static str) -> Self {
        Self {
            name,
            src,
            builtin: None,
        }
    }

    /// Carry the Rust builtin module constructed by `builtin`.
    pub const fn with_builtin(mut self, builtin: fn() -> BuiltInModule) -> Self {
        self.builtin = Some(builtin);
        self
    }
}

impl CompileError {
    /// The generated module, when compilation got far enough to produce one.
    /// Steel rejecting the module still yields it, so its source remains
    /// displayable and error spans resolvable.
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
/// any module is registered. See [`SteelModule`]. Module sources get the
/// base primitives and must `(require-builtin ...)` anything further
/// themselves.
///
/// Each module name is registered once, and the first module with a name
/// wins. A domain's module list can then include the modules it depends
/// on, and frontends can chain several such lists.
pub fn new_engine(extra_modules: &[SteelModule]) -> Engine {
    let mut vm = Engine::new_base();
    vm.set_prelude_string(std::borrow::Cow::Borrowed("(require-builtin steel/base)\n"));
    let mut registered: Vec<&SteelModule> = vec![];
    for m in modules().iter().chain(extra_modules) {
        if let Some(prev) = registered.iter().find(|prev| prev.name == m.name) {
            debug_assert_eq!(prev.src, m.src, "two modules named `{}`", m.name);
            continue;
        }
        if let Some(builtin) = m.builtin {
            vm.register_module(builtin());
        }
        vm.register_steel_module(m.name.to_string(), m.src.to_string());
        registered.push(m);
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
/// registered on the freshly created engine. See [`new_engine`].
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
/// The module runs as a single program so that the engine registers
/// [`Compiled::src`] verbatim as one source. Subsequent steel errors then
/// carry spans whose offsets index into it directly. See
/// [`steel_err_node`].
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
/// belongs to the module when its source text is exactly [`Compiled::src`].
/// The text is looked up in the engine by the span's source id. Spans from
/// other sources and span-less errors yield `None`. Other sources include
/// snippets run by node UIs and modules from before a recompile.
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

/// The first span attached to a steel error, without verifying which
/// source it points into.
///
/// Only sound when the error's provenance is already known. For example, an
/// error returned by [`compile`] itself can only index the module just run.
pub fn steel_err_raw_span(err: &SteelErr) -> Option<std::ops::Range<usize>> {
    steel_err_spans(err).next().map(|span| span.usize_range())
}

/// The full path of the node best attributed to a steel error. See
/// [`steel_err_span`].
pub fn steel_err_node(err: &SteelErr, vm: &Engine, compiled: &Compiled) -> Option<Vec<node::Id>> {
    compiled.map.node_at(steel_err_span(err, vm, compiled)?)
}

/// Format an error together with its full [`std::error::Error::source`] chain.
///
/// `Display` renders only the outermost message. A wrapper like
/// [`CompileError`] over [`crate::compile::ModuleError`] otherwise hides the
/// underlying cause behind a bare "module generation failed". This walks the
/// `source()` chain and appends each level on its own `caused by:` line.
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

/// The spans attached to a steel error. Its own span first, then its stack
/// trace frames innermost-first. Frames are pushed caller-first.
fn steel_err_spans(err: &SteelErr) -> impl Iterator<Item = Span> + '_ {
    err.span().into_iter().chain(
        err.stack_trace()
            .iter()
            .flat_map(|trace| trace.trace().iter().rev().filter_map(|frame| *frame.span())),
    )
}

#[cfg(test)]
mod tests {
    use super::new_engine;
    use steel::SteelVal;

    /// Values a partial graph eval can pass in place of a list or fn.
    const JUNK: &[&str] = &["'()", "void", "7", "'sym", "\"str\""];

    /// Values that are not usable as a count or bound.
    const NUM_JUNK: &[&str] = &[
        "'()", "void", "'sym", "\"str\"", "+nan.0", "+inf.0", "-inf.0",
    ];

    /// Assert that each `(expected, expr)` pair is `equal?` on one engine that
    /// requires `gantz/list`.
    fn assert_list_evals(cases: &[(String, String)]) {
        let mut vm = new_engine(&[]);
        vm.run("(require \"gantz/list\")".to_string())
            .expect("require gantz/list");
        for (expected, expr) in cases {
            let equal = vm
                .run(format!("(equal? {expected} {expr})"))
                .unwrap_or_else(|e| panic!("`{expr}` errored: {e}"));
            if equal.last() != Some(&SteelVal::BoolV(true)) {
                let actual = vm.run(expr.clone()).expect("eval");
                panic!("`{expr}`: expected `{expected}`, got `{actual:?}`");
            }
        }
    }

    fn cases(cases: &[(&str, &str)]) -> Vec<(String, String)> {
        cases
            .iter()
            .map(|(e, x)| (e.to_string(), x.to_string()))
            .collect()
    }

    /// Each case once per junk value, with the `J` in its expr replaced by
    /// that value.
    fn with_junk(cases: &[(&str, &str)], junk: &[&str]) -> Vec<(String, String)> {
        junk.iter()
            .flat_map(|j| {
                cases
                    .iter()
                    .map(move |(e, x)| (e.to_string(), x.replace('J', j)))
            })
            .collect()
    }

    #[test]
    fn list_higher_order_fns() {
        assert_list_evals(&cases(&[
            ("'(2 4 6)", "(list/map (lambda (x) (* x 2)) '(1 2 3))"),
            ("'(-1 -2)", "(list/map - '(1 2))"),
            ("'(2 4)", "(list/filter even? '(1 2 3 4))"),
            // `fold` calls `(f acc x)`, first to last.
            ("-6", "(list/fold - 0 '(1 2 3))"),
            (
                "'(3 2 1)",
                "(list/fold (lambda (acc x) (cons x acc)) '() '(1 2 3))",
            ),
            ("5", "(list/fold + 5 '())"),
            (
                "'(1 1 2 2)",
                "(list/flat-map (lambda (x) (list x x)) '(1 2))",
            ),
            // A non-list result counts as one item, and only one level joins.
            ("'(1 2 3)", "(list/flat-map (lambda (x) x) '(1 (2 3)))"),
            ("'(1 (2))", "(list/flat-map (lambda (x) x) '((1 (2))))"),
            ("#t", "(list/any even? '(1 2))"),
            ("#f", "(list/any even? '(1 3))"),
            ("#f", "(list/any even? '())"),
            ("#t", "(list/all odd? '(1 3))"),
            ("#f", "(list/all odd? '(1 2))"),
            ("#t", "(list/all odd? '())"),
            ("2", "(list/find even? '(1 2 4))"),
            ("#f", "(list/find even? '(1 3))"),
        ]));
    }

    #[test]
    fn list_sort_is_ordered_and_stable() {
        assert_list_evals(&cases(&[
            ("'(1 2 3 4 5)", "(list/sort < '(5 3 1 4 2))"),
            ("'(5 4 3 2 1)", "(list/sort > '(1 2 3 4 5))"),
            ("'()", "(list/sort < '())"),
            ("'(1)", "(list/sort < '(1))"),
            (
                "'((1 b) (1 d) (2 a) (2 c))",
                "(list/sort (lambda (a b) (< (car a) (car b))) '((2 a) (1 b) (2 c) (1 d)))",
            ),
        ]));
    }

    #[test]
    fn list_generator_slicing_and_combining_fns() {
        assert_list_evals(&cases(&[
            ("'(0 1 2)", "(list/range 0 3)"),
            ("'(-2 -1 0)", "(list/range -2 1)"),
            ("'()", "(list/range 3 3)"),
            ("'()", "(list/range 3 1)"),
            // Bounds round to exact integers.
            ("'(1 2)", "(list/range 0.6 2.6)"),
            ("'(1 2)", "(list/take '(1 2 3) 2)"),
            ("'(1 2)", "(list/take '(1 2 3) 1.6)"),
            ("'(1 2 3)", "(list/take '(1 2 3) 9)"),
            ("'()", "(list/take '(1 2 3) -1)"),
            ("'(3)", "(list/drop '(1 2 3) 2)"),
            ("'()", "(list/drop '(1 2 3) 9)"),
            ("'(1 2 3)", "(list/drop '(1 2 3) -1)"),
            ("3", "(list/last '(1 2 3))"),
            ("'()", "(list/last '())"),
            ("'((1 a) (2 b))", "(list/zip '(1 2 3) '(a b))"),
            ("'()", "(list/zip '() '(a))"),
            ("'(1 2 3 4)", "(list/concat '((1 2) () (3) 4))"),
        ]));
    }

    /// Junk in place of any argument evaluates to the empty result rather
    /// than an error.
    #[test]
    fn list_fns_are_total() {
        // Junk in place of a list counts as the empty list.
        let mut all = with_junk(
            &[
                ("'()", "(list/map - J)"),
                ("'()", "(list/filter even? J)"),
                ("0", "(list/fold + 0 J)"),
                ("'()", "(list/flat-map list J)"),
                ("'()", "(list/concat J)"),
                ("'()", "(list/zip J '(1))"),
                ("'()", "(list/zip '(1) J)"),
                ("#f", "(list/any even? J)"),
                ("#t", "(list/all even? J)"),
                ("#f", "(list/find even? J)"),
                ("'()", "(list/sort < J)"),
                ("'()", "(list/take J 1)"),
                ("'()", "(list/drop J 1)"),
                ("'()", "(list/last J)"),
            ],
            JUNK,
        );
        // Junk in place of a number counts as 0.
        all.extend(with_junk(
            &[
                ("'()", "(list/take '(1 2) J)"),
                ("'(1 2)", "(list/drop '(1 2) J)"),
                ("'(0 1)", "(list/range J 2)"),
                ("'()", "(list/range 0 J)"),
            ],
            NUM_JUNK,
        ));
        // `void` is itself a fn, so only the rest stand in for a fn.
        let non_fns: Vec<_> = JUNK.iter().copied().filter(|j| *j != "void").collect();
        all.extend(with_junk(
            &[
                ("'()", "(list/map J '(1 2))"),
                ("'()", "(list/filter J '(1 2))"),
                ("0", "(list/fold J 0 '(1 2))"),
                ("'()", "(list/flat-map J '(1 2))"),
                ("#f", "(list/any J '(1 2))"),
                ("#f", "(list/all J '(1 2))"),
                ("#f", "(list/find J '(1 2))"),
                ("'()", "(list/sort J '(2 1))"),
            ],
            &non_fns,
        ));
        assert_list_evals(&all);
    }
}
