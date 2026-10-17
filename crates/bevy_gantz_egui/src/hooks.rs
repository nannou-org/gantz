//! Observers that keep the GUI state in step with head events.

use crate::{
    GraphCache, GraphView, GuiState, HeadNodeInstances, NodeCodecRes, PerfVm, ResyncRefsEvent,
    refresh_cache,
};
use bevy_ecs::prelude::*;
use bevy_egui::EguiContexts;
use bevy_gantz::EvalEntryComplete;
use bevy_gantz::Registry;
use bevy_gantz::head;
use gantz_ca as ca;

/// Record VM execution timing from `EvalEntryComplete` events into the
/// evaluated head's [`PerfVm`] for the performance widget.
pub(crate) fn on_eval_entry_complete(
    trigger: On<EvalEntryComplete>,
    mut perf_vms: Query<&mut PerfVm>,
) {
    let event = trigger.event();
    if let Ok(mut perf_vm) = perf_vms.get_mut(event.entity) {
        perf_vm.0.record(event.duration);
    }
}

/// Initialize GUI state entry and components for an opened head.
///
/// Loads the view from the registry's view section and inserts the per-head
/// GUI components.
pub fn on_head_opened(
    trigger: On<head::OpenedEvent>,
    registry: Res<Registry>,
    mut gui_state: ResMut<GuiState>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    gui_state.open_heads.entry(event.head.clone()).or_default();

    let head_view = registry
        .head_commit_ca(&event.head)
        .and_then(|ca| gantz_egui::section::view(&registry, &ca))
        .unwrap_or_default();

    cmds.entity(event.entity)
        .insert(GraphView(head_view))
        .insert(HeadNodeInstances::default());
}

/// Migrate GUI state for a changed head and reset its components.
///
/// Loads the view for the new head and resets the per-head GUI components.
pub fn on_head_changed(
    trigger: On<head::ChangedEvent>,
    mut registry: ResMut<Registry>,
    mut gui_state: ResMut<GuiState>,
    mut ctxs: EguiContexts,
    graph_views: Query<&GraphView>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    gui_state.migrate_head(&event.old_head, &event.new_head, false);
    if let Ok(ctx) = ctxs.ctx_mut() {
        gantz_egui::widget::update_graph_pane_head(ctx, &event.old_head, &event.new_head);
    }

    // Load the view for the new head's commit. The target commit may have no
    // stored view. For example, a wire-fetched merge tip whose view blob went
    // missing, or a commit minted by a headless peer. Then carry the live
    // layout forward through the navigation node-identity matching. An empty
    // layout would make the scene auto-layout destructively. Seed the store
    // so the commit has a view before any same-frame session announce.
    let stored = event
        .new_commit
        .and_then(|ca| gantz_egui::section::view(&registry, &ca));
    let mut head_view = match stored {
        Some(view) => view,
        None => {
            let live = graph_views.get(event.entity).ok();
            let carried = match (live, event.old_commit, event.new_commit) {
                (Some(live), _, _) if event.same_graph => live.0.clone(),
                (Some(live), Some(old), Some(new)) => {
                    let node_count = registry
                        .commit_graph_ref(&new)
                        .map(|g| g.node_count())
                        .unwrap_or(0);
                    bevy_gantz::vm::navigation_matching(&registry, old, new)
                        .map(|m| gantz_egui::ops::carry_layout(&live.0, &m, node_count))
                        .unwrap_or_default()
                }
                _ => Default::default(),
            };
            if let Some(new) = event.new_commit {
                if !carried.layout.is_empty() {
                    gantz_egui::section::set_view(&mut registry.0, new, &carried);
                }
            }
            carried
        }
    };

    // Camera is excluded from undo. On a same-graph navigation, such as a
    // layout undo, keep the live camera rather than the target commit's
    // stored camera.
    if event.same_graph {
        if let Ok(current) = graph_views.get(event.entity) {
            head_view.camera = current.camera;
        }
    }

    cmds.entity(event.entity)
        .insert(GraphView(head_view))
        .insert(HeadNodeInstances::default());
}

/// Remove GUI state for closed head.
pub fn on_head_closed(trigger: On<head::ClosedEvent>, mut gui_state: ResMut<GuiState>) {
    let head = &trigger.event().head;
    gui_state.open_heads.remove(head);
    gui_state.redo_stacks.remove(head);
    gui_state.undo_cursors.remove(head);
}

/// Migrate GUI state for branch creation.
pub fn on_branch_created(
    trigger: On<head::BranchedHeadEvent>,
    mut gui_state: ResMut<GuiState>,
    mut ctxs: EguiContexts,
) {
    let event = trigger.event();
    gui_state.migrate_head(&event.old_head, &event.new_head, false);
    if let Ok(ctx) = ctxs.ctx_mut() {
        gantz_egui::widget::update_graph_pane_head(ctx, &event.old_head, &event.new_head);
    }
}

/// Handle a graph commit by updating egui state.
///
/// Also clears the redo stack, since a new edit invalidates the redo history.
pub fn on_head_committed(
    trigger: On<head::CommittedEvent>,
    mut gui_state: ResMut<GuiState>,
    mut ctxs: EguiContexts,
) {
    let event = trigger.event();
    gui_state.migrate_head(&event.old_head, &event.new_head, true);
    if let Ok(ctx) = ctxs.ctx_mut() {
        gantz_egui::widget::update_graph_pane_head(ctx, &event.old_head, &event.new_head);
    }
}

/// On any head commit, propagate the change to referrers with a
/// [`ResyncRefsEvent`]. For example, a nested graph edit propagates up to its
/// open parent.
pub fn on_head_committed_resync(_trigger: On<head::CommittedEvent>, mut cmds: Commands) {
    cmds.trigger(ResyncRefsEvent::default());
}

/// On a fork, give the fork independent nested children. Copy the original's
/// `parent:*` subtree to the fork and rewrite its references, then refresh
/// the open fork.
pub fn on_branched_head_fork_nested(
    trigger: On<head::BranchedHeadEvent>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let (ca::Head::Branch(old), ca::Head::Branch(new)) = (&event.old_head, &event.new_head) else {
        return;
    };
    let ts = bevy_gantz::reg::timestamp();
    // Give the fork independent nested children. When the fork renamed a
    // nested graph to a root name, repoint the parent's references to it.
    let mut moves = gantz_egui::sync::fork_nested(&mut registry, ts, old, new);
    moves.extend(gantz_egui::sync::promote_nested(
        &mut registry,
        ts,
        old,
        new,
    ));
    refresh_cache(&registry, &mut cache, &codec.0);
    refresh_moved_heads(&moves, &mut registry, &mut heads);
}

/// Carry moved graphs' views forward to their new commits, and refresh any open
/// head whose commit moved. Reload its working graph to the new version and
/// clear its compile memo so `vm::sync` recompiles it. No re-commit is needed,
/// since the registry already holds this graph.
pub(crate) fn refresh_moved_heads(
    moves: &[gantz_egui::sync::Moved],
    registry: &mut Registry,
    heads: &mut Query<head::OpenHeadData, With<head::OpenHead>>,
) {
    if moves.is_empty() {
        return;
    }
    for m in moves {
        if gantz_egui::section::view(registry, &m.new_commit).is_none() {
            if let Some(gv) = gantz_egui::section::view(registry, &m.old_commit) {
                gantz_egui::section::set_view(&mut registry.0, m.new_commit, &gv);
            }
        }
    }
    for mut data in heads.iter_mut() {
        let ca::Head::Branch(name) = data.head_ref.0.clone() else {
            continue;
        };
        let Some(m) = moves.iter().find(|m| m.name == name) else {
            continue;
        };
        let Some(graph) = registry
            .commits()
            .get(&m.new_commit)
            .and_then(|c| registry.graph(&c.graph))
        else {
            continue;
        };
        data.working_graph.0 = graph.clone();
        *data.compiled_inputs = bevy_gantz::vm::CompiledInputs::default();
    }
}
