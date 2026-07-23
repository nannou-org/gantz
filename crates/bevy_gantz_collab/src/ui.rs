//! The GUI bridge: payload dispatchers, the Settings > Collab subtab
//! provider, the per-frame display mirror and presence/pointer broadcasts.

use crate::{
    CollabIdentity, CollabRuntime, CollabSessions, JoinSessionEvent, LeaveSessionEvent, SessionRef,
    ShareSessionEvent,
};
use bevy_ecs::prelude::*;
use bevy_gantz_egui::ForHead;
use bevy_log as log;
use gantz_ca as ca;
use gantz_collab::{Access, Command, ConnState, GossipMsg, Role};
use std::collections::HashMap;
use std::time::Duration;

/// A pending configuration change emitted by the Settings > Collab subtab.
#[derive(Message)]
pub struct CollabSettingsChanged(pub gantz_egui::collab::CollabConfig);

/// Dispatch a [`CollabConfig`][gantz_egui::collab::CollabConfig] payload
/// emitted by the Settings > Collab subtab as a buffered
/// [`CollabSettingsChanged`] message (registered via
/// `RegisterResponseExt::register_response_with`).
pub(crate) fn dispatch_collab_settings(
    _entity: Option<Entity>,
    payload: gantz_egui::DynResponse,
    cmds: &mut Commands,
) {
    if let Ok(config) = payload.downcast::<gantz_egui::collab::CollabConfig>() {
        cmds.write_message(CollabSettingsChanged(config));
    }
}

/// Apply any pending configuration change to the persisted GUI state, then
/// provide this frame's Collab settings tab: a fresh snapshot of the config
/// plus the displayable identity (see `bevy_gantz_egui::SettingsTabs` for
/// the First/PreUpdate schedule contract).
pub(crate) fn sync_collab_settings(
    mut msgs: MessageReader<CollabSettingsChanged>,
    mut gui_state: ResMut<bevy_gantz_egui::GuiState>,
    ui: Res<bevy_gantz_egui::CollabUi>,
    mut tabs: ResMut<bevy_gantz_egui::SettingsTabs>,
) {
    if let Some(msg) = msgs.read().last() {
        gui_state.0.collab = msg.0.clone();
    }
    tabs.0.push(Box::new(gantz_egui::widget::CollabSettingsTab {
        config: gui_state.0.collab.clone(),
        peer_id: ui.0.peer_id.clone(),
        relays: ui.0.relays.clone(),
    }));
}

/// Dispatch the app-level [`gantz_egui::JoinSession`] payload.
pub(crate) fn dispatch_join_session(
    _entity: Option<Entity>,
    payload: gantz_egui::DynResponse,
    cmds: &mut Commands,
) {
    let gantz_egui::JoinSession { ticket } = bevy_gantz_egui::downcast_payload(payload);
    cmds.trigger(JoinSessionEvent { ticket });
}

/// Map the GUI's share payload to a [`ShareSessionEvent`].
pub(crate) fn on_share_head_payload(
    trigger: On<ForHead<gantz_egui::ShareHead>>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    let access = if event.data.public {
        Access::Public
    } else {
        Access::Restricted(Default::default())
    };
    cmds.trigger(ShareSessionEvent {
        head: event.head,
        access,
    });
}

/// Map the GUI's stop-sharing payload to a [`LeaveSessionEvent`].
pub(crate) fn on_stop_sharing_payload(
    trigger: On<ForHead<gantz_egui::StopSharing>>,
    refs: Query<&SessionRef>,
    mut cmds: Commands,
) {
    let Ok(session_ref) = refs.get(trigger.event().head) else {
        log::warn!("StopSharing: head has no session");
        return;
    };
    cmds.trigger(LeaveSessionEvent {
        session: session_ref.0,
    });
}

/// Mirror the session state into the GUI's display resource.
pub fn update_collab_ui(
    sessions: Res<CollabSessions>,
    identity: Option<Res<CollabIdentity>>,
    mut ui: ResMut<bevy_gantz_egui::CollabUi>,
) {
    let state = &mut ui.0;
    // The full-hex public key, for copying/allowlisting.
    state.peer_id = identity.map(|i| {
        i.0.peer_id()
            .0
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    });
    state.relays = sessions.relays.clone();
    // Recompute cheaply each frame: session counts are tiny.
    state.sessions.clear();
    for session_state in sessions.sessions.values() {
        let display = gantz_egui::collab::SessionDisplay {
            is_host: matches!(session_state.session.role, Role::Host),
            conn: match session_state.conn {
                ConnState::Connecting => gantz_egui::collab::SessionConn::Connecting,
                ConnState::Live => gantz_egui::collab::SessionConn::Live,
                ConnState::Degraded => gantz_egui::collab::SessionConn::Degraded,
            },
            awaiting_snapshot: session_state.placeholder.is_some(),
            peers: session_state
                .peers
                .iter()
                .map(|(id, name)| gantz_egui::collab::PeerDisplay {
                    id: id.to_string(),
                    name: name.clone(),
                })
                .collect(),
            ticket: session_state.ticket.clone(),
            conflicts: session_state.conflicts,
            error: session_state.error.clone(),
            pointers: session_state
                .pointers
                .iter()
                .filter(|(_, p)| p.at.elapsed() < POINTER_TTL)
                .filter_map(|(peer, p)| {
                    let pos = p.pos?;
                    let label = session_state
                        .peers
                        .get(peer)
                        .and_then(|name| name.clone())
                        .unwrap_or_else(|| peer.to_string());
                    Some(gantz_egui::collab::PointerDisplay::new(pos, label, &peer.0))
                })
                .collect(),
        };
        state.sessions.insert(session_state.branch_name(), display);
    }
}

/// Broadcast presence (and the configured username) whenever it changes or a
/// peer joins, so newcomers learn who we are.
pub fn broadcast_presence(
    runtime: Res<CollabRuntime>,
    identity: Option<Res<CollabIdentity>>,
    sessions: Res<CollabSessions>,
    gui_state: Res<bevy_gantz_egui::GuiState>,
    mut last: Local<Option<(String, usize)>>,
) {
    let (Some(handle), Some(identity)) = (runtime.0.as_ref(), identity) else {
        return;
    };
    let username = gui_state.0.collab.username.trim().to_string();
    // Re-broadcast when the username changes or the peer set grows.
    let peer_total: usize = sessions.sessions.values().map(|s| s.peers.len()).sum();
    let key = (username.clone(), peer_total);
    if last.as_ref() == Some(&key) {
        return;
    }
    *last = Some(key);
    let origin = identity.0.peer_id();
    let name = (!username.is_empty()).then_some(username);
    for id in sessions.sessions.keys() {
        let msg = GossipMsg::Presence {
            origin,
            name: name.clone(),
        };
        let _ = handle
            .cmds
            .try_send(Command::Broadcast { session: *id, msg });
    }
}

/// How long a received pointer stays displayable without an update; the
/// sender keepalive below refreshes well within it.
pub(crate) const POINTER_TTL: Duration = Duration::from_secs(3);

/// How often a resting (unmoved, still hovering) pointer re-announces, so
/// receiver expiry never hides a live cursor.
const POINTER_KEEPALIVE: Duration = Duration::from_secs(1);

/// Per-branch pointer send bookkeeping (see [`broadcast_pointers`]).
#[derive(Default)]
pub(crate) struct PointerSendState {
    seq: u64,
    /// The last position sent per session branch, and when.
    last: HashMap<ca::Name, (Option<(f32, f32)>, web_time::Instant)>,
}

/// Broadcast this peer's live pointer over each session's shared graph.
///
/// The position is read from the branch head's scene interaction state in
/// graph-space coordinates (camera-independent). Movement coalesces to the
/// newest position at the configured action rate; leaving the scene sends
/// one final `pos: None` immediately; a keepalive re-announces a resting
/// pointer so receiver expiry never hides it.
pub(crate) fn broadcast_pointers(
    runtime: Res<CollabRuntime>,
    identity: Option<Res<CollabIdentity>>,
    sessions: Res<CollabSessions>,
    gui_state: Res<bevy_gantz_egui::GuiState>,
    mut state: Local<PointerSendState>,
) {
    let (Some(handle), Some(identity)) = (runtime.0.as_ref(), identity) else {
        return;
    };
    let origin = identity.0.peer_id();
    let rate = Duration::from_millis(gui_state.0.collab.action_rate_ms);
    let now = web_time::Instant::now();
    for (id, session_state) in &sessions.sessions {
        let name = session_state.branch_name();
        let head = ca::Head::Branch(name.clone());
        let pos = gui_state
            .0
            .open_heads
            .get(&head)
            .and_then(|s| s.scene.interaction.live_pointer)
            .map(|p| (p.x, p.y));
        let send = match state.last.get(&name).copied() {
            // Never announced: only a live position is worth starting with.
            None => pos.is_some(),
            Some((prev, at)) => {
                if pos != prev {
                    // Leaving announces immediately; movement coalesces to
                    // the newest position at the configured rate.
                    pos.is_none() || now.duration_since(at) >= rate
                } else {
                    pos.is_some() && now.duration_since(at) >= POINTER_KEEPALIVE
                }
            }
        };
        if !send {
            continue;
        }
        state.seq += 1;
        let msg = GossipMsg::Pointer {
            origin,
            seq: state.seq,
            name: name.clone(),
            pos,
        };
        let _ = handle
            .cmds
            .try_send(Command::Broadcast { session: *id, msg });
        state.last.insert(name, (pos, now));
    }
    // Drop bookkeeping for sessions that ended.
    state
        .last
        .retain(|n, _| sessions.sessions.values().any(|s| s.branch_name() == *n));
}
