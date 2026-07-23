//! Bevy integration of gantz's peer-to-peer collaborative sessions (#286).
//!
//! [`CollabPlugin`] bridges the [`gantz_collab`] runtime into the app:
//!
//! - **Outbound**: any local commit marks the session state dirty;
//!   [`announce_sessions`] then mirrors the session's scoped closure into
//!   the served [`SessionRegistry`](gantz_collab::SessionRegistry) and
//!   broadcasts the changed tips. Fast-forwards and adoptions of *received*
//!   tips are never re-announced (echo suppression).
//! - **Inbound**: `poll_collab_events` drains the runtime, drives the
//!   want/fetch loop through `gantz_ca::sync::Staged` validation, applies
//!   completed closures to the registry and converges each scoped name -
//!   open heads via `bevy_gantz_egui`'s [`SyncRemoteTip`][bevy_gantz_egui::SyncRemoteTip] observer (VM
//!   state/layout/selection migration included), background names headlessly
//!   followed by a reference resync.
//!
//! The session's *decisions* (what to merge, in which orientation) are the
//! pure `gantz_ca::sync` rules; everything here is bookkeeping around them.

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_gantz::head;
use gantz_ca as ca;
use gantz_collab::{Access, ConnState, Handle, Identity, ObjectRef, PeerId, Session, SessionId};
pub use session::{
    attach_session_refs, mark_dirty, on_join_session, on_leave_session, on_share_session,
    session_resolutions,
};
use std::collections::{BTreeMap, HashMap, HashSet};
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

/// The plugin: registers the session resources, observers and systems.
///
/// Requires `bevy_gantz::GantzPlugin` and `bevy_gantz_egui`'s plugin. The
/// app provides the [`CollabIdentity`] resource (loaded or generated at
/// startup); sharing and joining are requested via [`ShareSessionEvent`] /
/// [`JoinSessionEvent`] triggers.
#[derive(Default)]
pub struct CollabPlugin;

/// The user's collaborative identity, provided by the app at startup.
#[derive(Resource)]
pub struct CollabIdentity(pub Identity);

/// The network runtime handle, spawned lazily on first share/join.
#[derive(Default, Resource)]
pub struct CollabRuntime(pub Option<Handle>);

/// All local session state, keyed by session id.
#[derive(Default, Resource)]
pub struct CollabSessions {
    pub sessions: HashMap<SessionId, SessionState>,
    /// Set on any local commit; consumed by [`announce_sessions`].
    pub dirty: bool,
    /// The endpoint's home relay(s) and their connection state.
    pub relays: Vec<(String, bool)>,
}

/// One session's local (non-persisted) runtime state.
pub struct SessionState {
    /// The persisted configuration.
    pub session: Session,
    /// The invite string, once the runtime has minted it.
    pub ticket: Option<String>,
    /// The connection lifecycle, for the GUI indicator.
    pub conn: ConnState,
    /// Connected peers and their self-reported usernames.
    pub peers: BTreeMap<PeerId, Option<String>>,
    /// Echo suppression: the tip most recently announced (or adopted from
    /// the network) per scoped name.
    pub last_announced: HashMap<ca::Name, ca::CommitAddr>,
    /// The per-origin gossip sequence number.
    pub seq: u64,
    /// In-flight fetches, per scoped name.
    pub pending: HashMap<ca::Name, PendingTip>,
    /// Auto-resolved conflicts accumulated since the session started, for
    /// surfacing in the GUI.
    pub conflicts: usize,
    /// The most recent session error (e.g. a failed join), cleared once the
    /// session progresses.
    pub error: Option<String>,
    /// The empty-graph commit minted at join time so the session's tab
    /// opens immediately; the snapshot adopts over it.
    pub placeholder: Option<ca::CommitAddr>,
    /// Commits already mirrored into the runtime-owned served store, so
    /// `serve_scope`'s updates stay incremental without reading it back.
    pub served_commits: HashSet<ca::CommitAddr>,
    /// Graphs already mirrored into the served store.
    pub served_graphs: HashSet<ca::GraphAddr>,
    /// Blobs already mirrored into the served store.
    pub served_blobs: HashSet<(ca::SectionId, ca::ContentAddr)>,
    /// Section entries already mirrored into the served store. Entries are
    /// recorded only once actually sent, so metadata seeded *after* its
    /// subject (e.g. a commit's view baseline) is caught by a later pass.
    pub served_sections: HashSet<(ca::SectionId, ca::Key)>,
    /// The served `name -> tip` map as last mirrored.
    pub served_heads: HashMap<ca::Name, ca::CommitAddr>,
    /// Peers' live pointers over the session's shared graph, keyed by
    /// origin. Entries persist through `pos: None` so reordered stale
    /// updates still drop by `seq`; freshness is filtered at display time.
    pub pointers: HashMap<PeerId, PeerPointer>,
}

/// One peer's last-known pointer state (see `GossipMsg::Pointer`).
pub struct PeerPointer {
    /// Graph-space position; `None` = the pointer left the scene.
    pub pos: Option<(f32, f32)>,
    /// The origin's latest sequence number, for stale-drop.
    pub seq: u64,
    /// When the update arrived, for display-time expiry.
    pub at: web_time::Instant,
}

/// An announced tip whose closure is still being fetched.
pub struct PendingTip {
    pub tip: ca::CommitAddr,
    pub from: PeerId,
    pub staged: ca::sync::Staged,
    /// The previous want, to detect a peer that cannot make progress.
    pub last_want: Option<Vec<ObjectRef>>,
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

/// Request leaving (and forgetting) a session.
#[derive(Debug, Event)]
pub struct LeaveSessionEvent {
    pub session: SessionId,
}

impl SessionState {
    fn new(session: Session) -> Self {
        Self {
            session,
            ticket: None,
            conn: ConnState::default(),
            peers: BTreeMap::new(),
            last_announced: HashMap::new(),
            seq: 0,
            pending: HashMap::new(),
            conflicts: 0,
            error: None,
            placeholder: None,
            served_commits: HashSet::new(),
            served_graphs: HashSet::new(),
            served_blobs: HashSet::new(),
            served_sections: HashSet::new(),
            served_heads: HashMap::new(),
            pointers: HashMap::new(),
        }
    }

    /// The session's shared branch as a structured name.
    fn branch_name(&self) -> ca::Name {
        self.session.branch.parse().expect("names parse infallibly")
    }
}

impl Plugin for CollabPlugin {
    fn build(&self, app: &mut App) {
        use bevy_gantz_egui::RegisterResponseExt;
        // The Settings > Collab subtab: the tab's emitted `CollabConfig`
        // payloads dispatch into a buffered message, applied (and
        // re-snapshotted into the tab) by `sync_collab_settings`. The
        // `init_resource` is idempotent (`bevy_gantz_egui` owns the clear).
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
            // Capture overrides (last registration wins over the
            // bevy_gantz_egui defaults; the app adds this plugin after it).
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
