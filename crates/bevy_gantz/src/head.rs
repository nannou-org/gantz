//! Entity-based head management for gantz.
//!
//! This module provides Bevy components and resources for managing open graph
//! heads as entities rather than parallel `Vec`s.

use bevy_ecs::{prelude::*, query::QueryData};
use gantz_ca as ca;
use gantz_egui::HeadDataMut;
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
pub struct OpenHeadData<N: Send + Sync + 'static> {
    pub entity: Entity,
    pub head_ref: &'static mut HeadRef,
    pub working_graph: &'static mut WorkingGraph<N>,
    pub views: &'static mut GraphViews,
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

/// View state (layout + camera) for a graph and all its nested subgraphs.
#[derive(Component, Default, Clone)]
pub struct GraphViews(pub gantz_egui::GraphViews);

/// The compiled Steel module for this head (as a string).
#[derive(Component, Default, Clone)]
pub struct CompiledModule(pub String);

/// Per-head GUI state (path, scene interaction, queued commands).
#[derive(Component, Default)]
pub struct HeadGuiState(pub gantz_egui::widget::gantz::OpenHeadState);

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
pub struct CreateBranchEvent {
    pub original: ca::Head,
    pub new_name: String,
}

// ----------------------------------------------------------------------------
// Hook Events (emitted after core operations for app-specific handling)
// ----------------------------------------------------------------------------

/// Emitted after a head has been opened.
#[derive(Event)]
pub struct HeadOpened {
    pub entity: Entity,
    pub head: ca::Head,
}

/// Emitted after a head has been closed.
#[derive(Event)]
pub struct HeadClosed {
    pub entity: Entity,
    pub head: ca::Head,
}

/// Emitted after a head has been replaced.
#[derive(Event)]
pub struct HeadReplaced {
    pub entity: Entity,
    pub old_head: ca::Head,
    pub new_head: ca::Head,
}

/// Emitted after a branch has been created from a head.
#[derive(Event)]
pub struct BranchCreated {
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

impl Deref for GraphViews {
    type Target = gantz_egui::GraphViews;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GraphViews {
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

impl Deref for HeadGuiState {
    type Target = gantz_egui::widget::gantz::OpenHeadState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for HeadGuiState {
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
// HeadAccess
// ----------------------------------------------------------------------------

/// Provides [`gantz_egui::HeadAccess`] implementation for Bevy ECS.
///
/// This struct wraps the necessary Bevy queries and resources to implement
/// the `HeadAccess` trait, allowing the gantz_egui widget to access head data
/// without knowing about Bevy's ECS.
///
/// Lifetime parameters:
/// - `'q` - the borrow lifetime of the query and vms references
/// - `'w` - the world lifetime from the Query
/// - `'s` - the state lifetime from the Query
pub struct HeadAccess<'q, 'w, 's, N: Send + Sync + 'static> {
    /// Heads in tab order, pre-collected.
    heads: Vec<ca::Head>,
    /// Map from head to entity for lookup.
    head_to_entity: HashMap<ca::Head, Entity>,
    /// Query for accessing head data mutably.
    query: &'q mut Query<'w, 's, OpenHeadData<N>, With<OpenHead>>,
    /// The VMs keyed by entity.
    vms: &'q mut HeadVms,
}

impl<'q, 'w, 's, N: Send + Sync + 'static> HeadAccess<'q, 'w, 's, N> {
    pub fn new(
        tab_order: &HeadTabOrder,
        query: &'q mut Query<'w, 's, OpenHeadData<N>, With<OpenHead>>,
        vms: &'q mut HeadVms,
    ) -> Self {
        // Pre-collect heads in tab order and build entity lookup.
        let mut heads = Vec::new();
        let mut head_to_entity = HashMap::new();

        for &entity in tab_order.iter() {
            if let Ok(data) = query.get(entity) {
                let head: ca::Head = (**data.head_ref).clone();
                heads.push(head.clone());
                head_to_entity.insert(head, entity);
            }
        }

        Self {
            heads,
            head_to_entity,
            query,
            vms,
        }
    }
}

impl<'q, 'w, 's, N: Send + Sync + 'static> gantz_egui::HeadAccess for HeadAccess<'q, 'w, 's, N> {
    type Node = N;

    fn heads(&self) -> &[ca::Head] {
        &self.heads
    }

    fn with_head_mut<R>(
        &mut self,
        head: &ca::Head,
        f: impl FnOnce(HeadDataMut<'_, Self::Node>) -> R,
    ) -> Option<R> {
        let entity = *self.head_to_entity.get(head)?;
        let mut data = self.query.get_mut(entity).ok()?;
        let vm = self.vms.get_mut(&entity)?;
        Some(f(HeadDataMut {
            graph: &mut *data.working_graph,
            views: &mut *data.views,
            vm,
        }))
    }

    fn compiled_module(&self, head: &ca::Head) -> Option<&str> {
        let entity = *self.head_to_entity.get(head)?;
        let data = self.query.get(entity).ok()?;
        Some(&*data.compiled)
    }
}

// ----------------------------------------------------------------------------
// Utility fns
// ----------------------------------------------------------------------------

/// Find the entity for the given head, if it exists.
pub fn find_head_entity(
    head: &ca::Head,
    heads: &Query<(Entity, &HeadRef), With<OpenHead>>,
) -> Option<Entity> {
    heads
        .iter()
        .find(|(_, head_ref)| &***head_ref == head)
        .map(|(entity, _)| entity)
}

/// Check if the given head is the currently focused head.
pub fn is_head_focused(
    head: &ca::Head,
    heads: &Query<(Entity, &HeadRef), With<OpenHead>>,
    focused: &FocusedHead,
) -> bool {
    find_head_entity(head, heads)
        .map(|entity| **focused == Some(entity))
        .unwrap_or(false)
}
