//! Error types for the compile module.

use crate::node;
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

/// Error when generating a module from a graph.
#[derive(Debug, Error)]
pub enum ModuleError {
    /// Error computing node connections.
    #[error(transparent)]
    NodeConns(#[from] NodeConnsError),
}
