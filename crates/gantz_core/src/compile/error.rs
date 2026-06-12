//! Error types for the compile module.

use crate::node;
use std::fmt;
use thiserror::Error;

/// Too many connections for a node (exceeds [`node::Conns::MAX`]).
#[derive(Debug, Error)]
#[error("too many connections ({0}), max is {max}", max = node::Conns::MAX)]
pub struct TooManyConns(pub usize);

/// An edge references an invalid output index.
#[derive(Debug, Error)]
#[error("edge references invalid output index {index} (node has {n_outputs} outputs)")]
pub struct InvalidOutputIndex {
    pub index: usize,
    pub n_outputs: usize,
}

/// A nested graph was not found at the expected path.
#[derive(Debug, Error)]
#[error("nested graph not found at path {0:?}")]
pub struct NestedGraphNotFound(pub Vec<node::Id>);

/// Expression generation failed for a node.
#[derive(Debug)]
pub struct NodeExprError {
    /// The path to the node that failed.
    pub path: Vec<node::Id>,
    /// The underlying error.
    pub error: node::ExprError,
}

/// Error during node function generation.
#[derive(Debug, Error)]
pub enum NodeFnError {
    /// A nested graph was not found.
    #[error(transparent)]
    NestedGraphNotFound(#[from] NestedGraphNotFound),
    /// Expression generation failed.
    #[error(transparent)]
    Expr(#[from] NodeExprError),
}

/// Multiple errors encountered during node function generation.
#[derive(Debug)]
pub struct NodeFnErrors(pub Vec<NodeFnError>);

/// Error when computing node connections from graph edges.
#[derive(Debug, Error)]
pub enum NodeConnsError {
    /// The node has too many connections.
    #[error(transparent)]
    TooManyConns(#[from] TooManyConns),
    /// An edge references an invalid output index.
    #[error(transparent)]
    InvalidOutputIndex(#[from] InvalidOutputIndex),
}

/// A node connection error with the path to the failing node.
#[derive(Debug)]
pub struct MetaError {
    /// The path to the node that caused the error.
    pub path: Vec<node::Id>,
    /// The underlying error.
    pub error: NodeConnsError,
}

/// Multiple errors encountered during meta collection.
#[derive(Debug)]
pub struct MetaErrors(pub Vec<MetaError>);

/// Error while lowering a graph to the IR.
///
/// Node ids are relative to the level being lowered.
#[derive(Debug, Error)]
pub enum LowerError {
    /// Error computing connection masks, at a specific node when known.
    #[error("connection error{}", at_node(node))]
    Conns {
        node: Option<node::Id>,
        #[source]
        error: NodeConnsError,
    },
    /// A node conditional on a branch arm depends on work outside the
    /// branch's region that has not yet been evaluated, so it can neither be
    /// lowered into the arm nor deferred past the branch.
    #[error(
        "branch {branch}: node {node} is conditional on an arm but depends on \
         unevaluated work outside the branch region"
    )]
    Entangled { branch: node::Id, node: node::Id },
    /// An input mixes branch-arm-varying sources with sources from other
    /// scopes (or multiple sources within one arm), which the lowering does
    /// not yet support.
    #[error("node {node} input {input}: unsupported mix of input sources")]
    MixedInputSources { node: node::Id, input: usize },
    /// Internal invariant breach: a value was needed but not in scope.
    #[error(
        "internal: output {output} of node {node} unavailable resolving an input of {consumer}"
    )]
    Unresolved {
        node: node::Id,
        output: usize,
        consumer: node::Id,
    },
}

/// Error when generating a module from a graph.
#[derive(Debug, Error)]
pub enum ModuleError {
    /// Error computing node connections while compiling the level at `path`.
    #[error("connection error at {}", level(path))]
    NodeConns {
        /// The level being compiled when the error arose.
        path: Vec<node::Id>,
        #[source]
        error: NodeConnsError,
    },
    /// Error lowering the level at `path` to the IR. Node ids carried by the
    /// underlying [`LowerError`] are relative to that level.
    #[error("failed to lower {}", level(path))]
    Lower {
        /// The level being lowered when the error arose.
        path: Vec<node::Id>,
        #[source]
        error: LowerError,
    },
    /// A nested graph was not found.
    #[error(transparent)]
    NestedGraphNotFound(#[from] NestedGraphNotFound),
    /// Multiple errors during meta collection.
    #[error(transparent)]
    MetaErrors(#[from] MetaErrors),
    /// Multiple errors during node function generation.
    #[error(transparent)]
    NodeFnErrors(#[from] NodeFnErrors),
    /// The lowering produced IR violating the compiler's own invariants.
    /// This is a bug in gantz, not in the compiled graph; validation runs on
    /// every lowering (unless disabled via [`Config::validate_ir`][super::Config])
    /// so it surfaces as a compile error rather than emitting malformed Steel.
    #[error("internal compiler error: invalid IR for {}: {detail}", level(path))]
    InvalidIr { path: Vec<node::Id>, detail: String },
}

impl fmt::Display for MetaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "node-{}: {}",
            super::names::path_string(&self.path),
            self.error
        )
    }
}

impl fmt::Display for MetaErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, err) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{err}")?;
        }
        Ok(())
    }
}

impl fmt::Display for NodeExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "node-{}: {}",
            super::names::path_string(&self.path),
            self.error
        )
    }
}

impl fmt::Display for NodeFnErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, err) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{err}")?;
        }
        Ok(())
    }
}

impl std::error::Error for MetaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl std::error::Error for MetaErrors {}

impl std::error::Error for NodeExprError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl std::error::Error for NodeFnErrors {}

/// Format a level path for error messages.
fn level(path: &[node::Id]) -> String {
    if path.is_empty() {
        "the root level".to_string()
    } else {
        format!("level {}", super::names::path_string(path))
    }
}

/// Format an optional node id for error messages.
fn at_node(node: &Option<node::Id>) -> String {
    node.map(|n| format!(" at node {n}")).unwrap_or_default()
}
