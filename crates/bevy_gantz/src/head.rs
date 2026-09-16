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
/// It requires the compile-outcome components, so every spawn path gets them
/// by default. `vm::sync` fills them in on the next `Update`.
#[derive(Component)]
#[require(Module, Diagnostics, crate::vm::CompiledInputs)]
pub struct OpenHead;

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
/// via [`crate::vm::commit_working_graph`]. Head open, replace, branch move
/// and resync instead replace the working graph with a registry commit's graph
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
/// or a branch move.
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
/// A head's VM owns its graph's runtime node state. Replace and branch move
/// point a head at a different graph. They keep the VM and migrate its node
/// state through the commits' node-identity mapping. See
/// [`crate::vm::migrate_vm_state`]. They also reset
/// [`crate::vm::CompiledInputs`] so `vm::sync` recompiles. In-place edits
/// leave both untouched. The VM is dropped and re-initialized with default
/// state only when no mapping can be derived.
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
    // reset the memo so `vm::sync` recompiles. The `GantzEguiPlugin` observer
    // updates `HeadGuiState` and `GraphViews`.
    let old_ca = old_head.as_ref().and_then(|h| registry.head_commit_ca(h));
    let old_graph = old_head
        .as_ref()
        .and_then(|h| registry.head_commit(h).map(|c| c.graph));
    let new_ca = registry.head_commit_ca(new_head);
    let new_graph = registry.head_commit(new_head).map(|c| c.graph);
    let same_graph = matches!((old_graph, new_graph), (Some(a), Some(b)) if a == b);
    if same_graph {
        cmds.entity(focused_entity)
            .insert((HeadRef(new_head.clone()), WorkingGraph(graph)));
    } else {
        crate::vm::migrate_vm_state(&registry, &mut vms, focused_entity, old_ca, new_ca);
        cmds.entity(focused_entity).insert((
            HeadRef(new_head.clone()),
            WorkingGraph(graph),
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

/// Observer for [`MoveBranchEvent`].
///
/// Updates the registry and the [`WorkingGraph`] and emits [`ChangedEvent`]
/// within one command flush, so no system observes an inconsistent state.
pub fn on_move_branch(
    trigger: On<MoveBranchEvent>,
    mut cmds: Commands,
    mut registry: ResMut<Registry>,
    mut vms: NonSendMut<HeadVms>,
) {
    let event = trigger.event();
    let head = ca::Head::Branch(event.name.clone());
    // The current commit is needed below to detect a same-graph move and to
    // migrate node state.
    let old_ca = registry.head_commit_ca(&head);
    let old_graph = registry.head_commit(&head).map(|c| c.graph);
    let prev = registry.set_head(event.name.clone(), event.target);
    let Some(graph) = head_working_graph(&registry, &head) else {
        log::error!("MoveBranch: graph data missing for target commit");
        match prev {
            Some(ca) => {
                registry.set_head(event.name.clone(), ca);
            }
            None => {
                registry.remove_head(&event.name);
            }
        }
        return;
    };
    // A same-graph target is a layout-only change. Keep the VM, its node state
    // and the compile memo, so `vm::sync` skips recompilation. Otherwise
    // migrate the node state through the commits' node-identity mapping and
    // reset the memo so `vm::sync` recompiles.
    let new_graph = registry.head_commit(&head).map(|c| c.graph);
    let same_graph = matches!((old_graph, new_graph), (Some(a), Some(b)) if a == b);
    if same_graph {
        cmds.entity(event.entity).insert(WorkingGraph(graph));
    } else {
        crate::vm::migrate_vm_state(
            &registry,
            &mut vms,
            event.entity,
            old_ca,
            Some(event.target),
        );
        cmds.entity(event.entity)
            .insert((WorkingGraph(graph), crate::vm::CompiledInputs::default()));
    }
    cmds.trigger(ChangedEvent {
        entity: event.entity,
        old_head: head.clone(),
        new_head: head,
        old_commit: old_ca,
        new_commit: Some(event.target),
        same_graph,
    });
}
