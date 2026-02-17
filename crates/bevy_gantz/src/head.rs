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
    pub compiled: &'static mut CompiledModule,
}

// ----------------------------------------------------------------------------
// Components
// ----------------------------------------------------------------------------

/// Marker component for an open gantz head entity.
#[derive(Component)]
pub struct OpenHead;

/// The gantz_ca::Head (branch or commit reference).
#[derive(Component, Clone)]
pub struct HeadRef(pub ca::Head);

/// The working copy of the graph associated with this head.
#[derive(Component)]
pub struct WorkingGraph<N>(pub gantz_core::node::graph::Graph<N>);

/// The compiled Steel module for this head (as a string).
#[derive(Component, Default, Clone)]
pub struct CompiledModule(pub String);

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
pub struct BranchEvent {
    pub original: ca::Head,
    pub new_name: String,
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

/// Emitted after a head has been replaced.
#[derive(Event)]
pub struct ReplacedEvent {
    pub entity: Entity,
    pub old_head: ca::Head,
    pub new_head: ca::Head,
}

/// Emitted after a branch has been created from a head.
#[derive(Event)]
pub struct BranchedEvent {
    pub entity: Entity,
    pub old_head: ca::Head,
    pub new_head: ca::Head,
}

/// Emitted when a head's working graph is committed (graph changed).
///
/// This event is emitted by `vm::update` when it detects a graph change
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

impl Deref for CompiledModule {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
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

    // Spawn entity (NO CompiledModule - app observer adds it after VM init).
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

    // Update entity components (NO CompiledModule - app observer adds it).
    // Note: HeadGuiState and GraphViews are updated by GantzEguiPlugin observer.
    cmds.entity(focused_entity)
        .insert(HeadRef(new_head.clone()))
        .insert(WorkingGraph(graph));

    // Emit hook.
    if let Some(old) = old_head {
        cmds.trigger(ReplacedEvent {
            entity: focused_entity,
            old_head: old,
            new_head: new_head.clone(),
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
pub fn on_branch<N>(
    trigger: On<BranchEvent>,
    mut cmds: Commands,
    mut registry: ResMut<Registry<N>>,
    mut heads: Query<(Entity, &mut HeadRef), With<OpenHead>>,
) where
    N: 'static + Send + Sync,
{
    let BranchEvent { original, new_name } = trigger.event();

    // Get commit CA from original head.
    let Some(commit_ca) = registry.head_commit_ca(original).copied() else {
        log::error!("Failed to get commit address for head: {:?}", original);
        return;
    };

    // Insert new branch name.
    registry.insert_name(new_name.clone(), commit_ca);

    // Find and update the entity.
    let new_head = ca::Head::Branch(new_name.clone());
    for (entity, mut head_ref) in heads.iter_mut() {
        if &**head_ref == original {
            let old_head = (**head_ref).clone();
            **head_ref = new_head.clone();

            cmds.trigger(BranchedEvent {
                entity,
                old_head,
                new_head,
            });
            break;
        }
    }
}
