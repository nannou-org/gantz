//! VM utilities for initializing and compiling gantz graphs.
//!
//! This module provides:
//! - Convenience wrappers around `gantz_core::vm` (`init`, `compile`)
//! - Observers for VM initialization on head events (`on_head_opened`, `on_head_replaced`)
//! - Systems for VM setup and update (`setup`, `update`)

use crate::BuiltinNodes;
use crate::head::{
    CompiledModule, HeadCommitted, HeadOpened, HeadReplaced, HeadVms, OpenHead, OpenHeadData,
    WorkingGraph,
};
use crate::reg::{Registry, RegistryRef};
use crate::view::Views;
use bevy_ecs::prelude::*;
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::Node;
use gantz_core::node::{GetNode, graph::Graph};
use gantz_core::vm::CompileError;
use gantz_egui::GraphViews;
use steel::steel_vm::engine::Engine;

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

/// VM init for opened heads.
pub fn on_head_opened<N>(
    trigger: On<HeadOpened>,
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut vms: NonSendMut<HeadVms>,
    mut cmds: Commands,
    graphs: Query<&WorkingGraph<N>>,
) where
    N: Node + Send + Sync + 'static,
{
    let event = trigger.event();
    let graph = graphs.get(event.entity).unwrap();
    let node_reg = RegistryRef::new(&*registry, &*builtins);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    let (vm, module) = match init(&get_node, &**graph) {
        Ok(result) => result,
        Err(e) => {
            log::error!("Failed to init VM for new head: {e}");
            return;
        }
    };
    cmds.entity(event.entity).insert(CompiledModule(module));
    vms.insert(event.entity, vm);
}

/// VM init for replaced heads.
pub fn on_head_replaced<N>(
    trigger: On<HeadReplaced>,
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut vms: NonSendMut<HeadVms>,
    mut cmds: Commands,
    graphs: Query<&WorkingGraph<N>>,
) where
    N: Node + Send + Sync + 'static,
{
    let event = trigger.event();
    let graph = graphs.get(event.entity).unwrap();
    let node_reg = RegistryRef::new(&*registry, &*builtins);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    let (vm, module) = match init(&get_node, &**graph) {
        Ok(result) => result,
        Err(e) => {
            log::error!("Failed to init VM for replaced head: {e}");
            return;
        }
    };
    cmds.entity(event.entity).insert(CompiledModule(module));
    vms.insert(event.entity, vm);
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Initialize VMs for all open heads (exclusive startup system).
pub fn setup<N>(world: &mut World)
where
    N: Node + Send + Sync + 'static,
{
    log::info!("Setting up VMs for all open heads!");

    let entities: Vec<Entity> = world
        .query_filtered::<Entity, With<OpenHead>>()
        .iter(world)
        .collect();

    let mut vms = HeadVms::default();
    let mut compiled_updates: Vec<(Entity, String)> = vec![];
    for entity in entities {
        let registry = world.resource::<Registry<N>>();
        let builtins = world.resource::<BuiltinNodes<N>>();
        let node_reg = RegistryRef::new(registry, &*builtins);
        let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
        let Some(wg) = world.get::<WorkingGraph<N>>(entity) else {
            continue;
        };
        let (vm, module) = match init(&get_node, &**wg) {
            Ok(result) => result,
            Err(e) => {
                log::error!("Failed to init VM for entity {entity}: {e}");
                continue;
            }
        };
        vms.insert(entity, vm);
        compiled_updates.push((entity, module));
    }

    for (entity, compiled_module) in compiled_updates {
        if let Some(mut compiled) = world.get_mut::<CompiledModule>(entity) {
            *compiled = CompiledModule(compiled_module);
        }
    }

    world.insert_non_send_resource(vms);
}

/// Detect graph changes and recompile into VMs.
///
/// When a graph change is detected, this system:
/// - Commits the new graph to the registry
/// - Recompiles the VM
/// - Emits a [`HeadCommitted`] event for UI updates
pub fn update<N>(
    mut cmds: Commands,
    mut registry: ResMut<Registry<N>>,
    mut views: ResMut<Views>,
    builtins: Res<BuiltinNodes<N>>,
    mut vms: NonSendMut<HeadVms>,
    mut heads_query: Query<OpenHeadData<N>, With<OpenHead>>,
) where
    N: Node + Clone + ca::CaHash + Send + Sync + 'static,
{
    for mut data in heads_query.iter_mut() {
        let head: &mut ca::Head = &mut *data.head_ref;
        let graph: &Graph<N> = &*data.working_graph;
        let head_views: &GraphViews = &*data.views;

        if let Some(commit_addr) = registry.head_commit_ca(head).copied() {
            views.insert(commit_addr, head_views.clone());
        }

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
            cmds.trigger(HeadCommitted {
                entity: data.entity,
                old_head: old_head.clone(),
                new_head: head.clone(),
            });

            if let Some(vm) = vms.get_mut(&data.entity) {
                let node_reg = RegistryRef::new(&*registry, &*builtins);
                let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
                gantz_core::graph::register(&get_node, graph, &[], vm);
                match compile(&get_node, graph, vm) {
                    Ok(module) => data.compiled.0 = module,
                    Err(e) => log::error!("Failed to compile graph: {e}"),
                }
            }
        }
    }
}
