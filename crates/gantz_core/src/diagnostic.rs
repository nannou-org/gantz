//! Structured diagnostics extracted from compile and runtime errors, for
//! frontends to surface against the graph (e.g. highlighting the offending
//! node) and the emitted module source (e.g. highlighting the error span).

use crate::{
    compile::ModuleError,
    compile::error::{LowerError, NodeConnsError, NodeFnError},
    node, vm,
};
use std::ops::Range;
use steel::{SteelErr, steel_vm::engine::Engine};

/// When the diagnosed error arose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    /// Generating or loading the module failed.
    Compile,
    /// Evaluating an entrypoint failed.
    Runtime,
}

/// One error attributed to a location in the graph and, when available, the
/// emitted module source.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    /// Full path of the implicated node; empty means the graph as a whole.
    pub path: Vec<node::Id>,
    /// Implicated input indices on the node (e.g. an edge referencing an
    /// invalid input), for edge-level attribution.
    pub inputs: Vec<usize>,
    /// Implicated output indices on the node.
    pub outputs: Vec<usize>,
    /// Byte range into the compiled module source ([`vm::Compiled::src`]).
    pub span: Option<Range<usize>>,
    /// Human-readable description.
    pub message: String,
    /// When the error arose.
    pub severity: Severity,
}

impl Diagnostic {
    /// A compile diagnostic for the node at `path`.
    fn compile(path: Vec<node::Id>, message: String) -> Self {
        Self {
            path,
            inputs: vec![],
            outputs: vec![],
            span: None,
            message,
            severity: Severity::Compile,
        }
    }
}

/// Diagnostics for a [`vm::CompileError`] from [`vm::compile`]/[`vm::init`].
pub fn from_compile_error(err: &vm::CompileError) -> Vec<Diagnostic> {
    match err {
        vm::CompileError::Module(err) => from_module_error(err),
        vm::CompileError::Eval { err, module } => {
            // The error came from running exactly this module, so its span
            // needs no source verification.
            let span = vm::steel_err_raw_span(err);
            let path = span
                .clone()
                .and_then(|span| module.map.node_at(span))
                .unwrap_or_default();
            vec![Diagnostic {
                path,
                inputs: vec![],
                outputs: vec![],
                span,
                message: err.to_string(),
                severity: Severity::Compile,
            }]
        }
    }
}

/// Diagnostics for a module generation error.
pub fn from_module_error(err: &ModuleError) -> Vec<Diagnostic> {
    match err {
        ModuleError::NodeConns { path, error } => vec![conns_diag(path.clone(), error)],
        ModuleError::Lower { path, error } => lower_diags(path, error),
        ModuleError::NestedGraphNotFound(error) => {
            vec![Diagnostic::compile(error.0.clone(), error.to_string())]
        }
        ModuleError::MetaErrors(errors) => errors
            .0
            .iter()
            .map(|e| conns_diag(e.path.clone(), &e.error))
            .collect(),
        ModuleError::NodeFnErrors(errors) => errors.0.iter().map(node_fn_diag).collect(),
        ModuleError::InvalidIr { path, .. } => {
            vec![Diagnostic::compile(path.clone(), err.to_string())]
        }
    }
}

/// The diagnostic for a steel error raised evaluating the compiled module
/// (typically from an entrypoint call).
pub fn from_eval_error(err: &SteelErr, vm: &Engine, compiled: &vm::Compiled) -> Diagnostic {
    let span = vm::steel_err_span(err, vm, compiled);
    let path = span
        .clone()
        .and_then(|span| compiled.map.node_at(span))
        .unwrap_or_default();
    Diagnostic {
        path,
        inputs: vec![],
        outputs: vec![],
        span,
        message: err.to_string(),
        severity: Severity::Runtime,
    }
}

/// A diagnostic for a connection error at the node or level at `path`.
fn conns_diag(path: Vec<node::Id>, error: &NodeConnsError) -> Diagnostic {
    let mut diag = Diagnostic::compile(path, error.to_string());
    match error {
        NodeConnsError::InvalidOutputIndex(e) => diag.outputs.push(e.index),
        NodeConnsError::TooManyConns(_) => {}
    }
    diag
}

/// Diagnostics for a lowering error, resolving the error's level-relative
/// node ids against the level's path.
fn lower_diags(level: &[node::Id], error: &LowerError) -> Vec<Diagnostic> {
    let full = |id: node::Id| level.iter().copied().chain([id]).collect::<Vec<_>>();
    let message = error.to_string();
    match error {
        LowerError::Conns { node, error } => {
            let path = node.map(full).unwrap_or_else(|| level.to_vec());
            vec![conns_diag(path, error)]
        }
        LowerError::Entangled { branch, node } => vec![
            Diagnostic::compile(full(*node), message.clone()),
            Diagnostic::compile(full(*branch), message),
        ],
        LowerError::MixedInputSources { node, input } => {
            let mut diag = Diagnostic::compile(full(*node), message);
            diag.inputs.push(*input);
            vec![diag]
        }
        LowerError::Unresolved {
            node,
            output,
            consumer,
        } => {
            let mut producer = Diagnostic::compile(full(*node), message.clone());
            producer.outputs.push(*output);
            vec![producer, Diagnostic::compile(full(*consumer), message)]
        }
    }
}

/// The diagnostic for a node fn generation error.
fn node_fn_diag(error: &NodeFnError) -> Diagnostic {
    match error {
        NodeFnError::NestedGraphNotFound(e) => Diagnostic::compile(e.0.clone(), e.to_string()),
        NodeFnError::Expr(e) => Diagnostic::compile(e.path.clone(), e.to_string()),
    }
}
