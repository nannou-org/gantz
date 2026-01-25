//! Error types for the compile module.

use crate::node;
use std::fmt;
use thiserror::Error;

/// Too many connections for a node (exceeds [`node::Conns::MAX`]).
#[derive(Debug, Error)]
#[error("too many connections ({0}), max is {max}", max = node::Conns::MAX)]
pub struct TooManyConns(pub usize);

/// An edge references an invalid input index.
#[derive(Debug, Error)]
#[error("edge references invalid input index {index} (node has {n_inputs} inputs)")]
pub struct InvalidInputIndex {
    pub index: usize,
    pub n_inputs: usize,
}

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

/// Error when computing node connections from graph edges.
#[derive(Debug, Error)]
pub enum NodeConnsError {
    /// The node has too many connections.
    #[error(transparent)]
    TooManyConns(#[from] TooManyConns),
    /// An edge references an invalid input index.
    #[error(transparent)]
    InvalidInputIndex(#[from] InvalidInputIndex),
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

/// Error during code generation.
#[derive(Debug, Error)]
pub enum CodegenError {
    /// The node has too many inputs.
    #[error(transparent)]
    TooManyInputs(#[from] TooManyConns),
    /// An edge references an invalid input index.
    #[error(transparent)]
    InvalidInputIndex(#[from] InvalidInputIndex),
}

/// Error when generating a module from a graph.
#[derive(Debug, Error)]
pub enum ModuleError {
    /// Error computing node connections.
    #[error(transparent)]
    NodeConns(#[from] NodeConnsError),
    /// Error during code generation.
    #[error(transparent)]
    Codegen(#[from] CodegenError),
    /// A nested graph was not found.
    #[error(transparent)]
    NestedGraphNotFound(#[from] NestedGraphNotFound),
    /// Multiple errors during meta collection.
    #[error(transparent)]
    MetaErrors(#[from] MetaErrors),
}

impl fmt::Display for MetaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "node-{}: {}",
            super::codegen::path_string(&self.path),
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

impl std::error::Error for MetaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl std::error::Error for MetaErrors {}
