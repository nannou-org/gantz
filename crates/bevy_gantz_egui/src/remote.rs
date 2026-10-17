//! Applying changes that arrive from elsewhere, such as a collaborative
//! session or the vault.

use crate::graphs::remove_name;
use crate::hooks::refresh_moved_heads;
use crate::merge::finish_merge_commit;
use crate::{BuiltinNodes, ForHead, GraphCache, GraphView, GuiState, NodeCodecRes, refresh_cache};
use bevy_ecs::prelude::*;
use bevy_egui::EguiContexts;
use bevy_gantz::Registry;
use bevy_gantz::head;
use bevy_log as log;
use gantz_ca as ca;
use std::collections::BTreeSet;

/// Bring an open head up to date with a remote session tip. See
/// [`gantz_egui::ops::sync_remote_tip`].
///
/// The collaborative-session layer triggers it as `ForHead<SyncRemoteTip>`
/// once the remote tip's closure has been fetched, validated and applied to
/// the registry. Not a GUI payload. Nothing emits it from widgets.
#[derive(Clone, Copy, Debug)]
pub struct SyncRemoteTip {
    /// The remote tip to converge with.
    pub remote: ca::CommitAddr,
    /// The session's fixed conflict-resolution policy.
    pub resolutions: ca::merge::Resolutions,
    /// Adopt an unrelated remote tip instead of surfacing it. Set when the
    /// head points at the join flow's placeholder graph.
    pub adopt_unrelated: bool,
}

/// Handle [`SyncRemoteTip`]. The session analogue of [`on_merge_head`].
///
/// [`on_merge_head`]: crate::on_merge_head
pub fn on_sync_remote_tip(
    trigger: On<ForHead<SyncRemoteTip>>,
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
        log::error!("SyncRemoteTip: head not found for entity {:?}", event.head);
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("SyncRemoteTip: GUI state not found for head");
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("SyncRemoteTip: VM not found for head");
        return;
    };

    let old_head = head_ref.0.clone();
    let outcome = gantz_egui::ops::sync_remote_tip(
        &mut registry,
        &mut head_ref.0,
        &mut wg,
        vm,
        &mut gv,
        &mut head_state.scene.interaction.selection,
        event.data.remote,
        event.data.resolutions,
        event.data.adopt_unrelated,
    );
    // The sync may have minted a merge commit and graph.
    refresh_cache(&registry, &mut cache, &codec.0);

    match outcome {
        gantz_egui::ops::SyncTipOutcome::UpToDate => (),
        gantz_egui::ops::SyncTipOutcome::Moved(target) => {
            // A remote edit invalidates local redo just like a local one. The
            // Merged arm clears via CommittedEvent. A stale session-redo would
            // otherwise mint a whole-graph revert that clobbers the peers'
            // newer work.
            gui_state.redo_stacks.remove(&old_head);
            gui_state.undo_cursors.remove(&old_head);
            cmds.trigger(head::MoveHeadEvent {
                entity: event.head,
                target,
            });
        }
        gantz_egui::ops::SyncTipOutcome::Merged {
            new_commit,
            conflicts,
            ..
        } => {
            if conflicts > 0 {
                log::info!("session merge auto-resolved {conflicts} conflict(s)");
            }
            log::debug!("session merge -> {}", new_commit.display_short());
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
        gantz_egui::ops::SyncTipOutcome::Blocked(reasons) => {
            log::warn!("session sync blocked: {}", reasons.join("; "));
        }
        gantz_egui::ops::SyncTipOutcome::Unrelated => {
            log::warn!("session sync: remote tip shares no history with the local graph");
        }
    }
}

/// Request a reference resync outside the usual committed flow. Bring
/// sync-enabled `NamedRef`s up to date and refresh open heads whose commits
/// moved.
///
/// A head move fires no [`head::CommittedEvent`], so the referrers of a moved
/// graph follow only via this event. [`on_undo`], [`on_redo`] and a merge
/// fast-forward trigger it after their [`head::MoveHeadEvent`]. The
/// collaborative-session layer triggers it after it moves scoped names that
/// no open head points at, such as fast-forwards of nested or referenced
/// graphs.
///
/// [`on_undo`]: crate::on_undo
/// [`on_redo`]: crate::on_redo
#[derive(Debug, Default, Event)]
pub struct ResyncRefsEvent {
    /// Names whose referrers keep their references as they are. See
    /// [`gantz_egui::sync::resync_except`].
    pub skip: BTreeSet<ca::Name>,
}

/// Move the open heads on `name` to exactly `to`, or detach them and remove
/// the name when `to` is `None`. The vault layer triggers it for a change
/// that contains everything the local head holds.
#[derive(Debug, Event)]
pub struct ResetHeadEvent {
    pub name: ca::Name,
    pub to: Option<ca::CommitAddr>,
}

/// Point the open heads on `from` at `to`, carrying their GUI state. For a
/// name another layer already moved in the registry, such as a vault moving
/// a local graph aside.
#[derive(Debug, Event)]
pub struct RenameHeadEvent {
    pub from: ca::Name,
    pub to: ca::Name,
}

/// Handle [`ResyncRefsEvent`]. The same pass as [`on_head_committed_resync`].
///
/// [`on_head_committed_resync`]: crate::on_head_committed_resync
pub fn on_resync_refs(
    trigger: On<ResyncRefsEvent>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
) {
    let skip = &trigger.event().skip;
    let moves = gantz_egui::sync::resync_except(&mut registry, bevy_gantz::reg::timestamp(), skip);
    refresh_cache(&registry, &mut cache, &codec.0);
    refresh_moved_heads(&moves, &mut registry, &mut heads);
}

/// Handle [`ResetHeadEvent`].
pub fn on_reset_head(
    trigger: On<ResetHeadEvent>,
    mut registry: ResMut<Registry>,
    mut gui_state: ResMut<GuiState>,
    mut heads: Query<(Entity, &mut head::HeadRef), With<head::OpenHead>>,
    mut cmds: Commands,
) {
    let ResetHeadEvent { name, to } = trigger.event();
    let head = ca::Head::Branch(name.clone());
    // A remote change invalidates local redo, as in `on_sync_remote_tip`.
    gui_state.redo_stacks.remove(&head);
    gui_state.undo_cursors.remove(&head);
    let Some(target) = *to else {
        let open = heads.iter_mut().map(|(_, head_ref)| head_ref);
        remove_name(&mut registry, open, name);
        return;
    };
    let mut open = false;
    for (entity, head_ref) in &heads {
        if **head_ref == head {
            cmds.trigger(head::MoveHeadEvent { entity, target });
            open = true;
        }
    }
    // The head closed since the reset was planned. Move the name alone.
    if !open {
        registry.set_head(name.clone(), target);
    }
}

/// Handle [`RenameHeadEvent`].
pub fn on_rename_head(
    trigger: On<RenameHeadEvent>,
    mut gui_state: ResMut<GuiState>,
    mut heads: Query<&mut head::HeadRef, With<head::OpenHead>>,
    mut ctxs: EguiContexts,
) {
    let RenameHeadEvent { from, to } = trigger.event();
    let old_head = ca::Head::Branch(from.clone());
    let new_head = ca::Head::Branch(to.clone());
    for mut head_ref in &mut heads {
        if **head_ref != old_head {
            continue;
        }
        **head_ref = new_head.clone();
        gui_state.migrate_head(&old_head, &new_head, false);
        if let Ok(ctx) = ctxs.ctx_mut() {
            gantz_egui::widget::update_graph_pane_head(ctx, &old_head, &new_head);
        }
    }
}
