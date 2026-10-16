//! Merging a branch into a head.

use crate::{
    BuiltinNodes, ForHead, GraphCache, GraphView, GuiState, NodeCodecRes, ResyncRefsEvent, env,
    refresh_cache,
};
use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head;
use bevy_log as log;
use gantz_ca as ca;

/// Seed `commit`'s stored view. See [`gantz_egui::section::seed_view`].
pub fn seed_view(registry: &mut Registry, commit: ca::CommitAddr, view: gantz_egui::SceneView) {
    gantz_egui::section::seed_view(&mut registry.0, commit, &view);
}

/// Finish a locally-minted merge commit. The op has already committed with
/// both parents, for a local merge or session convergence. Seed the minted
/// commit's view from the migrated live layout so it exists before any
/// same-frame session announce. A viewless tip auto-layouts on adopting
/// peers. Re-register the root graph so merged-in nodes get their state
/// initialized. Then fire the committed machinery. That is GUI-state
/// migration, redo-stack clear, NamedRef resync and `vm::sync` recompile.
#[allow(clippy::too_many_arguments)] // A cohesive tail over ECS-owned data.
pub(crate) fn finish_merge_commit(
    new_commit: ca::CommitAddr,
    committed: head::CommittedEvent,
    graph: &ca::DataGraph,
    live_view: &gantz_egui::SceneView,
    registry: &mut Registry,
    (cache, builtins, codec): (&GraphCache, &BuiltinNodes, &NodeCodecRes),
    vm: &mut steel::steel_vm::engine::Engine,
    cmds: &mut Commands,
) {
    seed_view(registry, new_commit, live_view.clone());
    let node_reg = env(registry, cache, builtins, codec);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    match codec.0.reify_graph(graph) {
        Ok(g) => gantz_core::graph::register(&get_node, &g, &[], vm),
        Err(e) => log::error!("merge finish: cannot re-register the merged graph: {e}"),
    }
    cmds.trigger(committed);
}

/// Handle merge payloads. Merge the named source branch into the head.
///
/// A fast-forward navigates the head to the source's tip and reloads the
/// working graph, VM and views. A true merge is applied to the working graph
/// and committed with two parents by [`gantz_egui::ops::merge_head`] itself.
/// So this triggers [`head::CommittedEvent`] directly. `commit_working_graph`
/// would see the already-committed graph and skip the event.
pub fn on_merge_head(
    trigger: On<ForHead<gantz_egui::MergeHead>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<
        (&mut head::HeadRef, &mut head::WorkingGraph, &mut GraphView),
        With<head::OpenHead>,
    >,
) {
    let event = trigger.event();
    let Ok((mut head_ref, mut wg, mut gv)) = heads.get_mut(event.head) else {
        log::error!("MergeHead: head not found for entity {:?}", event.head);
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("MergeHead: GUI state not found for head");
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("MergeHead: VM not found for head");
        return;
    };

    let old_head = head_ref.0.clone();
    let outcome = gantz_egui::ops::merge_head(
        &mut registry,
        bevy_gantz::reg::timestamp(),
        &mut head_ref.0,
        &mut wg,
        vm,
        &mut gv,
        &mut head_state.scene.interaction.selection,
        &event.data.source,
        event.data.resolutions,
        event.data.auto_resolve,
    );
    // The merge may have committed new graphs.
    refresh_cache(&registry, &mut cache, &codec.0);

    match outcome {
        gantz_egui::ops::MergeHeadOutcome::FastForward(target) => {
            cmds.trigger(head::MoveHeadEvent {
                entity: event.head,
                target,
            });
            cmds.trigger(ResyncRefsEvent::default());
        }
        gantz_egui::ops::MergeHeadOutcome::Merged { new_commit, .. } => {
            log::debug!(
                "Merged '{}' -> {}",
                event.data.source,
                new_commit.display_short()
            );
            let committed = head::CommittedEvent {
                entity: event.head,
                old_head,
                new_head: head_ref.0.clone(),
            };
            finish_merge_commit(
                new_commit,
                committed,
                &wg.0,
                &gv.0,
                &mut registry,
                (&cache, &builtins, &codec),
                vm,
                &mut cmds,
            );
        }
        gantz_egui::ops::MergeHeadOutcome::Refused(reasons) => {
            // The UI disables conflicted and blocked candidates.
            log::warn!(
                "MergeHead: refused to merge '{}': {}",
                event.data.source,
                reasons.join("; ")
            );
        }
        gantz_egui::ops::MergeHeadOutcome::Noop => (),
    }
}
