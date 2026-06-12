//! VM utilities for initializing and compiling gantz graphs.
//!
//! This module provides:
//! - Convenience wrappers around `gantz_core::vm` (`init`, `compile`)
//! - Evaluation events and observer (`EvalEntryEvent`, `on_eval_entry`)
//! - Observers for VM initialization on head events (`on_head_opened`, `on_head_changed`)
//! - Systems for VM setup and update (`setup`, `update`)

use crate::BuiltinNodes;
use crate::head;
use crate::reg::{Registry, lookup_node};
use bevy_ecs::prelude::*;
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::node::{self, GetNode, graph::Graph};
use gantz_core::vm::{Compiled, CompileError};
use gantz_core::{Node, compile as core_compile, diagnostic};
use std::time::Duration;
use steel::steel_vm::engine::Engine;

/// The component updates for one compile attempt: the module/error outcome
/// and the extracted compile diagnostics.
fn compile_components(
    result: Result<Compiled, CompileError>,
) -> (head::Module, head::Diagnostics) {
    match result {
        Ok(module) => (
            head::Module {
                compiled: Some(module),
                error: None,
            },
            head::Diagnostics(vec![]),
        ),
        Err(e) => {
            let error = gantz_core::vm::error_chain(&e);
            log::error!("Failed to compile graph: {error}");
            let diags = diagnostic::from_compile_error(&e);
            let module = head::Module {
                compiled: e.into_module(),
                error: Some(error),
            };
            (module, head::Diagnostics(diags))
        }
    }
}

/// A function that produces entrypoints for a given graph.
pub type EntrypointFn<N> = Box<
    dyn for<'a> Fn(node::GetNode<'a>, &Graph<N>) -> Vec<core_compile::Entrypoint> + Send + Sync,
>;

/// Resource holding all entrypoint provider functions.
///
/// Each provider is called during compilation to collect entrypoints.
/// `GantzPlugin` registers `push_pull_entrypoints` by default.
/// Downstream plugins (e.g. `GantzEguiPlugin`) push additional providers.
#[derive(Resource)]
pub struct EntrypointFns<N: 'static>(pub Vec<EntrypointFn<N>>);

impl<N: 'static> Default for EntrypointFns<N> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

/// Collect all entrypoints by calling each provider fn in the resource.
fn collect_entrypoints<N: Node>(
    ep_fns: &EntrypointFns<N>,
    get_node: GetNode<'_>,
    graph: &Graph<N>,
) -> Vec<core_compile::Entrypoint> {
    ep_fns.0.iter().flat_map(|f| f(get_node, graph)).collect()
}

/// Resource holding the [`core_compile::Config`] used whenever a head's graph
/// is (re)compiled into its VM.
///
/// Defaults to the core defaults. Override (and trigger a recompile) to e.g.
/// enable `emit_all_node_fns` when debugging codegen in the module view.
#[derive(Default, Resource)]
pub struct CompileConfig(pub core_compile::Config);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Event to trigger evaluation of an entrypoint.
#[derive(Event)]
pub struct EvalEntryEvent {
    /// The head entity to evaluate on.
    pub head: Entity,
    /// The entrypoint to evaluate.
    pub entrypoint: core_compile::Entrypoint,
}

/// Emitted after VM evaluation completes, for timing capture.
///
/// This event allows UI layers (like `bevy_gantz_egui`) to observe VM execution
/// timing without the core crate depending on UI-related types.
#[derive(Event)]
pub struct EvalEntryComplete {
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
/// Returns the initialized VM and the compiled module.
pub fn init<N>(
    get_node: GetNode,
    graph: &Graph<N>,
    entrypoints: &[core_compile::Entrypoint],
    config: &core_compile::Config,
) -> Result<(Engine, Compiled), CompileError>
where
    N: Node,
{
    gantz_core::vm::init(get_node, graph, entrypoints, config)
}

/// Compile the graph into a Steel module and run it in the VM.
pub fn compile<N>(
    get_node: GetNode,
    graph: &Graph<N>,
    vm: &mut Engine,
    entrypoints: &[core_compile::Entrypoint],
    config: &core_compile::Config,
) -> Result<Compiled, CompileError>
where
    N: Node,
{
    gantz_core::vm::compile(get_node, graph, vm, entrypoints, config)
}

// ---------------------------------------------------------------------------
// Observers
// ---------------------------------------------------------------------------

/// Initialize (or reinitialize) the VM for the given head entity.
fn init_head_vm<N>(
    entity: Entity,
    registry: &Registry<N>,
    builtins: &BuiltinNodes<N>,
    ep_fns: &EntrypointFns<N>,
    config: &CompileConfig,
    vms: &mut head::HeadVms,
    cmds: &mut Commands,
    graphs: &Query<&head::WorkingGraph<N>>,
) where
    N: 'static + Node + Send + Sync,
{
    let graph = graphs.get(entity).unwrap();
    let get_node = |ca: &ca::ContentAddr| lookup_node(registry, &**builtins, ca);
    let entrypoints = collect_entrypoints(ep_fns, &get_node, &**graph);
    match init(&get_node, &**graph, &entrypoints, &config.0) {
        Ok((vm, module)) => {
            cmds.entity(entity).insert(compile_components(Ok(module)));
            vms.insert(entity, vm);
        }
        Err(e) => {
            // Don't leave a stale VM/module from a previously-active graph in
            // place: surface the error in the compiled module and drop the VM so
            // eval systems (e.g. `drive_frame_bangs`, `on_eval_entry`) stop
            // driving the wrong graph - which otherwise manifests as a confusing
            // "free identifier: entry-fn-..." against an unrelated module.
            cmds.entity(entity).insert(compile_components(Err(e)));
            vms.remove(&entity);
        }
    }
}

/// VM init for opened heads.
pub fn on_head_opened<N>(
    trigger: On<head::OpenedEvent>,
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    ep_fns: Res<EntrypointFns<N>>,
    config: Res<CompileConfig>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    graphs: Query<&head::WorkingGraph<N>>,
) where
    N: 'static + Node + Send + Sync,
{
    init_head_vm(
        trigger.event().entity,
        &registry,
        &builtins,
        &ep_fns,
        &config,
        &mut vms,
        &mut cmds,
        &graphs,
    );
}

/// VM init for changed heads.
pub fn on_head_changed<N>(
    trigger: On<head::ChangedEvent>,
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    ep_fns: Res<EntrypointFns<N>>,
    config: Res<CompileConfig>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    graphs: Query<&head::WorkingGraph<N>>,
) where
    N: 'static + Node + Send + Sync,
{
    init_head_vm(
        trigger.event().entity,
        &registry,
        &builtins,
        &ep_fns,
        &config,
        &mut vms,
        &mut cmds,
        &graphs,
    );
}

/// Observer that handles evaluation events by calling the appropriate VM function.
///
/// Emits an `EvalEntryComplete` event with timing information for UI layers to observe.
pub fn on_eval_entry(
    trigger: On<EvalEntryEvent>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<(&head::Module, &mut head::Diagnostics)>,
) {
    let event = trigger.event();
    let fn_name = core_compile::entry_fn_name(&event.entrypoint.id());
    if let Some(vm) = vms.get_mut(&event.head) {
        let start = web_time::Instant::now();
        let result = vm.call_function_by_name_with_args(&fn_name, vec![]);
        // Runtime diagnostics reflect the latest evaluation only.
        if let Ok((module, mut diagnostics)) = heads.get_mut(event.head) {
            diagnostics
                .0
                .retain(|d| d.severity != diagnostic::Severity::Runtime);
            if let (Err(e), Some(compiled)) = (&result, &module.compiled) {
                diagnostics.0.push(diagnostic::from_eval_error(e, vm, compiled));
            }
        }
        if let Err(e) = result {
            log::error!("{e}");
        }
        cmds.trigger(EvalEntryComplete {
            entity: event.head,
            duration: start.elapsed(),
        });
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Initialize VMs for all open heads (exclusive startup system).
pub fn setup<N>(world: &mut World)
where
    N: 'static + Node + Send + Sync,
{
    log::info!("Setting up VMs for all open heads!");

    let entities: Vec<Entity> = world
        .query_filtered::<Entity, With<head::OpenHead>>()
        .iter(world)
        .collect();

    let mut vms = head::HeadVms::default();
    let mut compiled_updates: Vec<(Entity, Compiled)> = vec![];
    for entity in entities {
        let registry = world.resource::<Registry<N>>();
        let builtins = world.resource::<BuiltinNodes<N>>();
        let ep_fns = world.resource::<EntrypointFns<N>>();
        let config = world.resource::<CompileConfig>();
        let get_node = |ca: &ca::ContentAddr| lookup_node(registry, &**builtins, ca);
        let Some(wg) = world.get::<head::WorkingGraph<N>>(entity) else {
            continue;
        };
        let entrypoints = collect_entrypoints(ep_fns, &get_node, &**wg);
        let (vm, module) = match init(&get_node, &**wg, &entrypoints, &config.0) {
            Ok(result) => result,
            Err(e) => {
                log::error!("Failed to init VM for entity {entity}: {e}");
                continue;
            }
        };
        vms.insert(entity, vm);
        compiled_updates.push((entity, module));
    }

    for (entity, module) in compiled_updates {
        world.entity_mut(entity).insert(compile_components(Ok(module)));
    }

    world.insert_non_send_resource(vms);
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
    ep_fns: Res<EntrypointFns<N>>,
    config: Res<CompileConfig>,
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
                let entrypoints = collect_entrypoints(&ep_fns, &get_node, graph);
                // On error this surfaces commented error text rather than
                // leaving the prior module displayed, which would
                // misleadingly look up-to-date.
                let result = compile(&get_node, graph, vm, &entrypoints, &config.0);
                let (module, diagnostics) = compile_components(result);
                *data.module = module;
                *data.diagnostics = diagnostics;
            }
        }
    }
}

/// Recompile every open head's graph into its existing VM.
///
/// Runs when [`CompileConfig`] changes (see `GantzPlugin`). Unlike [`update`],
/// this never commits - the graph content is unchanged, only the codegen
/// options - and compiling into the existing VM preserves node state.
pub fn recompile_all<N>(
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    ep_fns: Res<EntrypointFns<N>>,
    config: Res<CompileConfig>,
    mut vms: NonSendMut<head::HeadVms>,
    mut heads_query: Query<head::OpenHeadData<N>, With<head::OpenHead>>,
) where
    N: 'static + Node + Send + Sync,
{
    for mut data in heads_query.iter_mut() {
        let graph: &Graph<N> = &*data.working_graph;
        let Some(vm) = vms.get_mut(&data.entity) else {
            continue;
        };
        let get_node = |ca: &ca::ContentAddr| lookup_node(&registry, &**builtins, ca);
        gantz_core::graph::register(&get_node, graph, &[], vm);
        let entrypoints = collect_entrypoints(&ep_fns, &get_node, graph);
        let result = compile(&get_node, graph, vm, &entrypoints, &config.0);
        let (module, diagnostics) = compile_components(result);
        *data.module = module;
        *data.diagnostics = diagnostics;
    }
}
