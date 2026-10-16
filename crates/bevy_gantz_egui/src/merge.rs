//! Merging a branch into a head.

use crate::edit::{Edit, HeadEdit};
use crate::{ForHead, ResyncRefsEvent};
use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head;
use bevy_log as log;
use gantz_ca as ca;

/// Seed `commit`'s stored view. See [`gantz_egui::section::seed_view`].
pub fn seed_view(registry: &mut Registry, commit: ca::CommitAddr, view: gantz_edit::SceneView) {
    gantz_egui::section::seed_view(&mut registry.0, commit, &view);
}

/// Finish a locally-minted merge commit. The op has already committed with
/// both parents, for a local merge or session convergence. Seed the minted
/// commit's view from the migrated live layout so it exists before any
/// same-frame session announce. A viewless tip auto-layouts on adopting
/// peers. Re-register the root graph so merged-in nodes get their state
/// initialized. Then fire the committed machinery. That is GUI-state
/// migration, redo-stack clear, NamedRef resync and `vm::sync` recompile.
pub(crate) fn finish_merge_commit(
    e: &mut Edit,
    new_commit: ca::CommitAddr,
    committed: head::CommittedEvent,
) {
    seed_view(e.registry, new_commit, e.view.clone());
    e.register_graph("merge finish");
    e.cmds.trigger(committed);
}

/// Handle merge payloads. Merge the named source branch into the head.
///
/// A fast-forward navigates the head to the source's tip and reloads the
/// working graph, VM and views. A true merge is applied to the working graph
/// and committed with two parents by [`gantz_egui::ops::merge_head`] itself.
/// So this triggers [`head::CommittedEvent`] directly. `commit_working_graph`
/// would see the already-committed graph and skip the event.
pub fn on_merge_head(trigger: On<ForHead<gantz_egui::MergeHead>>, mut edits: HeadEdit) {
    let event = trigger.event();
    let merge = &event.data;
    edits.apply(event.head, "MergeHead", |e| {
        let Some(vm) = e.vm.as_deref_mut() else {
            log::error!("MergeHead: VM not found for head");
            return;
        };
        let old_head = e.head.clone();
        let outcome = gantz_egui::ops::merge_head(
            e.registry,
            bevy_gantz::reg::timestamp(),
            e.head,
            e.graph,
            vm,
            e.view,
            &mut e.state.scene.interaction.selection,
            &merge.source,
            merge.resolutions,
            merge.auto_resolve,
        );
        match outcome {
            gantz_egui::ops::MergeHeadOutcome::FastForward(target) => {
                let entity = e.entity;
                e.cmds.trigger(head::MoveHeadEvent { entity, target });
                e.cmds.trigger(ResyncRefsEvent::default());
            }
            gantz_egui::ops::MergeHeadOutcome::Merged { new_commit, .. } => {
                log::debug!(
                    "Merged '{}' -> {}",
                    merge.source,
                    new_commit.display_short()
                );
                let committed = head::CommittedEvent {
                    entity: e.entity,
                    old_head,
                    new_head: e.head.clone(),
                };
                finish_merge_commit(e, new_commit, committed);
            }
            gantz_egui::ops::MergeHeadOutcome::Refused(reasons) => {
                // The UI disables conflicted and blocked candidates.
                log::warn!(
                    "MergeHead: refused to merge '{}': {}",
                    merge.source,
                    reasons.join("; ")
                );
            }
            gantz_egui::ops::MergeHeadOutcome::Noop => (),
        }
    });
}
