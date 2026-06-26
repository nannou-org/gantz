//! Entity-based head management for gantz.
//!
//! This module provides Bevy components and resources for managing open graph
//! heads as entities rather than parallel `Vec`s.

use crate::reg::Registry;
use bevy_ecs::{prelude::*, query::QueryData};
use bevy_log as log;
use gantz_ca as ca;
use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
};
use steel::steel_vm::engine::Engine;

// ----------------------------------------------------------------------------
// QueryData
// ----------------------------------------------------------------------------

/// QueryData for accessing open head components.
///
/// Simplifies Query type signatures by bundling head-related components.
/// Use with `Query<OpenHeadData<N>, With<OpenHead>>`.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct OpenHeadData<N: 'static + Send + Sync> {
    pub entity: Entity,
    pub head_ref: &'static mut HeadRef,
    pub working_graph: &'static mut WorkingGraph<N>,
    pub module: &'static mut Module,
    pub diagnostics: &'static mut Diagnostics,
    pub compiled_inputs: &'static mut crate::vm::CompiledInputs,
}

// ----------------------------------------------------------------------------
// Components
// ----------------------------------------------------------------------------

/// Marker component for an open gantz head entity.
///
/// Requires the compile-outcome components so every spawn path gets them by
/// default; `vm::sync` fills them in on the next `Update`.
#[derive(Component)]
#[require(Module, Diagnostics, crate::vm::CompiledInputs)]
pub struct OpenHead;

/// The gantz_ca::Head (branch or commit reference).
#[derive(Component, Clone)]
pub struct HeadRef(pub ca::Head);

/// The working copy of the graph associated with this head.
#[derive(Component)]
pub struct WorkingGraph<N>(pub gantz_core::node::graph::Graph<N>);

/// The latest compile outcome for this head.
///
/// `compiled` is kept even when steel rejected the generated module, so its
/// source remains displayable and error spans resolvable; it is `None` only
/// when module generation itself failed. Both fields can be present (a
/// generated module that failed evaluation).
#[derive(Component, Default)]
pub struct Module {
    /// The module artifact: source text + source map.
    pub compiled: Option<gantz_core::vm::Compiled>,
    /// The rendered error chain from a failed compile.
    pub error: Option<String>,
}

/// Diagnostics from the head's latest compile and entrypoint evaluations.
///
/// Compile diagnostics are replaced wholesale on every (re)compile; runtime
/// diagnostics are replaced per evaluation and cleared on success.
#[derive(Component, Default)]
pub struct Diagnostics(pub Vec<gantz_core::Diagnostic>);

// ----------------------------------------------------------------------------
// Events
// ----------------------------------------------------------------------------

/// Event to open a head as a new tab (or focus if already open).
#[derive(Event)]
pub struct OpenEvent(pub ca::Head);

/// Event to close a head tab.
#[derive(Event)]
pub struct CloseEvent(pub ca::Head);

/// Event to replace the focused head with a different head.
#[derive(Event)]
pub struct ReplaceEvent(pub ca::Head);

/// Event to create a new branch from an existing head.
#[derive(Event)]
pub struct BranchHeadEvent {
    pub original: ca::Head,
    pub new_name: String,
}

/// Event to move a branch's commit pointer to a different commit.
#[derive(Event)]
pub struct MoveBranchEvent {
    pub entity: Entity,
    pub name: ca::Branch,
    pub target: ca::CommitAddr,
}

// ----------------------------------------------------------------------------
// Hook Events (emitted after core operations for app-specific handling)
// ----------------------------------------------------------------------------

/// Emitted after a head has been opened.
#[derive(Event)]
pub struct OpenedEvent {
    pub entity: Entity,
    pub head: ca::Head,
}

/// Emitted after a head has been closed.
#[derive(Event)]
pub struct ClosedEvent {
    pub entity: Entity,
    pub head: ca::Head,
}

/// Emitted when a head's backing data has changed (replacement, branch move, etc.).
#[derive(Event)]
pub struct ChangedEvent {
    pub entity: Entity,
    pub old_head: ca::Head,
    pub new_head: ca::Head,
    /// Whether the new commit shares the previous commit's graph content
    /// address, i.e. only layout/metadata changed (e.g. a layout undo/redo).
    /// The VM and its node state are preserved across such a change.
    pub same_graph: bool,
}

/// Emitted after a branch has been created from a head.
#[derive(Event)]
pub struct BranchedHeadEvent {
    pub entity: Entity,
    pub old_head: ca::Head,
    pub new_head: ca::Head,
}

/// Emitted when a head's working graph is committed (graph changed).
///
/// This event is emitted by `vm::sync` when it detects a graph change
/// and commits to the registry. Apps can observe this to update UI state.
#[derive(Event)]
pub struct CommittedEvent {
    pub entity: Entity,
    pub old_head: ca::Head,
    pub new_head: ca::Head,
}

// ----------------------------------------------------------------------------
// Resources
// ----------------------------------------------------------------------------

/// Per-head VMs stored in a NonSend resource.
///
/// VMs are keyed by Entity ID since `Engine` is not `Send`.
///
/// A head's VM owns its graph's runtime node state. Paths that point a head
/// at a *different* graph (replace, branch move) remove the VM (and reset
/// [`vm::CompiledInputs`][crate::vm::CompiledInputs]) so that `vm::sync`
/// performs a fresh init, discarding the old graph's state; in-place edits
/// leave the VM so recompiles preserve it.
#[derive(Default)]
pub struct HeadVms(pub HashMap<Entity, Engine>);

/// The currently focused head entity.
#[derive(Resource, Default)]
pub struct FocusedHead(pub Option<Entity>);

/// Tab ordering for open heads (entities in display order).
#[derive(Resource, Default)]
pub struct HeadTabOrder(pub Vec<Entity>);

// ----------------------------------------------------------------------------
// Deref impls
// ----------------------------------------------------------------------------

impl Deref for HeadRef {
    type Target = ca::Head;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for HeadRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<N> Deref for WorkingGraph<N> {
    type Target = gantz_core::node::graph::Graph<N>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<N> DerefMut for WorkingGraph<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for HeadVms {
    type Target = HashMap<Entity, Engine>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for HeadVms {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for FocusedHead {
    type Target = Option<Entity>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FocusedHead {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for HeadTabOrder {
    type Target = Vec<Entity>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for HeadTabOrder {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

// ----------------------------------------------------------------------------
// Utility fns
// ----------------------------------------------------------------------------

/// Find the entity for the given head, if it exists.
pub fn find_entity(
    head: &ca::Head,
    heads: &Query<(Entity, &HeadRef), With<OpenHead>>,
) -> Option<Entity> {
    heads
        .iter()
        .find(|(_, head_ref)| &***head_ref == head)
        .map(|(entity, _)| entity)
}

/// Check if the given head is the currently focused head.
pub fn is_focused(
    head: &ca::Head,
    heads: &Query<(Entity, &HeadRef), With<OpenHead>>,
    focused: &FocusedHead,
) -> bool {
    find_entity(head, heads)
        .map(|entity| **focused == Some(entity))
        .unwrap_or(false)
}

// ----------------------------------------------------------------------------
// Event Handlers (Observers)
// ----------------------------------------------------------------------------

/// Handle request to open a head as a new tab (or focus if already open).
pub fn on_open<N>(
    trigger: On<OpenEvent>,
    mut cmds: Commands,
    registry: Res<Registry<N>>,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
    heads: Query<(Entity, &HeadRef), With<OpenHead>>,
) where
    N: 'static + Clone + Send + Sync,
{
    let OpenEvent(new_head) = trigger.event();

    // If already open, just focus it.
    if let Some(entity) = find_entity(new_head, &heads) {
        **focused = Some(entity);
        return;
    }

    // Get graph from registry.
    let Some(graph) = registry.head_graph(new_head).cloned() else {
        log::error!("cannot open head: graph missing from registry");
        return;
    };

    // Spawn the entity. `OpenHead`'s required components cover the compile
    // outcome; `vm::sync` initializes the VM on the next `Update`.
    // Note: HeadGuiState and GraphViews are added by GantzEguiPlugin observer.
    let entity = cmds
        .spawn((OpenHead, HeadRef(new_head.clone()), WorkingGraph(graph)))
        .id();

    tab_order.push(entity);
    **focused = Some(entity);

    // Emit hook for app to do VM init + GUI state + views.
    cmds.trigger(OpenedEvent {
        entity,
        head: new_head.clone(),
    });
}

/// Handle request to replace the focused head with a different head.
pub fn on_replace<N>(
    trigger: On<ReplaceEvent>,
    mut cmds: Commands,
    registry: Res<Registry<N>>,
    mut focused: ResMut<FocusedHead>,
    mut vms: NonSendMut<HeadVms>,
    heads: Query<(Entity, &HeadRef), With<OpenHead>>,
) where
    N: 'static + Clone + Send + Sync,
{
    let ReplaceEvent(new_head) = trigger.event();

    // If new head already open, just focus it.
    if let Some(entity) = find_entity(new_head, &heads) {
        **focused = Some(entity);
        return;
    }

    let Some(focused_entity) = **focused else {
        return;
    };
    let old_head = heads.get(focused_entity).ok().map(|(_, h)| (**h).clone());

    // Get new graph.
    let Some(graph) = registry.head_graph(new_head).cloned() else {
        log::error!("cannot replace head: graph missing from registry");
        return;
    };

    // When the new commit shares the old commit's graph content (a layout-only
    // change, e.g. a layout undo/redo), the graph is byte-identical, so keep the
    // VM and its node state and leave the compile memo intact (`vm::sync` then
    // skips recompilation). Only when the graph actually differs do we drop the
    // VM and reset the memo so `vm::sync` performs a fresh init.
    // Note: HeadGuiState and GraphViews are updated by GantzEguiPlugin observer.
    let old_graph = old_head
        .as_ref()
        .and_then(|h| registry.head_commit(h).map(|c| c.graph));
    let new_graph = registry.head_commit(new_head).map(|c| c.graph);
    let same_graph = matches!((old_graph, new_graph), (Some(a), Some(b)) if a == b);
    if same_graph {
        cmds.entity(focused_entity)
            .insert((HeadRef(new_head.clone()), WorkingGraph(graph)));
    } else {
        vms.remove(&focused_entity);
        cmds.entity(focused_entity).insert((
            HeadRef(new_head.clone()),
            WorkingGraph(graph),
            crate::vm::CompiledInputs::default(),
        ));
    }

    // Emit hook.
    if let Some(old) = old_head {
        cmds.trigger(ChangedEvent {
            entity: focused_entity,
            old_head: old,
            new_head: new_head.clone(),
            same_graph,
        });
    }
}

/// Handle request to close a head tab.
pub fn on_close<N>(
    trigger: On<CloseEvent>,
    mut cmds: Commands,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
    mut vms: NonSendMut<HeadVms>,
    heads: Query<(Entity, &HeadRef), With<OpenHead>>,
) where
    N: 'static + Send + Sync,
{
    let CloseEvent(head) = trigger.event();

    // Don't close if last head.
    if tab_order.len() <= 1 {
        return;
    }

    let Some(entity) = find_entity(head, &heads) else {
        return;
    };
    let Some(ix) = tab_order.iter().position(|&x| x == entity) else {
        return;
    };

    // Clean up VM for this head.
    vms.remove(&entity);

    cmds.entity(entity).despawn();
    tab_order.retain(|&x| x != entity);

    // Update focus to remain valid.
    if **focused == Some(entity) {
        let new_ix = ix.saturating_sub(1).min(tab_order.len().saturating_sub(1));
        **focused = tab_order.get(new_ix).copied();
    }

    cmds.trigger(ClosedEvent {
        entity,
        head: head.clone(),
    });
}

/// Handle request to create a new branch from an existing head.
pub fn on_branch_head<N>(
    trigger: On<BranchHeadEvent>,
    mut cmds: Commands,
    mut registry: ResMut<Registry<N>>,
    mut heads: Query<(Entity, &mut HeadRef), With<OpenHead>>,
) where
    N: 'static + Send + Sync,
{
    let BranchHeadEvent { original, new_name } = trigger.event();

    // Get commit CA from original head.
    let Some(commit_ca) = registry.head_commit_ca(original).copied() else {
        log::error!("Failed to get commit address for head: {:?}", original);
        return;
    };

    // Create a new commit pointing to the same graph so the new branch gets
    // its own independent `CommitAddr` (and therefore its own views/layout).
    let graph_addr = registry.commits()[&commit_ca].graph;
    let new_commit_ca =
        registry.commit_graph(crate::reg::timestamp(), Some(commit_ca), graph_addr, || {
            unreachable!("graph already exists in registry")
        });

    // Insert new branch name pointing to the fresh commit.
    registry.insert_name(new_name.clone(), new_commit_ca);

    // Find and update the entity.
    let new_head = ca::Head::Branch(new_name.clone());
    for (entity, mut head_ref) in heads.iter_mut() {
        if &**head_ref == original {
            let old_head = (**head_ref).clone();
            **head_ref = new_head.clone();

            cmds.trigger(BranchedHeadEvent {
                entity,
                old_head,
                new_head,
            });
            break;
        }
    }
}

/// Handle request to move a branch's commit pointer to a different commit.
///
/// Atomically updates the registry, WorkingGraph, and emits ChangedEvent
/// within the command flush to prevent inconsistent state between systems.
pub fn on_move_branch<N>(
    trigger: On<MoveBranchEvent>,
    mut cmds: Commands,
    mut registry: ResMut<Registry<N>>,
    mut vms: NonSendMut<HeadVms>,
) where
    N: 'static + Clone + Send + Sync,
{
    let event = trigger.event();
    let head = ca::Head::Branch(event.name.clone());
    // Capture the current graph before moving the branch pointer so we can
    // detect a same-graph move (a layout-only undo/redo) below.
    let old_graph = registry.head_commit(&head).map(|c| c.graph);
    registry.insert_name(event.name.clone(), event.target);
    let Some(graph) = registry.head_graph(&head).cloned() else {
        log::error!("MoveBranch: graph missing for target commit");
        return;
    };
    // When the target shares the old commit's graph content (a layout-only
    // change), keep the VM, its node state and the compile memo intact so
    // `vm::sync` skips recompilation. Otherwise drop the VM and reset the memo
    // so `vm::sync` performs a fresh init.
    let new_graph = registry.head_commit(&head).map(|c| c.graph);
    let same_graph = matches!((old_graph, new_graph), (Some(a), Some(b)) if a == b);
    if same_graph {
        cmds.entity(event.entity).insert(WorkingGraph(graph));
    } else {
        vms.remove(&event.entity);
        cmds.entity(event.entity)
            .insert((WorkingGraph(graph), crate::vm::CompiledInputs::default()));
    }
    cmds.trigger(ChangedEvent {
        entity: event.entity,
        old_head: head.clone(),
        new_head: head,
        same_graph,
    });
}
