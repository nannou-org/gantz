//! Node state persistence.
//!
//! Provides:
//! - [`States`] — serialized node state snapshots keyed by commit address
//! - [`PersistStateConfig`] — names of graphs with state persistence enabled
//! - [`PersistEvent`] — trigger for persisting live VM state into [`States`]
//! - [`on_persist`] — observer that snapshots state when triggered
//! - [`restore_for_head`] — restores persisted state for a head

use crate::head;
use crate::reg::Registry;
use bevy_ecs::prelude::*;
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::node::state::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};

// ----------------------------------------------------------------------------
// Resources
// ----------------------------------------------------------------------------

/// Serialized node state snapshots keyed by commit address.
///
/// Mirrors the `Views` pattern: the VM's `ROOT_STATE` is the live source of
/// truth while a head is open, and this resource stores serialized snapshots
/// that survive head close/reopen and app restart.
#[derive(Resource, Default)]
pub struct States(pub HashMap<ca::CommitAddr, Bytes>);

/// Names of graphs that have state persistence enabled.
///
/// When a stateful graph is created, persistence defaults to enabled.
/// Toggling it off removes the name from this set.
#[derive(Resource, Default, Serialize, Deserialize)]
pub struct PersistStateConfig(pub HashSet<String>);

// ----------------------------------------------------------------------------
// Deref impls
// ----------------------------------------------------------------------------

impl Deref for States {
    type Target = HashMap<ca::CommitAddr, Bytes>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for States {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for PersistStateConfig {
    type Target = HashSet<String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for PersistStateConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

// ----------------------------------------------------------------------------
// Events
// ----------------------------------------------------------------------------

/// Trigger to persist live VM state for a head into [`States`].
#[derive(Event)]
pub struct PersistEvent {
    pub head: Entity,
}

// ----------------------------------------------------------------------------
// Observer
// ----------------------------------------------------------------------------

/// Snapshot node state to [`States`] when triggered.
///
/// Only persists state for heads whose branch name is in [`PersistStateConfig`].
pub fn on_persist<N: 'static + Send + Sync>(
    trigger: On<PersistEvent>,
    registry: Res<Registry<N>>,
    heads: Query<&head::HeadRef, With<head::OpenHead>>,
    mut vms: NonSendMut<head::HeadVms>,
    mut states: ResMut<States>,
    config: Res<PersistStateConfig>,
) {
    let entity = trigger.event().head;
    let Ok(head_ref) = heads.get(entity) else {
        return;
    };
    let ca::Head::Branch(name) = &**head_ref else {
        return;
    };
    if !config.contains(name) {
        return;
    }
    let Some(commit_ca) = registry.head_commit_ca(&**head_ref).copied() else {
        log::warn!("State persistence enabled for \"{name}\" but commit not found");
        return;
    };
    let Some(vm) = vms.get_mut(&entity) else {
        log::warn!("State persistence enabled for \"{name}\" but VM not available");
        return;
    };
    match gantz_core::node::state::serialize_root(vm) {
        Ok(bytes) => {
            log::debug!("Saved {} bytes of state for \"{name}\"", bytes.len());
            states.insert(commit_ca, bytes);
        }
        Err(e) => {
            log::warn!("Failed to serialize state for \"{name}\": {e}");
        }
    }
}

// ----------------------------------------------------------------------------
// Functions
// ----------------------------------------------------------------------------

/// Restore persisted node state for a head if configured and available.
pub fn restore_for_head<N: 'static + Send + Sync>(
    head: &ca::Head,
    entity: Entity,
    registry: &Registry<N>,
    states: &States,
    config: &PersistStateConfig,
    vms: &mut head::HeadVms,
) {
    let ca::Head::Branch(name) = head else {
        return;
    };
    if !config.contains(name) {
        return;
    }
    let Some(commit_ca) = registry.head_commit_ca(head) else {
        log::warn!("State persistence enabled for \"{name}\" but commit not found");
        return;
    };
    let Some(bytes) = states.get(commit_ca) else {
        return;
    };
    let Some(vm) = vms.get_mut(&entity) else {
        log::warn!("State persistence enabled for \"{name}\" but VM not available");
        return;
    };
    match gantz_core::node::state::deserialize_and_restore_root(vm, bytes) {
        Ok(()) => log::debug!("Restored {} bytes of state for \"{name}\"", bytes.len()),
        Err(e) => log::warn!("Failed to restore state for \"{name}\": {e}"),
    }
}
