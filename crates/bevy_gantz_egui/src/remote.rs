//! Applying changes that arrive from elsewhere, such as a collaborative
//! session or the vault.

use crate::edit::HeadEdit;
use crate::graphs::remove_name;
use crate::hooks::refresh_moved_heads;
use crate::merge::finish_merge_commit;
use crate::{ForHead, GraphCache, GuiState, NodeCodecRes, refresh_cache};
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
pub fn on_sync_remote_tip(trigger: On<ForHead<SyncRemoteTip>>, mut edits: HeadEdit) {
    let event = trigger.event();
    let tip = &event.data;
    let moved = edits.apply(event.head, "SyncRemoteTip", |e| {
        let Some(vm) = e.vm.as_deref_mut() else {
            log::error!("SyncRemoteTip: VM not found for head");
            return None;
        };
        let old_head = e.head.clone();
        let outcome = gantz_egui::ops::sync_remote_tip(
            e.registry,
            e.head,
            e.graph,
            vm,
            e.view,
            &mut e.state.scene.interaction.selection,
            tip.remote,
            tip.resolutions,
            tip.adopt_unrelated,
        );
        match outcome {
            gantz_egui::ops::SyncTipOutcome::UpToDate => (),
            gantz_egui::ops::SyncTipOutcome::Moved(target) => {
                let entity = e.entity;
                e.cmds.trigger(head::MoveHeadEvent { entity, target });
                return Some(old_head);
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
                    entity: e.entity,
                    old_head,
                    new_head: e.head.clone(),
                };
                finish_merge_commit(e, new_commit, committed);
            }
            gantz_egui::ops::SyncTipOutcome::Blocked(reasons) => {
                log::warn!("session sync blocked: {}", reasons.join("; "));
            }
            gantz_egui::ops::SyncTipOutcome::Unrelated => {
                log::warn!("session sync: remote tip shares no history with the local graph");
            }
        }
        None
    });
    // A remote edit invalidates local redo just like a local one. A merge
    // clears it through its `CommittedEvent`. A stale session-redo would
    // otherwise mint a whole-graph revert that clobbers the peers' newer work.
    if let Some(Some(old_head)) = moved {
        edits.gui_state.redo_stacks.remove(&old_head);
        edits.gui_state.undo_cursors.remove(&old_head);
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

/// Handle [`ResyncRefsEvent`]. Bring sync-enabled `NamedRef`s up to date and
/// refresh the open heads whose commits moved.
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
