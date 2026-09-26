//! Bevy integration of gantz's peer-to-peer collaborative sessions.
//!
//! [`CollabPlugin`] bridges the [`gantz_collab`] runtime into the app.
//!
//! Outbound: any local commit marks the session state dirty.
//! [`announce_sessions`] then mirrors the session's scoped closure into the
//! served [`gantz_collab::SessionRegistry`] and broadcasts the changed tips.
//! Fast-forwards and adoptions of received tips are never re-announced.
//!
//! Inbound: `poll_collab_events` drains the runtime, drives the want and
//! fetch loop through `gantz_ca::sync::Staged` validation, applies completed
//! closures to the registry and converges each scoped name. Open heads
//! converge via the [`bevy_gantz_egui::SyncRemoteTip`] observer, which also
//! migrates VM state, layout and selection. Background names move headlessly,
//! followed by a reference resync.
//!
//! The pure `gantz_ca::sync` rules decide what to merge and in which
//! orientation. Everything here is bookkeeping around them.

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_gantz::head;
use gantz_collab::{Access, Handle, Identity, SessionId};
pub use gantz_collab_sync::{PeerPointer, PendingTip, SessionState, Sessions, session_resolutions};
pub use session::{
    attach_session_refs, mark_dirty, on_join_session, on_leave_session, on_share_session,
};
pub use sync::announce_sessions;
pub(crate) use sync::poll_collab_events;
pub use ui::{CollabSettingsChanged, broadcast_presence, update_collab_ui};
use ui::{
    dispatch_collab_settings, dispatch_join_session, on_share_head_payload,
    on_stop_sharing_payload, sync_collab_settings,
};

pub mod action;
mod session;
pub mod storage;
mod sync;
mod ui;

/// The plugin. Registers the session resources, observers and systems.
///
/// Requires `bevy_gantz::GantzPlugin` and `bevy_gantz_egui`'s plugin. The
/// app provides the [`CollabIdentity`] resource at startup. Sharing and
/// joining are requested via [`ShareSessionEvent`] and [`JoinSessionEvent`]
/// triggers.
#[derive(Default)]
pub struct CollabPlugin;

/// The user's collaborative identity, provided by the app at startup.
#[derive(Resource)]
pub struct CollabIdentity(pub Identity);

/// The network runtime handle, spawned lazily on first share/join.
#[derive(Default, Resource)]
pub struct CollabRuntime(pub Option<Handle>);

/// All local session state, keyed by session id. See
/// [`gantz_collab_sync::Sessions`].
#[derive(Default, Resource)]
pub struct CollabSessions(pub Sessions);

impl std::ops::Deref for CollabSessions {
    type Target = Sessions;
    fn deref(&self) -> &Sessions {
        &self.0
    }
}

impl std::ops::DerefMut for CollabSessions {
    fn deref_mut(&mut self) -> &mut Sessions {
        &mut self.0
    }
}

/// Attached to an open head entity participating in a session.
#[derive(Component)]
pub struct SessionRef(pub SessionId);

/// Request sharing the graph open on `head` as a new session.
#[derive(Debug, Event)]
pub struct ShareSessionEvent {
    pub head: Entity,
    pub access: Access,
}

/// Request joining a session from an invite ticket string.
#[derive(Debug, Event)]
pub struct JoinSessionEvent {
    pub ticket: String,
}

/// Request leaving and forgetting a session.
#[derive(Debug, Event)]
pub struct LeaveSessionEvent {
    pub session: SessionId,
}

impl Plugin for CollabPlugin {
    fn build(&self, app: &mut App) {
        use bevy_gantz_egui::RegisterResponseExt;
        // The Settings > Collab subtab's `CollabConfig` payloads dispatch
        // into a buffered message. `sync_collab_settings` applies it and
        // re-snapshots it into the tab.
        app.init_resource::<bevy_gantz_egui::SettingsTabs>()
            .add_message::<CollabSettingsChanged>()
            .register_response_with::<gantz_egui::collab::CollabConfig>(dispatch_collab_settings)
            .add_systems(PreUpdate, sync_collab_settings);
        app.init_resource::<CollabRuntime>()
            .init_resource::<CollabSessions>()
            .init_resource::<bevy_gantz_egui::CollabUi>()
            .init_resource::<action::ActionOutbox>()
            .init_resource::<action::ActionInbox>()
            .init_resource::<action::ActionLog>()
            .register_head_response::<gantz_egui::ShareHead>()
            .register_head_response::<gantz_egui::StopSharing>()
            .register_response_with::<gantz_egui::JoinSession>(dispatch_join_session)
            // Capture overrides. The last registration wins, and the app adds
            // this plugin after `bevy_gantz_egui`'s.
            .register_response_with::<gantz_egui::StateWritten>(action::dispatch_state_written)
            .register_response_with::<gantz_egui::EvalEntry>(action::dispatch_eval_entry)
            .add_observer(action::on_capture_write)
            .add_observer(action::on_capture_eval)
            .add_observer(on_share_head_payload)
            .add_observer(on_stop_sharing_payload)
            .add_observer(on_share_session)
            .add_observer(on_join_session)
            .add_observer(on_leave_session)
            .add_observer(mark_dirty::<head::CommittedEvent>)
            .add_observer(mark_dirty::<head::ChangedEvent>)
            .add_observer(mark_dirty::<bevy_gantz_egui::LayoutCommittedEvent>)
            .add_systems(
                Update,
                (
                    (
                        poll_collab_events,
                        action::apply_remote_actions.after(poll_collab_events),
                        attach_session_refs,
                    )
                        .before(bevy_gantz::VmSet),
                    (
                        // The announce runs after the view persistence passes
                        // so a commit minted this frame has its view seeded
                        // before its tip can be served to peers.
                        announce_sessions.after(bevy_gantz_egui::ViewPersistSet),
                        action::broadcast_actions,
                        broadcast_presence,
                        ui::broadcast_pointers,
                        update_collab_ui,
                    )
                        .after(bevy_gantz::VmSet),
                ),
            );
    }
}
