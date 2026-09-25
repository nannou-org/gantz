//! Session lifecycle: sharing, joining, leaving, and the bookkeeping that
//! keeps open heads and the dirty flag in step with session membership.

use crate::{
    CollabIdentity, CollabRuntime, CollabSessions, JoinSessionEvent, LeaveSessionEvent, SessionRef,
    ShareSessionEvent,
};
use bevy_ecs::prelude::*;
use bevy_gantz::head;
use bevy_gantz::reg::Registry;
use bevy_log as log;
use gantz_ca as ca;
use gantz_collab::{Handle, Identity};

/// The runtime handle, spawned on first use with the user's collab
/// configuration. A later config change applies when the app restarts.
fn ensure_runtime<'a>(
    runtime: &'a mut CollabRuntime,
    identity: &Identity,
    config: &gantz_egui::collab::CollabConfig,
) -> &'a Handle {
    runtime.0.get_or_insert_with(|| {
        let infra = gantz_collab_sync::infra(config.custom_relay.as_deref());
        gantz_collab::spawn(identity.clone(), gantz_collab::RuntimeConfig { infra })
    })
}

/// Observer for [`ShareSessionEvent`]. Mints a session for the head's
/// branch, fills its served store and starts gossiping.
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
    let handle = ensure_runtime(&mut runtime, &identity.0, &gui_state.0.collab);
    let id = gantz_collab_sync::share(
        &mut sessions.0,
        &registry.0,
        handle,
        branch,
        event.access.clone(),
    );
    cmds.entity(event.head)
        .insert((SessionRef(id), bevy_gantz_egui::SessionHead));
}

/// Observer for [`JoinSessionEvent`]. Parses the ticket and asks the runtime
/// to join. The snapshot lands via `poll_collab_events`.
///
/// The session's tab opens immediately. An unknown name shows the empty
/// placeholder graph with the connecting overlay until the snapshot adopts
/// over it. An existing local graph opens as-is and reconciles then.
pub fn on_join_session(
    trigger: On<JoinSessionEvent>,
    mut runtime: ResMut<CollabRuntime>,
    identity: Option<Res<CollabIdentity>>,
    mut sessions: ResMut<CollabSessions>,
    mut registry: ResMut<Registry>,
    gui_state: Res<bevy_gantz_egui::GuiState>,
    mut cmds: Commands,
) {
    let Some(identity) = identity else {
        log::error!("JoinSession: no collab identity resource");
        return;
    };
    let handle = ensure_runtime(&mut runtime, &identity.0, &gui_state.0.collab);
    match gantz_collab_sync::join(
        &mut sessions.0,
        &mut registry.0,
        handle,
        &trigger.event().ticket,
        bevy_gantz::reg::timestamp(),
    ) {
        Ok((_, branch)) => cmds.trigger(head::OpenEvent(ca::Head::Branch(branch))),
        Err(e) => log::warn!("JoinSession: {e}"),
    }
}

/// Observer for [`LeaveSessionEvent`]. Stops gossiping and forgets the
/// session.
pub fn on_leave_session(
    trigger: On<LeaveSessionEvent>,
    runtime: Res<CollabRuntime>,
    mut sessions: ResMut<CollabSessions>,
    refs: Query<(Entity, &SessionRef)>,
    mut cmds: Commands,
) {
    let id = trigger.event().session;
    match runtime.0.as_ref() {
        Some(handle) => gantz_collab_sync::leave(&mut sessions.0, handle, id),
        None => {
            sessions.sessions.remove(&id);
        }
    }
    for (entity, session_ref) in &refs {
        if session_ref.0 == id {
            cmds.entity(entity)
                .remove::<(SessionRef, bevy_gantz_egui::SessionHead)>();
        }
    }
}

/// Mark the sessions dirty on `E` so [`crate::announce_sessions`] re-checks
/// their scoped tips. Registered once per tip-moving event. Those are local
/// commits, head navigation and settled node-moves' layout-only commits.
/// Layout-only commits skip the committed machinery, but peers still follow
/// node positions.
pub fn mark_dirty<E: Event>(_trigger: On<E>, mut sessions: ResMut<CollabSessions>) {
    sessions.dirty = true;
}

/// Keep `SessionRef` components attached to open heads whose branch is a
/// session's shared graph. This covers heads opened after the join.
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
