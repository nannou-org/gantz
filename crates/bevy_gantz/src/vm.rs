//! VM utilities for initializing and compiling gantz graphs.
//!
//! This module provides:
//! - Convenience wrappers around `gantz_core::vm` (`init`, `compile`)
//! - Evaluation events and observer (`EvalEvent`, `EvalKind`, `on_eval`)
//! - Observers for VM initialization on head events (`on_head_opened`, `on_head_changed`)
//! - Systems for VM setup and update (`setup`, `update`)

use crate::BuiltinNodes;
use crate::head;
use crate::reg::{Registry, lookup_node};
use crate::state;
use bevy_ecs::prelude::*;
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::node::{self, GetNode, graph::Graph};
use gantz_core::vm::CompileError;
use gantz_core::{Node, compile as core_compile};
use std::time::Duration;
use steel::steel_vm::engine::Engine;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

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

/// Emitted after VM evaluation completes, for timing capture.
///
/// This event allows UI layers (like `bevy_gantz_egui`) to observe VM execution
/// timing without the core crate depending on UI-related types.
#[derive(Event)]
pub struct EvalCompleted {
    /// The head entity that was evaluated.
    pub entity: Entity,
    /// The duration of the VM execution.
    pub duration: Duration,
}

// ---------------------------------------------------------------------------
// Core VM utilities
// ---------------------------------------------------------------------------

/// Initialize a new VM with root state and register the given graph.
///
/// Returns the initialized VM and the compiled module as a formatted string.
pub fn init<N>(get_node: GetNode, graph: &Graph<N>) -> Result<(Engine, String), CompileError>
where
    N: Node,
{
    let (vm, module) = gantz_core::vm::init(get_node, graph)?;
    Ok((vm, gantz_core::vm::fmt_module(&module)))
}

/// Compile the graph into a Steel module and run it in the VM.
///
/// Returns the compiled module as a formatted string.
pub fn compile<N>(
    get_node: GetNode,
    graph: &Graph<N>,
    vm: &mut Engine,
) -> Result<String, CompileError>
where
    N: Node,
{
    let module = gantz_core::vm::compile(get_node, graph, vm)?;
    Ok(gantz_core::vm::fmt_module(&module))
}

// ---------------------------------------------------------------------------
// Observers
// ---------------------------------------------------------------------------

/// Initialize (or reinitialize) the VM for the given head entity.
fn init_head_vm<N>(
    entity: Entity,
    registry: &Registry<N>,
    builtins: &BuiltinNodes<N>,
    vms: &mut head::HeadVms,
    cmds: &mut Commands,
    graphs: &Query<&head::WorkingGraph<N>>,
) where
    N: 'static + Node + Send + Sync,
{
    let graph = graphs.get(entity).unwrap();
    let get_node = |ca: &ca::ContentAddr| lookup_node(registry, &**builtins, ca);
    let (vm, module) = match init(&get_node, &**graph) {
        Ok(result) => result,
        Err(e) => {
            log::error!("Failed to init VM for head: {e}");
            return;
        }
    };
    cmds.entity(entity).insert(head::CompiledModule(module));
    vms.insert(entity, vm);
}

/// VM init for opened heads.
pub fn on_head_opened<N>(
    trigger: On<head::OpenedEvent>,
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    graphs: Query<&head::WorkingGraph<N>>,
    states: Res<state::States>,
    config: Res<state::PersistStateConfig>,
) where
    N: 'static + Node + Send + Sync,
{
    let event = trigger.event();
    init_head_vm(
        event.entity,
        &registry,
        &builtins,
        &mut vms,
        &mut cmds,
        &graphs,
    );
    state::restore_for_head(
        &event.head,
        event.entity,
        &registry,
        &states,
        &config,
        &mut vms,
    );
}

/// VM init for changed heads.
pub fn on_head_changed<N>(
    trigger: On<head::ChangedEvent>,
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    graphs: Query<&head::WorkingGraph<N>>,
    states: Res<state::States>,
    config: Res<state::PersistStateConfig>,
) where
    N: 'static + Node + Send + Sync,
{
    let event = trigger.event();
    init_head_vm(
        event.entity,
        &registry,
        &builtins,
        &mut vms,
        &mut cmds,
        &graphs,
    );
    state::restore_for_head(
        &event.new_head,
        event.entity,
        &registry,
        &states,
        &config,
        &mut vms,
    );
}

/// Observer that handles evaluation events by calling the appropriate VM function.
///
/// Emits an `EvalCompleted` event with timing information for UI layers to observe.
pub fn on_eval(trigger: On<EvalEvent>, mut vms: NonSendMut<head::HeadVms>, mut cmds: Commands) {
    let event = trigger.event();
    let fn_name = match event.kind {
        EvalKind::Push => core_compile::push_eval_fn_name(&event.path),
        EvalKind::Pull => core_compile::pull_eval_fn_name(&event.path),
    };
    if let Some(vm) = vms.get_mut(&event.head) {
        let start = web_time::Instant::now();
        if let Err(e) = vm.call_function_by_name_with_args(&fn_name, vec![]) {
            log::error!("{e}");
        }
        cmds.trigger(EvalCompleted {
            entity: event.head,
            duration: start.elapsed(),
        });
        cmds.trigger(state::PersistEvent { head: event.head });
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Trigger [`head::OpenedEvent`] for all open heads so that the existing
/// observer chain handles VM initialization and state restoration.
pub fn setup(heads: Query<(Entity, &head::HeadRef), With<head::OpenHead>>, mut cmds: Commands) {
    for (entity, head_ref) in heads.iter() {
        cmds.trigger(head::OpenedEvent {
            entity,
            head: (**head_ref).clone(),
        });
    }
}

/// Detect graph changes and recompile into VMs.
///
/// When a graph change is detected, this system:
/// - Commits the new graph to the registry
/// - Recompiles the VM
/// - Emits a [`head::CommittedEvent`] for UI updates
pub fn update<N>(
    mut cmds: Commands,
    mut registry: ResMut<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut vms: NonSendMut<head::HeadVms>,
    mut heads_query: Query<head::OpenHeadData<N>, With<head::OpenHead>>,
) where
    N: 'static + Node + Clone + ca::CaHash + Send + Sync,
{
    for mut data in heads_query.iter_mut() {
        let head: &mut ca::Head = &mut *data.head_ref;
        let graph: &Graph<N> = &*data.working_graph;

        let new_graph_ca = ca::graph_addr(graph);
        let Some(head_commit) = registry.head_commit(head) else {
            continue;
        };
        if head_commit.graph != new_graph_ca {
            let old_head = head.clone();
            let old_commit_ca = registry.head_commit_ca(head).copied().unwrap();
            let new_commit_ca = registry.commit_graph_to_head(
                crate::reg::timestamp(),
                new_graph_ca,
                || crate::clone_graph(graph),
                head,
            );
            log::debug!(
                "Graph changed: {} -> {}",
                old_commit_ca.display_short(),
                new_commit_ca.display_short()
            );

            // Emit event for UI state updates (handled by GantzEguiPlugin if present).
            cmds.trigger(head::CommittedEvent {
                entity: data.entity,
                old_head: old_head.clone(),
                new_head: head.clone(),
            });

            if let Some(vm) = vms.get_mut(&data.entity) {
                let get_node = |ca: &ca::ContentAddr| lookup_node(&registry, &**builtins, ca);
                gantz_core::graph::register(&get_node, graph, &[], vm);
                match compile(&get_node, graph, vm) {
                    Ok(module) => data.compiled.0 = module,
                    Err(e) => log::error!("Failed to compile graph: {e}"),
                }
            }
        }
    }
}
