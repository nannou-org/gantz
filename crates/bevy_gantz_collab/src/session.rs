//! Session lifecycle: sharing, joining, leaving, and the bookkeeping that
//! keeps open heads and the dirty flag in step with session membership.

use crate::{
    CollabIdentity, CollabRuntime, CollabSessions, JoinSessionEvent, LeaveSessionEvent, SessionRef,
    SessionState, ShareSessionEvent, sync::serve_scope,
};
use bevy_ecs::prelude::*;
use bevy_gantz::head;
use bevy_gantz::reg::Registry;
use bevy_log as log;
use gantz_ca as ca;
use gantz_collab::{
    Command, Handle, Identity, Role, Session, SessionEntry, SessionId, SessionRegistry,
};

/// The fixed conflict policy for shared sessions: last edit wins, edits beat
/// deletes. Symmetric, so independently merging peers converge.
pub fn session_resolutions() -> ca::merge::Resolutions {
    ca::merge::Resolutions {
        both_modified: ca::merge::BothModified::KeepNewest,
        delete_modify: ca::merge::EditOrDelete::KeepEdit,
    }
}

/// The runtime handle, spawning it on first use with the user's collab
/// configuration (a later config change applies when the app restarts).
fn ensure_runtime<'a>(
    runtime: &'a mut CollabRuntime,
    identity: &Identity,
    config: &gantz_egui::collab::CollabConfig,
) -> &'a Handle {
    runtime.0.get_or_insert_with(|| {
        let infra = match config.custom_relay.as_deref() {
            // A custom relay means self-hosted infrastructure: nothing n0.
            // Peers reach each other via invite-ticket addresses and the
            // relay itself, so no address-lookup service is required.
            Some(url) => gantz_collab::Infra::Custom {
                relays: vec![url.to_string()],
                pkarr: None,
            },
            None => gantz_collab::Infra::N0,
        };
        gantz_collab::spawn(identity.clone(), gantz_collab::RuntimeConfig { infra })
    })
}

/// Handle [`ShareSessionEvent`]: mint a session for the head's branch, fill
/// its served store, and start gossiping.
pub fn on_share_session(
    trigger: On<ShareSessionEvent>,
    mut runtime: ResMut<CollabRuntime>,
    identity: Option<Res<CollabIdentity>>,
    mut sessions: ResMut<CollabSessions>,
    registry: Res<Registry>,
    gui_state: Res<bevy_gantz_egui::GuiState>,
    heads: Query<&head::HeadRef, With<head::OpenHead>>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    let Some(identity) = identity else {
        log::error!("ShareSession: no collab identity resource");
        return;
    };
    let Ok(head_ref) = heads.get(event.head) else {
        log::error!("ShareSession: head not found for entity {:?}", event.head);
        return;
    };
    let ca::Head::Branch(branch) = &head_ref.0 else {
        log::warn!("ShareSession: only named graphs can be shared");
        return;
    };
    let session = Session {
        id: SessionId::generate(),
        branch: branch.to_string(),
        access: event.access.clone(),
        resolutions: session_resolutions(),
        role: Role::Host,
    };
    let id = session.id;
    let scope = gantz_egui::sync::session_scope(&registry, branch);
    let mut state = SessionState::new(session.clone());

    let handle = ensure_runtime(&mut runtime, &identity.0, &gui_state.0.collab);
    let _ = handle.cmds.try_send(Command::Register(SessionEntry {
        session,
        store: SessionRegistry::default(),
    }));
    serve_scope(handle, &mut state, &registry, &scope);
    state.last_announced = state.served_heads.clone();
    sessions.sessions.insert(id, state);
    cmds.entity(event.head)
        .insert((SessionRef(id), bevy_gantz_egui::SessionHead));
    if handle.cmds.try_send(Command::Share(id)).is_err() {
        log::error!("ShareSession: collab runtime is gone");
    }
}

/// Handle [`JoinSessionEvent`]: parse the ticket and ask the runtime to
/// join; the snapshot lands via `poll_collab_events`.
pub fn on_join_session(
    trigger: On<JoinSessionEvent>,
    mut runtime: ResMut<CollabRuntime>,
    identity: Option<Res<CollabIdentity>>,
    mut sessions: ResMut<CollabSessions>,
    mut registry: ResMut<Registry>,
    gui_state: Res<bevy_gantz_egui::GuiState>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    let Some(identity) = identity else {
        log::error!("JoinSession: no collab identity resource");
        return;
    };
    let ticket: gantz_collab::SessionTicket = match event.ticket.trim().parse() {
        Ok(ticket) => ticket,
        Err(e) => {
            log::warn!("JoinSession: invalid ticket: {e}");
            return;
        }
    };
    if ticket.proto != gantz_collab::PROTO_VERSION {
        log::warn!(
            "JoinSession: protocol mismatch (ticket v{}, this build v{})",
            ticket.proto,
            gantz_collab::PROTO_VERSION
        );
        return;
    }
    let session = Session {
        id: ticket.session,
        branch: ticket.name.clone(),
        access: ticket.access.clone(),
        resolutions: ticket.resolutions,
        role: Role::Guest,
    };
    let id = session.id;
    let handle = ensure_runtime(&mut runtime, &identity.0, &gui_state.0.collab);
    let _ = handle.cmds.try_send(Command::Register(SessionEntry {
        session: session.clone(),
        store: SessionRegistry::default(),
    }));
    let mut state = SessionState::new(session);

    // Open the session's tab immediately: when the name is unknown locally,
    // mint an empty placeholder graph for it (recorded so the snapshot
    // adopts over it rather than renaming it aside); an existing local graph
    // opens as-is and reconciles when the snapshot lands. Either way the
    // scene shows the connecting overlay until then.
    let branch: ca::Name = ticket.name.parse().expect("names parse infallibly");
    if registry.head(&branch).is_none() {
        let graph = ca::DataGraph::default();
        let graph_ca = ca::graph_addr(&graph);
        let placeholder =
            registry.commit_graph(bevy_gantz::reg::timestamp(), None, graph_ca, || graph);
        registry.set_head(branch.clone(), placeholder);
        state.placeholder = Some(placeholder);
    }
    sessions.sessions.insert(id, state);
    cmds.trigger(head::OpenEvent(ca::Head::Branch(branch)));

    if handle.cmds.try_send(Command::Join(ticket)).is_err() {
        log::error!("JoinSession: collab runtime is gone");
    }
}

/// Handle [`LeaveSessionEvent`]: stop gossiping and forget the session.
pub fn on_leave_session(
    trigger: On<LeaveSessionEvent>,
    runtime: Res<CollabRuntime>,
    mut sessions: ResMut<CollabSessions>,
    refs: Query<(Entity, &SessionRef)>,
    mut cmds: Commands,
) {
    let id = trigger.event().session;
    sessions.sessions.remove(&id);
    if let Some(handle) = runtime.0.as_ref() {
        let _ = handle.cmds.try_send(Command::Leave(id));
        let _ = handle.cmds.try_send(Command::Forget(id));
    }
    for (entity, session_ref) in &refs {
        if session_ref.0 == id {
            cmds.entity(entity)
                .remove::<(SessionRef, bevy_gantz_egui::SessionHead)>();
        }
    }
}

/// Mark the sessions dirty on `E` so [`announce_sessions`][crate::announce_sessions] re-checks their
/// scoped tips. Registered once per tip-moving event: local commits
/// (`head::CommittedEvent`), head navigation (`head::ChangedEvent` - e.g.
/// history-pane moves), and settled node-moves' layout-only commits
/// (`LayoutCommittedEvent` - no committed machinery, but peers still follow
/// node positions).
pub fn mark_dirty<E: Event>(_trigger: On<E>, mut sessions: ResMut<CollabSessions>) {
    sessions.dirty = true;
}

/// Keep `SessionRef` components attached to open heads whose branch is a
/// session's shared graph (covers heads opened after the join).
pub fn attach_session_refs(
    sessions: Res<CollabSessions>,
    open: Query<(Entity, &head::HeadRef), (With<head::OpenHead>, Without<SessionRef>)>,
    mut cmds: Commands,
) {
    if sessions.sessions.is_empty() {
        return;
    }
    for (entity, head_ref) in &open {
        let ca::Head::Branch(name) = &head_ref.0 else {
            continue;
        };
        if let Some((id, _)) = sessions
            .sessions
            .iter()
            .find(|(_, s)| s.branch_name() == *name)
        {
            cmds.entity(entity)
                .insert((SessionRef(*id), bevy_gantz_egui::SessionHead));
        }
    }
}
