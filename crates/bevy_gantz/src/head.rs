//! Entity-based head management for gantz.
//!
//! This module provides Bevy components and resources that manage open graph
//! heads as entities.

use crate::reg::Registry;
use bevy_ecs::{prelude::*, query::QueryData};
use bevy_log as log;
use gantz_ca as ca;
use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
};
use steel::steel_vm::engine::Engine;

/// QueryData that bundles the open head components.
///
/// Use with `Query<OpenHeadData, With<OpenHead>>`.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct OpenHeadData {
    pub entity: Entity,
    pub head_ref: &'static mut HeadRef,
    pub working_graph: &'static mut WorkingGraph,
    pub module: &'static mut Module,
    pub diagnostics: &'static mut Diagnostics,
    pub compiled_inputs: &'static mut crate::vm::CompiledInputs,
}

// Components

/// Marker component for an open gantz head entity.
///
/// It requires the compile-outcome components and [`PendingLoad`], so every
/// spawn path gets them by default. `vm::sync` fills in the compile outcome
/// on the next `Update`.
#[derive(Component)]
#[require(Module, Diagnostics, crate::vm::CompiledInputs, PendingLoad)]
pub struct OpenHead;

/// Marks an open head whose graph was loaded into its tab but has not yet run
/// its load evaluation.
///
/// Every spawned head gets it through [`OpenHead`], and [`on_replace`] adds it
/// again. A [`MoveHeadEvent`] navigates history and is not a load, so it does
/// not add it. The UI layer's `load!` driver removes it once the loaded graph
/// has compiled, then fires the graph's `load!` nodes.
#[derive(Component, Default)]
pub struct PendingLoad;

/// The [`gantz_ca::Head`], a branch or commit reference.
#[derive(Component, Clone)]
pub struct HeadRef(pub ca::Head);

/// The working copy of the graph associated with this head, in its stored
/// [`gantz_ca::DataGraph`] form.
///
/// Typed nodes appear only transiently. The GUI reifies one node at a time
/// through the app's codec. Compilation reads the reified cache at the
/// committed address.
///
/// # Invariant: commit before returning
///
/// Any system that mutates a head's `WorkingGraph` must commit it before the
/// system returns. In-place edits such as the GUI pass and graph ops commit
/// via [`crate::vm::commit_working_graph`]. Head open, replace, move and
/// resync instead replace the working graph with a registry commit's graph
/// and reset [`crate::vm::CompiledInputs`]. As a result, between systems the
/// working graph's content address always equals the head's committed graph
/// CA, `registry.head_commit(head).graph`.
///
/// The UI layer's VM synchronisation system `bevy_gantz_egui::vm::sync`
/// relies on this. It reads the committed CA to decide when to recompile and
/// never re-hashes the working graph. [`crate::vm::validate_committed`] is a
/// default-off debug check that flags any violation of this invariant.
#[derive(Component)]
pub struct WorkingGraph(pub ca::DataGraph);

/// The latest compile outcome for this head.
///
/// `compiled` is kept even when steel rejected the generated module, so its
/// source stays displayable and error spans stay resolvable. It is `None` only
/// when module generation itself failed. Both fields are present when a
/// generated module failed evaluation.
#[derive(Component, Default)]
pub struct Module {
    /// The module artifact, made of source text and a source map.
    pub compiled: Option<gantz_core::vm::Compiled>,
    /// The rendered error chain from a failed compile.
    pub error: Option<String>,
}

/// Diagnostics from the head's latest compile and entrypoint evaluations.
///
/// Every compile replaces the compile diagnostics. Every evaluation replaces
/// the runtime diagnostics and clears them on success.
#[derive(Component, Default)]
pub struct Diagnostics(pub Vec<gantz_core::Diagnostic>);

// Events

/// Event to open a head as a new tab, or focus it if already open.
#[derive(Event)]
pub struct OpenEvent(pub ca::Head);

/// Event to close a head tab.
#[derive(Event)]
pub struct CloseEvent(pub ca::Head);

/// Event to load a different head into the focused tab, for example from the
/// graph select or when entering a nested graph.
///
/// To navigate a head through its own history, use [`MoveHeadEvent`].
#[derive(Event)]
pub struct ReplaceEvent(pub ca::Head);

/// Event to create a new branch from an existing head.
#[derive(Event)]
pub struct BranchHeadEvent {
    pub original: ca::Head,
    pub new_name: String,
}

/// Event to move an open head to a different commit in place, as undo, redo
/// and merges do.
///
/// A branch head moves its branch to the target. A detached commit head
/// points at the target commit instead.
#[derive(Event)]
pub struct MoveHeadEvent {
    pub entity: Entity,
    pub target: ca::CommitAddr,
}

// Hook events, emitted after core operations for app-specific handling.

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

/// Emitted when a head's backing data has changed, such as on a replacement
/// or a move.
#[derive(Event)]
pub struct ChangedEvent {
    pub entity: Entity,
    pub old_head: ca::Head,
    pub new_head: ca::Head,
    /// The commit the head pointed at before the change, if it resolved.
    pub old_commit: Option<ca::CommitAddr>,
    /// The commit the head points at after the change, if it resolves.
    pub new_commit: Option<ca::CommitAddr>,
    /// Whether the new commit shares the previous commit's graph content
    /// address, so only layout or metadata changed. The VM and its node state
    /// survive such a change.
    pub same_graph: bool,
}

/// Emitted after a branch has been created from a head.
#[derive(Event)]
pub struct BranchedHeadEvent {
    pub entity: Entity,
    pub old_head: ca::Head,
    pub new_head: ca::Head,
}

/// Emitted when `vm::sync` detects a graph change and commits a head's working
/// graph to the registry. Apps can observe this to update UI state.
#[derive(Event)]
pub struct CommittedEvent {
    pub entity: Entity,
    pub old_head: ca::Head,
    pub new_head: ca::Head,
}

// Resources

/// Per-head VMs stored in a NonSend resource, keyed by entity because
/// `Engine` is not `Send`.
///
/// A head's VM owns its graph's runtime node state. When replace or move
/// point a head at a different graph, they keep the VM and migrate its
/// node state through the commits' node-identity mapping. See
/// [`crate::vm::migrate_vm_state`]. They also reset
/// [`crate::vm::CompiledInputs`] so `vm::sync` recompiles. A same-graph move
/// keeps the VM, its state and the memo. The VM is dropped and re-initialized
/// with default state only when no mapping can be derived.
#[derive(Default)]
pub struct HeadVms(pub HashMap<Entity, Engine>);

/// The currently focused head entity.
#[derive(Resource, Default)]
pub struct FocusedHead(pub Option<Entity>);

/// The open head entities in tab display order.
#[derive(Resource, Default)]
pub struct HeadTabOrder(pub Vec<Entity>);

/// The id of the shared graph-description section. The GUI layer's typed
/// declaration `gantz_egui::section::Descriptions` mirrors it.
pub const DESCRIPTIONS_ID: &str = "gantz.description";

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

impl Deref for WorkingGraph {
    type Target = ca::DataGraph;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for WorkingGraph {
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

/// The entity for the given head, if it is open.
pub fn find_entity(
    head: &ca::Head,
    heads: &Query<(Entity, &HeadRef), With<OpenHead>>,
) -> Option<Entity> {
    heads
        .iter()
        .find(|(_, head_ref)| &***head_ref == head)
        .map(|(entity, _)| entity)
}

/// The stored description for the named graph, if any.
///
/// Reads the [`DESCRIPTIONS_ID`] section directly, so this crate stays
/// independent of the GUI layer that declares the typed accessor.
pub fn description(reg: &ca::Registry, name: &ca::Name) -> Option<String> {
    let key = ca::Key::Name(name.clone());
    reg.section_entry(DESCRIPTIONS_ID, &key)
        .and_then(ca::section::value_from_datum)
}

/// Store a description for the named graph in the [`DESCRIPTIONS_ID`]
/// section. An empty string removes the entry.
pub fn set_description(reg: &mut ca::Registry, name: ca::Name, description: String) {
    let key = ca::Key::Name(name);
    if description.is_empty() {
        reg.remove_section_entry(DESCRIPTIONS_ID, &key);
    } else {
        ca::section_insert_datum(
            reg,
            DESCRIPTIONS_ID,
            ca::MergePolicy::KeepExisting,
            ca::Liveness::WithName,
            key,
            &description,
        )
        .expect("a `String` always encodes as a datum");
    }
}

/// The head's committed graph cloned from the registry as a working copy.
/// No reify is involved, so opens never fail on node content. The result is
/// `None` only when the head's commit or graph data is missing.
fn head_working_graph(registry: &Registry, head: &ca::Head) -> Option<ca::DataGraph> {
    let addr = registry.head_commit(head)?.graph;
    registry.graph(&addr).cloned()
}

/// Whether the given head is the focused head.
pub fn is_focused(
    head: &ca::Head,
    heads: &Query<(Entity, &HeadRef), With<OpenHead>>,
    focused: &FocusedHead,
) -> bool {
    find_entity(head, heads)
        .map(|entity| **focused == Some(entity))
        .unwrap_or(false)
}

// Observers

/// Observer for [`OpenEvent`].
pub fn on_open(
    trigger: On<OpenEvent>,
    mut cmds: Commands,
    registry: Res<Registry>,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
    heads: Query<(Entity, &HeadRef), With<OpenHead>>,
) {
    let OpenEvent(new_head) = trigger.event();

    if let Some(entity) = find_entity(new_head, &heads) {
        **focused = Some(entity);
        return;
    }

    let Some(graph) = head_working_graph(&registry, new_head) else {
        log::error!("cannot open head: graph data missing from the registry");
        return;
    };

    // `OpenHead`'s required components cover the compile outcome. `vm::sync`
    // initializes the VM on the next `Update`. The `GantzEguiPlugin` observer
    // adds `HeadGuiState` and `GraphViews`.
    let entity = cmds
        .spawn((OpenHead, HeadRef(new_head.clone()), WorkingGraph(graph)))
        .id();

    tab_order.push(entity);
    **focused = Some(entity);

    cmds.trigger(OpenedEvent {
        entity,
        head: new_head.clone(),
    });
}

/// Observer for [`ReplaceEvent`].
pub fn on_replace(
    trigger: On<ReplaceEvent>,
    mut cmds: Commands,
    registry: Res<Registry>,
    mut focused: ResMut<FocusedHead>,
    mut vms: NonSendMut<HeadVms>,
    heads: Query<(Entity, &HeadRef), With<OpenHead>>,
) {
    let ReplaceEvent(new_head) = trigger.event();

    if let Some(entity) = find_entity(new_head, &heads) {
        **focused = Some(entity);
        return;
    }

    let Some(focused_entity) = **focused else {
        return;
    };
    let old_head = heads.get(focused_entity).ok().map(|(_, h)| (**h).clone());

    let Some(graph) = head_working_graph(&registry, new_head) else {
        log::error!("cannot replace head: graph data missing from the registry");
        return;
    };

    // A same-graph commit is a layout-only change. Keep the VM, its node state
    // and the compile memo, so `vm::sync` skips recompilation. Otherwise
    // migrate the node state through the commits' node-identity mapping and
    // reset the memo so `vm::sync` recompiles. Either way the tab has loaded
    // a head, so its load is pending. The `GantzEguiPlugin` observer updates
    // `HeadGuiState` and `GraphViews`.
    let old_ca = old_head.as_ref().and_then(|h| registry.head_commit_ca(h));
    let old_graph = old_head
        .as_ref()
        .and_then(|h| registry.head_commit(h).map(|c| c.graph));
    let new_ca = registry.head_commit_ca(new_head);
    let new_graph = registry.head_commit(new_head).map(|c| c.graph);
    let same_graph = matches!((old_graph, new_graph), (Some(a), Some(b)) if a == b);
    if same_graph {
        cmds.entity(focused_entity).insert((
            HeadRef(new_head.clone()),
            WorkingGraph(graph),
            PendingLoad,
        ));
    } else {
        crate::vm::migrate_vm_state(&registry, &mut vms, focused_entity, old_ca, new_ca);
        cmds.entity(focused_entity).insert((
            HeadRef(new_head.clone()),
            WorkingGraph(graph),
            PendingLoad,
            crate::vm::CompiledInputs::default(),
        ));
    }

    if let Some(old) = old_head {
        cmds.trigger(ChangedEvent {
            entity: focused_entity,
            old_head: old,
            new_head: new_head.clone(),
            old_commit: old_ca,
            new_commit: new_ca,
            same_graph,
        });
    }
}

/// Observer for [`CloseEvent`]. The last open head never closes.
pub fn on_close(
    trigger: On<CloseEvent>,
    mut cmds: Commands,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
    mut vms: NonSendMut<HeadVms>,
    heads: Query<(Entity, &HeadRef), With<OpenHead>>,
) {
    let CloseEvent(head) = trigger.event();

    if tab_order.len() <= 1 {
        return;
    }

    let Some(entity) = find_entity(head, &heads) else {
        return;
    };
    let Some(ix) = tab_order.iter().position(|&x| x == entity) else {
        return;
    };

    vms.remove(&entity);

    cmds.entity(entity).despawn();
    tab_order.retain(|&x| x != entity);

    if **focused == Some(entity) {
        let new_ix = ix.saturating_sub(1).min(tab_order.len().saturating_sub(1));
        **focused = tab_order.get(new_ix).copied();
    }

    cmds.trigger(ClosedEvent {
        entity,
        head: head.clone(),
    });
}

/// Observer for [`BranchHeadEvent`].
pub fn on_branch_head(
    trigger: On<BranchHeadEvent>,
    mut cmds: Commands,
    mut registry: ResMut<Registry>,
    mut heads: Query<(Entity, &mut HeadRef), With<OpenHead>>,
) {
    let BranchHeadEvent { original, new_name } = trigger.event();
    let new_name: ca::Name = new_name.parse().expect("infallible");

    let Some(commit_ca) = registry.head_commit_ca(original) else {
        log::error!("Failed to get commit address for head: {:?}", original);
        return;
    };

    // A new commit that points at the same graph gives the new branch its own
    // `CommitAddr`, and therefore its own views and layout.
    let graph_addr = registry.commits()[&commit_ca].graph;
    let new_commit_ca =
        registry.commit_graph(crate::reg::timestamp(), Some(commit_ca), graph_addr, || {
            unreachable!("graph already exists in registry")
        });

    registry.set_head(new_name.clone(), new_commit_ca);

    if let ca::Head::Branch(orig_name) = original {
        if let Some(desc) = description(&registry, orig_name) {
            set_description(&mut registry, new_name.clone(), desc);
        }
    }

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

/// Observer for [`MoveHeadEvent`].
///
/// Updates the registry and the [`WorkingGraph`] and emits [`ChangedEvent`]
/// within one command flush, so no system observes an inconsistent state.
///
/// Each head has at most one tab. When a commit head's target commit is
/// already open in another tab, that tab takes focus and this head does not
/// move.
pub fn on_move_head(
    trigger: On<MoveHeadEvent>,
    mut cmds: Commands,
    mut registry: ResMut<Registry>,
    mut focused: ResMut<FocusedHead>,
    mut vms: NonSendMut<HeadVms>,
    heads: Query<(Entity, &HeadRef), With<OpenHead>>,
) {
    let &MoveHeadEvent { entity, target } = trigger.event();
    let Ok((_, head_ref)) = heads.get(entity) else {
        log::error!("MoveHead: head not found for entity {entity:?}");
        return;
    };
    let old_head = head_ref.0.clone();
    let Some(graph) = registry.commit_graph_ref(&target).cloned() else {
        log::error!("MoveHead: graph data missing for target commit");
        return;
    };
    // The current commit is needed below to detect a same-graph move and to
    // migrate node state.
    let old_ca = registry.head_commit_ca(&old_head);
    let old_graph = registry.head_commit(&old_head).map(|c| c.graph);
    let new_head = match &old_head {
        ca::Head::Branch(name) => {
            registry.set_head(name.clone(), target);
            old_head.clone()
        }
        ca::Head::Commit(_) => {
            let new_head = ca::Head::Commit(target);
            if let Some(open) = find_entity(&new_head, &heads) {
                **focused = Some(open);
                return;
            }
            new_head
        }
    };
    // A same-graph target is a layout-only change. Keep the VM, its node state
    // and the compile memo, so `vm::sync` skips recompilation. Otherwise
    // migrate the node state through the commits' node-identity mapping and
    // reset the memo so `vm::sync` recompiles.
    let same_graph = old_graph == Some(registry.commits()[&target].graph);
    if same_graph {
        cmds.entity(entity)
            .insert((HeadRef(new_head.clone()), WorkingGraph(graph)));
    } else {
        crate::vm::migrate_vm_state(&registry, &mut vms, entity, old_ca, Some(target));
        cmds.entity(entity).insert((
            HeadRef(new_head.clone()),
            WorkingGraph(graph),
            crate::vm::CompiledInputs::default(),
        ));
    }
    cmds.trigger(ChangedEvent {
        entity,
        old_head,
        new_head,
        old_commit: old_ca,
        new_commit: Some(target),
        same_graph,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An app with the head observers over the registry from
    /// `vm::tests::base_and_child`.
    fn app() -> (bevy_app::App, ca::CommitAddr, ca::CommitAddr) {
        let (reg, base, child) = crate::vm::tests::base_and_child();
        let mut app = bevy_app::App::new();
        app.add_plugins(crate::GantzPlugin)
            .insert_resource(Registry(reg));
        (app, base, child)
    }

    /// Trigger the event and apply the commands its observers queue.
    fn trigger<E: for<'a> Event<Trigger<'a>: Default>>(app: &mut bevy_app::App, event: E) {
        app.world_mut().trigger(event);
        app.world_mut().flush();
    }

    /// The entity of the open head.
    fn entity(app: &mut bevy_app::App, head: &ca::Head) -> Entity {
        let mut q = app
            .world_mut()
            .query_filtered::<(Entity, &HeadRef), With<OpenHead>>();
        q.iter(app.world())
            .find(|(_, h)| &h.0 == head)
            .map(|(e, _)| e)
            .expect("head is open")
    }

    /// The head and working graph address of an open head entity.
    fn head_state(app: &bevy_app::App, entity: Entity) -> (ca::Head, ca::GraphAddr) {
        let world = app.world();
        let head = world.get::<HeadRef>(entity).unwrap().0.clone();
        let graph = ca::graph_addr(&world.get::<WorkingGraph>(entity).unwrap().0);
        (head, graph)
    }

    #[test]
    fn move_head_moves_a_commit_head_in_place() {
        let (mut app, base, child) = app();
        let head = ca::Head::Commit(base);
        trigger(&mut app, OpenEvent(head.clone()));
        let e = entity(&mut app, &head);
        trigger(
            &mut app,
            MoveHeadEvent {
                entity: e,
                target: child,
            },
        );
        let child_graph = app.world().resource::<Registry>().commits()[&child].graph;
        assert_eq!(head_state(&app, e), (ca::Head::Commit(child), child_graph));
    }

    #[test]
    fn move_head_moves_a_branch_in_place() {
        let (mut app, base, child) = app();
        let name: ca::Branch = "g".parse().expect("infallible");
        app.world_mut()
            .resource_mut::<Registry>()
            .set_head(name.clone(), base);
        let head = ca::Head::Branch(name);
        trigger(&mut app, OpenEvent(head.clone()));
        let e = entity(&mut app, &head);
        trigger(
            &mut app,
            MoveHeadEvent {
                entity: e,
                target: child,
            },
        );
        let reg = app.world().resource::<Registry>();
        assert_eq!(reg.head_commit_ca(&head), Some(child));
        let child_graph = reg.commits()[&child].graph;
        assert_eq!(head_state(&app, e), (head, child_graph));
    }

    /// Opening and replacing load a head into its tab and mark its load
    /// pending. A move navigates history and does not.
    #[test]
    fn open_and_replace_mark_a_pending_load_and_move_does_not() {
        let (mut app, base, child) = app();
        let head = ca::Head::Commit(base);
        trigger(&mut app, OpenEvent(head.clone()));
        let e = entity(&mut app, &head);
        assert!(app.world().get::<PendingLoad>(e).is_some());
        app.world_mut().entity_mut(e).remove::<PendingLoad>();
        trigger(
            &mut app,
            MoveHeadEvent {
                entity: e,
                target: child,
            },
        );
        assert!(app.world().get::<PendingLoad>(e).is_none());
        trigger(&mut app, ReplaceEvent(head));
        assert!(app.world().get::<PendingLoad>(e).is_some());
    }

    /// A commit head never moves onto a commit that another tab has open.
    /// That tab takes focus instead.
    #[test]
    fn move_head_focuses_an_open_target_commit() {
        let (mut app, base, child) = app();
        trigger(&mut app, OpenEvent(ca::Head::Commit(child)));
        trigger(&mut app, OpenEvent(ca::Head::Commit(base)));
        let child_e = entity(&mut app, &ca::Head::Commit(child));
        let base_e = entity(&mut app, &ca::Head::Commit(base));
        trigger(
            &mut app,
            MoveHeadEvent {
                entity: base_e,
                target: child,
            },
        );
        assert_eq!(**app.world().resource::<FocusedHead>(), Some(child_e));
        let base_graph = app.world().resource::<Registry>().commits()[&base].graph;
        assert_eq!(
            head_state(&app, base_e),
            (ca::Head::Commit(base), base_graph)
        );
    }
}
