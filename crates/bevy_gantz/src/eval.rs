//! Evaluation events for gantz graphs.

use bevy_ecs::prelude::*;
use gantz_core::node;

/// The kind of evaluation to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalKind {
    /// Push evaluation: propagate values forward from sources.
    Push,
    /// Pull evaluation: request values backward from sinks.
    Pull,
}

/// Event to trigger evaluation of a node path.
#[derive(Event)]
pub struct EvalEvent {
    /// The head entity to evaluate on.
    pub head: Entity,
    /// The path to the node/subgraph to evaluate.
    pub path: Vec<node::Id>,
    /// The kind of evaluation (push or pull).
    pub kind: EvalKind,
}
