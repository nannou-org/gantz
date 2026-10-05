//! The Bevy side of the sync plane. The systems gather the world's inputs,
//! call [`gantz_collab_sync`] and replay its effects as triggers.

use crate::{CollabIdentity, CollabRuntime, CollabSessions, VaultLinkState, action};
use bevy_ecs::prelude::*;
use bevy_gantz::head;
use bevy_gantz::reg::Registry;
use bevy_gantz_egui::{ForHead, SyncRemoteTip};
use gantz_ca as ca;
use gantz_collab_sync::{Effect, OpenHeads};

/// Drain the runtime's events. Fetch, validate, apply and converge. Open
/// heads converge via the [`SyncRemoteTip`] observer, which migrates VM
/// state, layout and selection. Background names move headlessly, followed
/// by a reference resync.
pub(crate) fn poll_collab_events(
    runtime: Res<CollabRuntime>,
    mut sessions: ResMut<CollabSessions>,
    mut inbox: ResMut<action::ActionInbox>,
    mut registry: ResMut<Registry>,
    codec: Res<bevy_gantz_egui::NodeCodecRes>,
    mut vault_state: ResMut<VaultLinkState>,
    open: Query<
        (Entity, &head::HeadRef, Option<&bevy_gantz_egui::GraphView>),
        With<head::OpenHead>,
    >,
    mut cmds: Commands,
) {
    let Some(handle) = runtime.0.as_ref() else {
        return;
    };
    let open_heads: OpenHeads = open
        .iter()
        .filter_map(|(_, hr, gv)| match &hr.0 {
            ca::Head::Branch(name) => Some((name.clone(), gv.map(|gv| gv.0.camera))),
            ca::Head::Commit(_) => None,
        })
        .collect();
    let effects = gantz_collab_sync::poll(&mut sessions.0, &mut registry.0, handle, &open_heads);
    let vault = sessions.vault.as_ref().map(|link| link.id);
    crate::vault::track_newer_data(&mut vault_state, &registry, &codec.0, vault, &effects);
    for effect in effects {
        match effect {
            Effect::Open(name) => cmds.trigger(head::OpenEvent(ca::Head::Branch(name))),
            Effect::RemoteTip {
                name,
                remote,
                resolutions,
                adopt_unrelated,
            } => {
                let entity = open
                    .iter()
                    .find(|(_, hr, _)| matches!(&hr.0, ca::Head::Branch(n) if *n == name))
                    .map(|(entity, _, _)| entity);
                if let Some(head) = entity {
                    cmds.trigger(ForHead {
                        head,
                        data: SyncRemoteTip {
                            remote,
                            resolutions,
                            adopt_unrelated,
                        },
                    });
                }
            }
            Effect::ResyncRefs { skip } => cmds.trigger(bevy_gantz_egui::ResyncRefsEvent { skip }),
            Effect::Reset { name, to } => {
                cmds.trigger(bevy_gantz_egui::ResetHeadEvent { name, to })
            }
            Effect::Renamed { from, to } => {
                cmds.trigger(bevy_gantz_egui::RenameHeadEvent { from, to })
            }
            // Queue ephemeral actions for `action::apply_remote_actions`,
            // which runs after this system and before `VmSet`.
            Effect::Action {
                session,
                origin,
                seq,
                timestamp,
                name,
                graph,
                data,
            } => inbox.receive(action::InboundAction {
                session,
                origin,
                seq,
                timestamp,
                name,
                graph,
                data,
                received: web_time::Instant::now(),
            }),
            Effect::Moved { .. }
            | Effect::Joined { .. }
            | Effect::PeerUp { .. }
            | Effect::PeerDown { .. }
            | Effect::Error { .. } => {}
        }
    }
}

/// Mirror each dirty session's scoped closure into its served store and
/// broadcast the changed tips.
pub fn announce_sessions(
    runtime: Res<CollabRuntime>,
    identity: Option<Res<CollabIdentity>>,
    mut sessions: ResMut<CollabSessions>,
    registry: Res<Registry>,
) {
    let (Some(handle), Some(identity)) = (runtime.0.as_ref(), identity) else {
        return;
    };
    gantz_collab_sync::announce(&mut sessions.0, &registry.0, handle, identity.0.peer_id());
}
