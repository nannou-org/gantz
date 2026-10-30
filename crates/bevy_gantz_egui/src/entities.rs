//! The node entities of each open head, mirrored from its typed graph. See
//! [`bevy_gantz::node`].
//!
//! [`mirror`] runs in [`bevy_gantz::NodeSet`]. The head's VM holds the entity
//! of each node, and every migration of node state migrates it too. So a node
//! keeps its entity for as long as it keeps its state slot, across edits,
//! nesting, merges and history navigation. A node whose type changes gets a
//! new entity.
//!
//! A node type registered with [`NodeKindAppExt::add_node_kind`] is also a
//! component on its node entities. A change of the node's fields replaces the
//! component, so `Changed<T>` sees the edit.

use crate::reg::{BuiltinNodes, GraphCache, lookup_node};
use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head::{HeadRef, HeadVms, OpenHead};
use bevy_gantz::node::{self as node_entity, HeadNodes, NodeOf, NodePath, PushEntry};
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::{compile::entrypoint, node, visit};
use gantz_egui::node::DynNode;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use steel::steel_vm::engine::Engine;

/// Adds node types as components of their node entities.
pub trait NodeKindAppExt {
    /// Insert the node type `T` on the entity of each node of type `T`.
    fn add_node_kind<T: Component + Clone + PartialEq>(&mut self) -> &mut Self;
}

/// The node types that their node entities carry as components. See
/// [`NodeKindAppExt::add_node_kind`].
#[derive(Default, Resource)]
pub struct NodeKinds(HashMap<TypeId, InsertKind>);

/// The graph whose tree an open head's node entities mirror.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Mirrored(ca::GraphAddr);

/// The concrete type of a node entity's node.
#[derive(Component, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NodeType(TypeId);

/// Inserts a node as its type's component on its entity.
type InsertKind = fn(&dyn Any, EntityCommands);

/// The visitor behind [`mirror`].
struct Mirror<'a, 'w, 's> {
    head: Entity,
    kinds: &'a NodeKinds,
    get_node: node::GetNode<'a>,
    vm: &'a mut Engine,
    /// The head's node entities that the visit has not reached yet.
    unseen: HashMap<Entity, Seen>,
    cmds: &'a mut Commands<'w, 's>,
}

/// A node entity as the previous mirror left it.
struct Seen {
    path: Vec<node::Id>,
    ty: TypeId,
    push: Option<PushEntry>,
}

impl NodeKindAppExt for App {
    fn add_node_kind<T: Component + Clone + PartialEq>(&mut self) -> &mut Self {
        self.world_mut()
            .get_resource_or_init::<NodeKinds>()
            .0
            .insert(TypeId::of::<T>(), insert_kind::<T>);
        self
    }
}

impl visit::TypedVisitor<DynNode> for Mirror<'_, '_, '_> {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &DynNode) {
        let n: &dyn gantz_core::Node = &**node;
        let any = n as &dyn Any;
        let ty = any.type_id();
        let path = ctx.path();
        let push = entrypoint::node_entrypoints(self.get_node, path, n, entrypoint::EvalKind::Push)
            .into_iter()
            .next()
            .map(PushEntry);
        let known = node_entity::entity(self.vm, path).unwrap_or_else(|e| {
            log::error!("node entity read failed: {e}");
            None
        });
        // An entity of another type stays unseen, so the mirror despawns it.
        let kept = known
            .filter(|e| self.unseen.get(e).is_some_and(|seen| seen.ty == ty))
            .and_then(|e| self.unseen.remove(&e).map(|seen| (e, seen)));
        let entity = match kept {
            Some((entity, seen)) => {
                let mut cmds = self.cmds.entity(entity);
                if seen.path != path {
                    cmds.insert(NodePath(path.to_vec()));
                }
                match (seen.push != push, push) {
                    (false, _) => (),
                    (true, Some(push)) => {
                        cmds.insert(push);
                    }
                    (true, None) => {
                        cmds.remove::<PushEntry>();
                    }
                }
                entity
            }
            None => {
                let mut cmds =
                    self.cmds
                        .spawn((NodeOf(self.head), NodePath(path.to_vec()), NodeType(ty)));
                if let Some(push) = push {
                    cmds.insert(push);
                }
                let entity = cmds.id();
                if let Err(e) = node_entity::set_entity(self.vm, path, entity) {
                    log::error!("node entity write failed: {e}");
                }
                entity
            }
        };
        if let Some(insert) = self.kinds.0.get(&ty) {
            insert(any, self.cmds.entity(entity));
        }
    }
}

/// Keep the node entities of each open head in step with its committed graph.
///
/// A head is mirrored again only when its committed graph changes. A head
/// without a VM, or whose graph is not in the cache yet, keeps its previous
/// entities until it has both.
pub fn mirror(
    registry: Res<Registry>,
    cache: Res<GraphCache>,
    builtins: Res<BuiltinNodes>,
    kinds: Res<NodeKinds>,
    mut vms: NonSendMut<HeadVms>,
    heads: Query<(Entity, &HeadRef, Option<&Mirrored>, Option<&HeadNodes>), With<OpenHead>>,
    nodes: Query<(&NodePath, &NodeType, Option<&PushEntry>)>,
    mut cmds: Commands,
) {
    for (head, head_ref, mirrored, head_nodes) in heads.iter() {
        let Some(graph_ca) = registry.head_commit(&head_ref.0).map(|c| c.graph) else {
            continue;
        };
        if mirrored == Some(&Mirrored(graph_ca)) {
            continue;
        }
        let (Some(graph), Some(vm)) = (cache.get(&graph_ca), vms.get_mut(&head)) else {
            continue;
        };
        let unseen = head_nodes
            .into_iter()
            .flat_map(|head_nodes| head_nodes.iter())
            .filter_map(|entity| {
                let (path, ty, push) = nodes.get(entity).ok()?;
                let seen = Seen {
                    path: path.0.clone(),
                    ty: ty.0,
                    push: push.cloned(),
                };
                Some((entity, seen))
            })
            .collect();
        let get_node = |ca: &ca::ContentAddr| lookup_node(&cache, &builtins.instances, ca);
        let mut visitor = Mirror {
            head,
            kinds: &kinds,
            get_node: &get_node,
            vm,
            unseen,
            cmds: &mut cmds,
        };
        gantz_core::graph::visit_typed(&get_node, graph, &[], &mut visitor);
        for entity in visitor.unseen.into_keys() {
            cmds.entity(entity).despawn();
        }
        cmds.entity(head).insert(Mirrored(graph_ca));
    }
}

/// Insert the node as `T` on its entity, unless the entity holds an equal
/// `T` already.
fn insert_kind<T: Component + Clone + PartialEq>(node: &dyn Any, mut entity: EntityCommands) {
    let Some(value) = node.downcast_ref::<T>().cloned() else {
        return;
    };
    entity.queue(move |mut entity: EntityWorldMut| {
        if entity.get::<T>() != Some(&value) {
            entity.insert(value);
        }
    });
}
