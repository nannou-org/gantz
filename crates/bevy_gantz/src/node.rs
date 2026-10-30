//! The nodes of open heads as entities.
//!
//! Each node in an open head's graph tree is an entity with [`NodeOf`] and
//! [`NodePath`]. A node keeps its entity across edits and history navigation
//! for as long as it keeps its VM state. The UI layer mirrors the entities
//! from each head's typed graph in [`crate::NodeSet`].
//!
//! Systems drive nodes through their entities. [`PushEval`] evaluates a
//! node's push entrypoint. [`NodeState`] reads and writes a node's VM state.
//! Despawning a node entity drops the components on it, so a node's tasks and
//! other runtime objects live on its entity.
//!
//! The head's VM holds the entity of each node as its identity, see
//! [`gantz_core::node::state::ROOT_IDS`]. Every migration of node state also
//! migrates the identities, so the mirror reads each node's entity from the
//! VM. See [`entity`] and [`set_entity`].

use crate::head::HeadVms;
use crate::vm::EvalEntryEvent;
use bevy_ecs::{prelude::*, system::SystemParam};
use bevy_log as log;
use gantz_core::{compile, node};
use std::fmt;
use steel::rvals::{Custom, IntoSteelVal};
use steel::steel_vm::engine::Engine;
use steel::{SteelErr, SteelVal, rvals::FromSteelVal};

/// The open head whose graph tree holds this node. Closing the head despawns
/// its nodes.
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
#[relationship(relationship_target = HeadNodes)]
pub struct NodeOf(pub Entity);

/// The node entities of an open head.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = NodeOf, linked_spawn)]
pub struct HeadNodes(Vec<Entity>);

/// The path of a node from its head's root graph. `[5, 3]` is node 3 inside
/// node 5.
#[derive(Component, Clone, Debug, Eq, Hash, PartialEq)]
pub struct NodePath(pub Vec<node::Id>);

/// The push entrypoint of a node whose type declares one with
/// [`gantz_core::Node::push_eval`]. [`PushEval`] evaluates it.
#[derive(Component, Clone, Debug, Eq, PartialEq)]
pub struct PushEntry(pub compile::Entrypoint);

/// Event that evaluates a node's [`PushEntry`].
#[derive(EntityEvent)]
pub struct PushEval {
    /// The node entity.
    pub entity: Entity,
    /// The time of the evaluation. See [`EvalEntryEvent::time`].
    pub time: Option<f64>,
}

/// Reads and writes the VM state of node entities.
///
/// The head VMs are not `Send`, so a system with this parameter runs on the
/// main thread.
#[derive(SystemParam)]
pub struct NodeState<'w, 's> {
    vms: NonSendMut<'w, HeadVms>,
    nodes: Query<'w, 's, (&'static NodeOf, &'static NodePath)>,
}

/// The entity of a node as its identity in the VM.
#[derive(Clone, Copy, Debug)]
struct NodeEntity(Entity);

/// The reason that [`NodeState`] did not read or write a node's state.
#[derive(Debug)]
pub enum StateError {
    /// The entity is not a node.
    NotANode(Entity),
    /// The node's head has no VM, for example before its first compile.
    NoVm(Entity),
    /// The VM rejected the access.
    Steel(SteelErr),
}

impl PushEval {
    /// A push evaluation of the node now.
    pub fn now(entity: Entity) -> Self {
        Self { entity, time: None }
    }
}

impl NodeState<'_, '_> {
    /// The node's state value. `None` when the node holds no state.
    pub fn get(&self, node: Entity) -> Result<Option<SteelVal>, StateError> {
        let (head, path) = locate(&self.nodes, node)?;
        let vm = self.vms.get(&head).ok_or(StateError::NoVm(head))?;
        node::state::extract_value(vm, path).map_err(StateError::Steel)
    }

    /// Replace the node's state value.
    pub fn set(&mut self, node: Entity, value: SteelVal) -> Result<(), StateError> {
        let (head, path) = locate(&self.nodes, node)?;
        let vm = self.vms.get_mut(&head).ok_or(StateError::NoVm(head))?;
        node::state::update_value(vm, path, value).map_err(StateError::Steel)
    }
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StateError::NotANode(e) => write!(f, "entity {e} is not a node"),
            StateError::NoVm(head) => write!(f, "head {head} has no VM"),
            StateError::Steel(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StateError {}

impl Custom for NodeEntity {}

/// The entity of the node at `path`, as the VM's identity of the node.
/// `None` when the node has no identity or another identity than an entity.
pub fn entity(vm: &Engine, path: &[node::Id]) -> Result<Option<Entity>, SteelErr> {
    let id = node::state::id(vm, path)?;
    Ok(id.and_then(|id| NodeEntity::from_steelval(&id).ok().map(|NodeEntity(e)| e)))
}

/// Make `entity` the VM's identity of the node at `path`.
pub fn set_entity(vm: &mut Engine, path: &[node::Id], entity: Entity) -> Result<(), SteelErr> {
    let id = NodeEntity(entity).into_steelval()?;
    node::state::set_id(vm, path, id)
}

/// Observer for [`PushEval`]. Triggers an [`EvalEntryEvent`] for the node's
/// push entrypoint.
///
/// The head's module may not compile the entrypoint yet, for example when a
/// task resolves in the same update as a graph edit. The evaluation is then
/// skipped.
pub fn on_push_eval(
    on: On<PushEval>,
    vms: NonSend<HeadVms>,
    nodes: Query<(&NodeOf, &PushEntry)>,
    mut cmds: Commands,
) {
    let Ok((&NodeOf(head), PushEntry(entrypoint))) = nodes.get(on.entity) else {
        log::debug!("push eval skipped: {} has no push entrypoint", on.entity);
        return;
    };
    let fn_name = compile::entry_fn_name(&entrypoint.id());
    let compiled = vms
        .get(&head)
        .is_some_and(|vm| vm.extract_value(&fn_name).is_ok());
    if !compiled {
        log::debug!("push eval skipped: entry fn {fn_name} not compiled");
        return;
    }
    cmds.trigger(EvalEntryEvent {
        head,
        entrypoint: entrypoint.clone(),
        time: on.time,
    });
}

/// The head and path of a node entity.
fn locate<'a>(
    nodes: &'a Query<(&NodeOf, &NodePath)>,
    node: Entity,
) -> Result<(Entity, &'a [node::Id]), StateError> {
    let (head, path) = nodes.get(node).map_err(|_| StateError::NotANode(node))?;
    Ok((head.0, &path.0))
}
