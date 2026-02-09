//! Evaluation events for gantz graphs.

use crate::head::HeadVms;
use bevy_ecs::prelude::*;
use bevy_log as log;
use gantz_core::{compile, node};
use std::time::Duration;

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

/// Emitted after VM execution completes, for timing capture.
///
/// This event allows UI layers (like `bevy_gantz_egui`) to observe VM execution
/// timing without the core crate depending on UI-related types.
#[derive(Event)]
pub struct VmExecCompleted {
    /// The head entity that was evaluated.
    pub entity: Entity,
    /// The duration of the VM execution.
    pub duration: Duration,
}

/// Observer that handles evaluation events by calling the appropriate VM function.
///
/// Emits a `VmExecCompleted` event with timing information for UI layers to observe.
pub fn on_eval_event(trigger: On<EvalEvent>, mut vms: NonSendMut<HeadVms>, mut cmds: Commands) {
    let event = trigger.event();
    let fn_name = match event.kind {
        EvalKind::Push => compile::push_eval_fn_name(&event.path),
        EvalKind::Pull => compile::pull_eval_fn_name(&event.path),
    };
    if let Some(vm) = vms.get_mut(&event.head) {
        let start = web_time::Instant::now();
        if let Err(e) = vm.call_function_by_name_with_args(&fn_name, vec![]) {
            log::error!("{e}");
        }
        cmds.trigger(VmExecCompleted {
            entity: event.head,
            duration: start.elapsed(),
        });
    }
}
