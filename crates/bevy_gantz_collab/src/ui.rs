//! The GUI bridge: payload dispatchers, the Settings > Collab subtab
//! provider, the per-frame display mirror and presence broadcasts.

use crate::{
    CollabIdentity, CollabRuntime, CollabSessions, JoinSessionEvent, LeaveSessionEvent, SessionRef,
    ShareSessionEvent,
};
use bevy_ecs::prelude::*;
use bevy_gantz_egui::ForHead;
use bevy_log as log;
use gantz_collab::{Access, Command, ConnState, GossipMsg, Role};

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
