//! Bevy integration of gantz's peer-to-peer collaborative sessions (#286).
//!
//! [`CollabPlugin`] bridges the [`gantz_collab`] runtime into the app:
//!
//! - **Outbound**: any local commit marks the session state dirty;
//!   [`announce_sessions`] then mirrors the session's scoped closure into
//!   the served [`SessionRegistry`](gantz_collab::SessionRegistry) and
//!   broadcasts the changed tips. Fast-forwards and adoptions of *received*
//!   tips are never re-announced (echo suppression).
//! - **Inbound**: [`poll_collab_events`] drains the runtime, drives the
//!   want/fetch loop through `gantz_ca::sync::Staged` validation, applies
//!   completed closures to the registry and converges each scoped name -
//!   open heads via `bevy_gantz_egui`'s [`SyncRemoteTip`] observer (VM
//!   state/layout/selection migration included), background names headlessly
//!   followed by a reference resync.
//!
//! The session's *decisions* (what to merge, in which orientation) are the
//! pure `gantz_ca::sync` rules; everything here is bookkeeping around them.

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_gantz::head;
use bevy_gantz::reg::Registry;
use bevy_gantz_egui::{ForHead, SyncRemoteTip};
use bevy_log as log;
use gantz_ca as ca;
use gantz_collab::{
    Access, Command, ConnState, Event as CollabEvent, GossipMsg, Handle, Identity, Object,
    ObjectRef, Objects, PeerId, Role, Session, SessionEntry, SessionId, SessionRegistry, Want,
    proto,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub mod action;
pub mod storage;

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
    /// [`serve_scope`]'s updates stay incremental without reading it back.
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

/// The maximum tip entries per gossip `Tips` message, keeping each message
/// well under iroh-gossip's size limit.
const TIPS_PER_MSG: usize = 16;

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
            .add_observer(on_committed_mark_dirty)
            .add_observer(on_changed_mark_dirty)
            .add_observer(on_layout_committed_mark_dirty)
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
                        update_collab_ui,
                    )
                        .after(bevy_gantz::VmSet),
                ),
            );
    }
}

/// A pending configuration change emitted by the Settings > Collab subtab.
#[derive(Message)]
pub struct CollabSettingsChanged(pub gantz_egui::collab::CollabConfig);

/// Dispatch a [`CollabConfig`][gantz_egui::collab::CollabConfig] payload
/// emitted by the Settings > Collab subtab as a buffered
/// [`CollabSettingsChanged`] message (registered via
/// `RegisterResponseExt::register_response_with`).
fn dispatch_collab_settings(
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
fn sync_collab_settings(
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
fn dispatch_join_session(
    _entity: Option<Entity>,
    payload: gantz_egui::DynResponse,
    cmds: &mut Commands,
) {
    let gantz_egui::JoinSession { ticket } = bevy_gantz_egui::downcast_payload(payload);
    cmds.trigger(JoinSessionEvent { ticket });
}

/// Map the GUI's share payload to a [`ShareSessionEvent`].
fn on_share_head_payload(trigger: On<ForHead<gantz_egui::ShareHead>>, mut cmds: Commands) {
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
fn on_stop_sharing_payload(
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
/// join; the snapshot lands via [`poll_collab_events`].
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

/// Any local commit may move scoped tips: mark the sessions dirty so
/// [`announce_sessions`] re-checks them.
pub fn on_committed_mark_dirty(
    _trigger: On<head::CommittedEvent>,
    mut sessions: ResMut<CollabSessions>,
) {
    sessions.dirty = true;
}

/// Head navigation (e.g. history-pane moves) also moves branch tips.
pub fn on_changed_mark_dirty(
    _trigger: On<head::ChangedEvent>,
    mut sessions: ResMut<CollabSessions>,
) {
    sessions.dirty = true;
}

/// Settled node-moves commit layout-only changes without the committed
/// machinery; sessions still announce them so peers follow node positions.
pub fn on_layout_committed_mark_dirty(
    _trigger: On<bevy_gantz_egui::LayoutCommittedEvent>,
    mut sessions: ResMut<CollabSessions>,
) {
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

/// Drain the runtime's events: fetch, validate, apply and converge.
pub fn poll_collab_events(
    runtime: Res<CollabRuntime>,
    mut sessions: ResMut<CollabSessions>,
    mut registry: ResMut<Registry>,
    mut inbox: ResMut<action::ActionInbox>,
    open: Query<(Entity, &head::HeadRef), With<head::OpenHead>>,
    graph_views: Query<&bevy_gantz_egui::GraphView, With<head::OpenHead>>,
    mut cmds: Commands,
) {
    let Some(handle) = runtime.0.as_ref() else {
        return;
    };
    while let Ok(event) = handle.events.try_recv() {
        match event {
            CollabEvent::Ready { peer } => log::info!("collab endpoint ready: {peer}"),
            CollabEvent::TicketReady { session, ticket } => {
                if let Some(state) = sessions.sessions.get_mut(&session) {
                    state.ticket = Some(ticket);
                }
            }
            CollabEvent::Joined {
                session,
                heads,
                objects,
            } => {
                apply_join_snapshot(
                    handle,
                    &mut sessions,
                    &mut registry,
                    &open,
                    &graph_views,
                    &mut cmds,
                    session,
                    heads,
                    objects,
                );
            }
            CollabEvent::Gossip { session, from, msg } => match msg {
                GossipMsg::Tips { changed, .. } => {
                    for (name, tip, _graph) in changed {
                        start_fetch(
                            handle,
                            &mut sessions,
                            &mut registry,
                            &open,
                            &mut cmds,
                            session,
                            from,
                            name,
                            tip,
                        );
                    }
                }
                GossipMsg::Presence { origin, name } => {
                    if let Some(state) = sessions.sessions.get_mut(&session) {
                        state.peers.insert(origin, name);
                    }
                }
                // Anti-entropy digests: heads pulls are a follow-up; gossip
                // re-announcement covers transient losses meanwhile.
                GossipMsg::Digest { .. } => {}
                // Ephemeral actions: queue for [`action::apply_remote_actions`]
                // (which runs after this system, before `VmSet`).
                GossipMsg::Action {
                    origin,
                    seq,
                    timestamp,
                    name,
                    graph,
                    data,
                } => {
                    inbox.receive(action::InboundAction {
                        session,
                        origin,
                        seq,
                        timestamp,
                        name,
                        graph,
                        data,
                        received: web_time::Instant::now(),
                    });
                }
            },
            CollabEvent::Objects {
                session, objects, ..
            } => {
                feed_objects(
                    handle,
                    &mut sessions,
                    &mut registry,
                    &open,
                    &graph_views,
                    &mut cmds,
                    session,
                    objects,
                );
            }
            CollabEvent::PeerUp { session, peer } => {
                if let Some(state) = sessions.sessions.get_mut(&session) {
                    state.peers.entry(peer).or_insert(None);
                    state.conn = ConnState::Live;
                    state.error = None;
                }
            }
            CollabEvent::RelayStatus { relays } => {
                sessions.relays = relays;
            }
            CollabEvent::PeerDown { session, peer } => {
                if let Some(state) = sessions.sessions.get_mut(&session) {
                    state.peers.remove(&peer);
                    if state.peers.is_empty() {
                        state.conn = ConnState::Degraded;
                    }
                }
            }
            CollabEvent::Error { session, message } => {
                log::warn!("collab: {message}");
                if let Some(state) = session.and_then(|s| sessions.sessions.get_mut(&s)) {
                    if state.conn == ConnState::Connecting {
                        state.conn = ConnState::Degraded;
                    }
                    state.error = Some(message);
                }
            }
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
    if !sessions.dirty {
        return;
    }
    sessions.dirty = false;
    let (Some(handle), Some(identity)) = (runtime.0.as_ref(), identity) else {
        return;
    };
    let origin = identity.0.peer_id();
    for (id, state) in sessions.sessions.iter_mut() {
        let scope = gantz_egui::sync::session_scope(&registry, &state.branch_name());
        let mut changed = Vec::new();
        serve_scope(handle, state, &registry, &scope);
        for name in &scope {
            let Some(tip) = registry.head(name) else {
                continue;
            };
            if state.last_announced.get(name) == Some(&tip) {
                continue;
            }
            let Some(commit) = registry.commits().get(&tip) else {
                continue;
            };
            state.last_announced.insert(name.clone(), tip);
            changed.push((name.clone(), tip, commit.graph));
        }
        for chunk in changed.chunks(TIPS_PER_MSG) {
            state.seq += 1;
            let msg = GossipMsg::Tips {
                origin,
                seq: state.seq,
                changed: chunk.to_vec(),
            };
            let _ = handle
                .cmds
                .try_send(Command::Broadcast { session: *id, msg });
        }
    }
}

/// Mirror the scoped closure of `scope` into the session's served store via
/// [`Command::Update`]: one reachability walk from the scoped tips
/// ([`ca::closure_from`]) surfaces every required commit, graph (nested
/// references included) and content-referenced blob. In-scope metadata
/// section entries (e.g. commits' stored views, so peers place synced nodes
/// where their author put them) ride along.
///
/// The runtime owns the store, so `state`'s served-content shadows keep the
/// update incremental without reading it back. Inserts are content-addressed
/// and idempotent: shadow loss only ever costs a re-send.
fn serve_scope(
    handle: &Handle,
    state: &mut SessionState,
    registry: &ca::Registry,
    scope: &BTreeSet<ca::Name>,
) {
    let tips = scope.iter().filter_map(|n| registry.head(n));
    let live = ca::closure_from(registry, tips);
    let mut commits = Vec::new();
    let mut graphs = Vec::new();
    let mut blobs = Vec::new();
    for &addr in &live.commits {
        if state.served_commits.contains(&addr) {
            continue;
        }
        let Some(commit) = registry.commits().get(&addr) else {
            continue;
        };
        state.served_commits.insert(addr);
        commits.push((addr, commit.clone()));
    }
    for &ga in &live.graphs {
        if state.served_graphs.contains(&ga) {
            continue;
        }
        let Some(graph) = registry.graph(&ga) else {
            continue;
        };
        state.served_graphs.insert(ga);
        graphs.push((ga, graph.clone()));
    }
    for (section, addrs) in &live.blobs {
        let Some(store) = registry.blobs().get(section) else {
            continue;
        };
        for &addr in addrs {
            let key = (section.clone(), addr);
            if state.served_blobs.contains(&key) {
                continue;
            }
            let Some(bytes) = store.get(&addr) else {
                continue;
            };
            state.served_blobs.insert(key);
            blobs.push((section.clone(), store.liveness, addr, bytes.clone()));
        }
    }
    // In-scope metadata: section entries keyed by scoped names or content in
    // the closure. The `heads` section travels as the head list; arbitrary
    // address-keyed entries have no scoping rule and stay local.
    let mut sections = Vec::new();
    for (id, section) in registry.sections() {
        if id.as_str() == ca::HEADS_ID {
            continue;
        }
        for (key, value) in &section.entries {
            let in_scope = match key {
                ca::Key::Commit(ca) => live.commits.contains(ca),
                ca::Key::Name(name) => scope.contains(name),
                ca::Key::Graph(ga) => live.graphs.contains(ga),
                ca::Key::Addr(_) => false,
            };
            if !in_scope {
                continue;
            }
            let entry = (id.clone(), key.clone());
            if state.served_sections.contains(&entry) {
                continue;
            }
            state.served_sections.insert(entry);
            sections.push((
                id.clone(),
                section.policy,
                section.liveness,
                key.clone(),
                value.clone(),
            ));
        }
    }
    let mut heads = Vec::new();
    for name in scope {
        let Some(tip) = registry.head(name) else {
            continue;
        };
        if state.served_heads.insert(name.clone(), tip) != Some(tip) {
            heads.push((name.clone(), tip));
        }
    }
    if heads.is_empty()
        && commits.is_empty()
        && graphs.is_empty()
        && blobs.is_empty()
        && sections.is_empty()
    {
        return;
    }
    let _ = handle.cmds.try_send(Command::Update {
        session: state.session.id,
        heads,
        commits,
        graphs,
        sections,
        blobs,
    });
}

/// Apply a join snapshot: staged (grandfathered) validation, per-name
/// reconciliation, store fill, and opening the shared graph.
#[allow(clippy::too_many_arguments)]
fn apply_join_snapshot(
    handle: &Handle,
    sessions: &mut CollabSessions,
    registry: &mut Registry,
    open: &Query<(Entity, &head::HeadRef), With<head::OpenHead>>,
    graph_views: &Query<&bevy_gantz_egui::GraphView, With<head::OpenHead>>,
    cmds: &mut Commands,
    session: SessionId,
    heads: Vec<(ca::Name, ca::CommitAddr)>,
    objects: Objects,
) {
    let Some(state) = sessions.sessions.get_mut(&session) else {
        return;
    };
    let mut staged = ca::sync::Staged::new();
    let mut sections = Vec::new();
    for object in objects.objects {
        match object {
            Object::Commit(addr, wire) => {
                staged.insert_commit_grandfathered(addr, wire.into());
            }
            Object::Graph(addr, blob) => {
                let graph = match proto::decode_graph(&blob) {
                    Ok(graph) => graph,
                    Err(e) => {
                        log::error!("join: undecodable graph in snapshot: {e}");
                        return;
                    }
                };
                if let Err(e) = staged.insert_graph(addr, graph) {
                    log::error!("join: snapshot graph failed verification: {e}");
                    return;
                }
            }
            Object::Blob {
                section,
                liveness,
                addr,
                bytes,
            } => {
                if let Err(e) = staged.insert_blob(section, liveness, addr, bytes) {
                    log::error!("join: snapshot blob failed verification: {e}");
                    return;
                }
            }
            Object::Section {
                id,
                policy,
                liveness,
                key,
                value,
            } => match proto::decode_value(&value) {
                Ok(value) => sections.push((id, policy, liveness, key, value)),
                Err(e) => log::warn!("join: undecodable section entry in snapshot: {e}"),
            },
        }
    }
    let applied = match staged.apply(registry) {
        Ok(applied) => applied,
        Err(e) => {
            log::error!("join: snapshot failed to apply: {e}");
            return;
        }
    };
    log::info!(
        "joined session {session}: {} commits, {} graphs, {} blobs ({} truncated)",
        applied.commits.len(),
        applied.graphs.len(),
        applied.blobs.len(),
        applied.truncated,
    );
    // Adopt the host's node layouts (before opening the head, so the shared
    // graph opens with its nodes where the host placed them).
    apply_sections(registry, sections, local_camera(state, open, graph_views));

    // Reconcile each snapshot head with any local state.
    let resolutions = state.session.resolutions;
    for (name, tip) in heads {
        match registry.head(&name) {
            None => {
                registry.set_head(name.clone(), tip);
                state.last_announced.insert(name, tip);
            }
            Some(local) if local == tip => {
                state.last_announced.insert(name, tip);
            }
            // The placeholder minted at join time: adopt over it (the
            // resolve path recognises it and navigates the open head).
            Some(local) if state.placeholder == Some(local) => {
                resolve_tip(state, registry, open, cmds, &name, tip, resolutions);
            }
            Some(local) => match ca::plan_sync_step(registry.commits(), local, tip) {
                ca::SyncStep::Unrelated => {
                    // The session owns the name: rename the local graph
                    // aside rather than losing it or deadlocking the join.
                    let aside: ca::Name = format!("{name}-local-{}", local.display_short())
                        .parse()
                        .expect("names parse infallibly");
                    log::warn!(
                        "join: local '{name}' is unrelated to the session's; \
                         renamed aside as '{aside}'"
                    );
                    registry.set_head(aside, local);
                    registry.set_head(name.clone(), tip);
                    state.last_announced.insert(name, tip);
                }
                _ => {
                    // Behind/ahead/diverged: the live convergence path.
                    resolve_tip(state, registry, open, cmds, &name, tip, resolutions);
                }
            },
        }
    }
    state.conn = ConnState::Live;
    state.error = None;

    // Serve the adopted closure onward and open the shared graph.
    let branch = state.branch_name();
    let scope = gantz_egui::sync::session_scope(registry, &branch);
    serve_scope(handle, state, registry, &scope);
    let branch_head = ca::Head::Branch(branch);
    let already_open = open.iter().any(|(_, hr)| hr.0 == branch_head);
    if !already_open {
        cmds.trigger(head::OpenEvent(branch_head));
    }
    cmds.trigger(bevy_gantz_egui::ResyncRefsEvent);
}

/// Apply received section entries to the local registry per their stamped
/// merge policy. Advisory metadata: no addresses to verify, and a decode
/// failure upstream simply skips the entry.
///
/// Adopted view entries have their camera replaced with `local_camera` (the
/// session head's live camera) when given: peers' layouts are welcome, but
/// adopting a view must never yank the local viewport to a peer's.
fn apply_sections(
    registry: &mut Registry,
    sections: Vec<(
        ca::SectionId,
        ca::MergePolicy,
        ca::Liveness,
        ca::Key,
        ca::Value,
    )>,
    local_camera: Option<gantz_egui::Camera>,
) {
    use gantz_ca::SectionDecl;
    for (id, policy, liveness, key, mut value) in sections {
        let keep_existing = matches!(policy, ca::MergePolicy::KeepExisting);
        if keep_existing && registry.section_entry(&id, &key).is_some() {
            continue;
        }
        if id == gantz_egui::section::VIEWS_ID {
            if let (Some(camera), Some(mut view)) =
                (local_camera, gantz_egui::section::Views::decode(&value))
            {
                view.camera = camera;
                match gantz_egui::section::Views::encode(&view) {
                    Ok(encoded) => value = encoded,
                    Err(e) => {
                        log::warn!("failed to re-encode an adopted view: {e}");
                        continue;
                    }
                }
            }
        }
        registry.set_section_value(id, policy, liveness, key, value);
    }
}

/// The live camera of the session's open branch head, if any.
fn local_camera(
    state: &SessionState,
    open: &Query<(Entity, &head::HeadRef), With<head::OpenHead>>,
    graph_views: &Query<&bevy_gantz_egui::GraphView, With<head::OpenHead>>,
) -> Option<gantz_egui::Camera> {
    let branch_head = ca::Head::Branch(state.branch_name());
    open.iter()
        .find(|(_, hr)| hr.0 == branch_head)
        .and_then(|(entity, _)| graph_views.get(entity).ok())
        .map(|gv| gv.0.camera)
}

/// Begin (or refresh) fetching an announced tip's closure.
#[allow(clippy::too_many_arguments)]
fn start_fetch(
    handle: &Handle,
    sessions: &mut CollabSessions,
    registry: &mut Registry,
    open: &Query<(Entity, &head::HeadRef), With<head::OpenHead>>,
    cmds: &mut Commands,
    session: SessionId,
    from: PeerId,
    name: ca::Name,
    tip: ca::CommitAddr,
) {
    let Some(state) = sessions.sessions.get_mut(&session) else {
        return;
    };
    // Already known and contained: drop silently.
    if registry.commits().contains_key(&tip) {
        if let Some(local) = registry.head(&name) {
            if ca::plan_sync_step(registry.commits(), local, tip) == ca::SyncStep::UpToDate {
                return;
            }
        }
    }
    if state.pending.get(&name).is_some_and(|p| p.tip == tip) {
        return;
    }
    let mut pending = PendingTip {
        tip,
        from,
        staged: ca::sync::Staged::new(),
        last_want: None,
    };
    let want = compute_want(registry, &mut pending);
    if want.is_empty() {
        let resolutions = state.session.resolutions;
        resolve_tip(state, registry, open, cmds, &name, tip, resolutions);
        return;
    }
    pending.last_want = Some(want.refs.clone());
    state.pending.insert(name, pending);
    let _ = handle.cmds.try_send(Command::Fetch {
        session,
        from,
        want,
    });
}

/// Feed fetched objects into every pending tip of the session, applying and
/// converging those whose closure completed.
#[allow(clippy::too_many_arguments)]
fn feed_objects(
    handle: &Handle,
    sessions: &mut CollabSessions,
    registry: &mut Registry,
    open: &Query<(Entity, &head::HeadRef), With<head::OpenHead>>,
    graph_views: &Query<&bevy_gantz_egui::GraphView, With<head::OpenHead>>,
    cmds: &mut Commands,
    session: SessionId,
    objects: Objects,
) {
    let Some(state) = sessions.sessions.get_mut(&session) else {
        return;
    };
    let resolutions = state.session.resolutions;
    // Decode graphs once and split by kind; verification happens per staged
    // insert. Section entries (answered commits' piggybacked views) apply
    // directly: advisory metadata rides no closure.
    let mut commits: Vec<(ca::CommitAddr, ca::Commit)> = Vec::new();
    let mut graphs: Vec<(ca::GraphAddr, ca::DataGraph)> = Vec::new();
    let mut blobs: Vec<(ca::SectionId, ca::BlobLiveness, ca::ContentAddr, ca::Bytes)> = Vec::new();
    let mut sections = Vec::new();
    for object in objects.objects {
        match object {
            Object::Commit(addr, wire) => commits.push((addr, wire.into())),
            Object::Graph(addr, blob) => match proto::decode_graph(&blob) {
                Ok(graph) => graphs.push((addr, graph)),
                Err(e) => log::warn!("fetch: undecodable graph {addr}: {e}"),
            },
            Object::Blob {
                section,
                liveness,
                addr,
                bytes,
            } => blobs.push((section, liveness, addr, ca::Bytes::from(bytes))),
            Object::Section {
                id,
                policy,
                liveness,
                key,
                value,
            } => match proto::decode_value(&value) {
                Ok(value) => sections.push((id, policy, liveness, key, value)),
                Err(e) => log::warn!("fetch: undecodable section entry: {e}"),
            },
        }
    }
    // Adopt the peer's layouts for incoming commits before any of them can
    // be navigated to or merged (merged-in nodes seed their positions from
    // the other tip's view).
    apply_sections(registry, sections, local_camera(state, open, graph_views));
    let names: Vec<ca::Name> = state.pending.keys().cloned().collect();
    for name in names {
        let Some(mut pending) = state.pending.remove(&name) else {
            continue;
        };
        let mut poisoned = false;
        for (addr, commit) in &commits {
            if let Err(e) = pending.staged.insert_commit(*addr, commit.clone()) {
                log::warn!("fetch: rejected commit for '{name}': {e}");
                poisoned = true;
                break;
            }
        }
        if !poisoned {
            for (addr, graph) in &graphs {
                if let Err(e) = pending.staged.insert_graph(*addr, graph.clone()) {
                    log::warn!("fetch: rejected graph for '{name}': {e}");
                    poisoned = true;
                    break;
                }
            }
        }
        if !poisoned {
            for (section, liveness, addr, bytes) in &blobs {
                let staged = &mut pending.staged;
                if let Err(e) = staged.insert_blob(section.clone(), *liveness, *addr, bytes.clone())
                {
                    log::warn!("fetch: rejected blob for '{name}': {e}");
                    poisoned = true;
                    break;
                }
            }
        }
        if poisoned {
            continue;
        }
        let want = compute_want(registry, &mut pending);
        if want.is_empty() {
            let tip = pending.tip;
            match pending.staged.apply(registry) {
                Ok(_) => {
                    resolve_tip(state, registry, open, cmds, &name, tip, resolutions);
                }
                Err(e) => log::warn!("fetch: closure for '{name}' failed to apply: {e}"),
            }
            continue;
        }
        // No progress means the peer cannot supply the closure: drop and let
        // a future announcement retry.
        if pending.last_want.as_ref() == Some(&want.refs) {
            log::warn!("fetch: no progress on '{name}'; dropping");
            continue;
        }
        pending.last_want = Some(want.refs.clone());
        let from = pending.from;
        state.pending.insert(name, pending);
        let _ = handle.cmds.try_send(Command::Fetch {
            session,
            from,
            want,
        });
    }
}

/// Everything still needed for a pending tip: its commit/graph closure via
/// [`ca::sync::Staged::missing`], plus the staged graphs' outgoing references
/// ([`ca::data_graph_out`]) - nested graphs and content-referenced blobs -
/// that neither the registry nor the staging area holds yet.
fn compute_want(registry: &ca::Registry, pending: &mut PendingTip) -> Want {
    let missing = pending.staged.missing(registry, pending.tip);
    let mut refs: Vec<ObjectRef> = missing.commits.into_iter().map(ObjectRef::Commit).collect();
    let mut graph_wants: Vec<ca::GraphAddr> = missing.graphs;
    let mut blob_wants: Vec<(ca::SectionId, ca::ContentAddr)> = Vec::new();
    let staged_graphs: HashSet<ca::GraphAddr> =
        pending.staged.graphs().map(|(ga, _)| *ga).collect();
    let staged_blobs: HashSet<(ca::SectionId, ca::ContentAddr)> =
        pending.staged.blobs().cloned().collect();
    for (_, graph) in pending.staged.graphs() {
        let out = ca::data_graph_out(graph);
        for ga in out.graphs {
            if registry.graph(&ga).is_none()
                && !staged_graphs.contains(&ga)
                && !graph_wants.contains(&ga)
            {
                graph_wants.push(ga);
            }
        }
        for (section, addr) in out.blobs {
            let key = (section, addr);
            if registry.blob(&key.0, &addr).is_none()
                && !staged_blobs.contains(&key)
                && !blob_wants.contains(&key)
            {
                blob_wants.push(key);
            }
        }
    }
    refs.extend(graph_wants.into_iter().map(ObjectRef::Graph));
    refs.extend(
        blob_wants
            .into_iter()
            .map(|(section, addr)| ObjectRef::Blob { section, addr }),
    );
    Want { refs }
}

/// Converge a scoped name with a (now fully applied) remote tip.
///
/// A name open as a head goes through the [`SyncRemoteTip`] observer, which
/// migrates VM state/layout/selection and fires the committed machinery;
/// background names move headlessly, followed by a reference resync.
fn resolve_tip(
    state: &mut SessionState,
    registry: &mut Registry,
    open: &Query<(Entity, &head::HeadRef), With<head::OpenHead>>,
    cmds: &mut Commands,
    name: &ca::Name,
    tip: ca::CommitAddr,
    resolutions: ca::merge::Resolutions,
) {
    let Some(local) = registry.head(name) else {
        // A name born on the remote side: adopt it.
        registry.set_head(name.clone(), tip);
        state.last_announced.insert(name.clone(), tip);
        cmds.trigger(bevy_gantz_egui::ResyncRefsEvent);
        return;
    };
    // The join flow's placeholder (an empty graph minted so the session's
    // tab opens immediately) is deliberately unrelated to the session
    // content it awaits: adopt over it rather than surfacing `Unrelated`.
    let adopt_unrelated = state.placeholder == Some(local);
    let plan = ca::plan_sync_step(registry.commits(), local, tip);
    let open_entity = open
        .iter()
        .find(|(_, hr)| matches!(&hr.0, ca::Head::Branch(n) if n == name))
        .map(|(entity, _)| entity);
    match (open_entity, plan) {
        (_, ca::SyncStep::UpToDate) => (),
        (_, ca::SyncStep::Adopt(t)) if t == local => (),
        (open_entity, ca::SyncStep::Unrelated) => {
            if !adopt_unrelated {
                log::warn!("session: remote tip for '{name}' shares no local history; ignoring");
                return;
            }
            state.placeholder = None;
            state.last_announced.insert(name.clone(), tip);
            match open_entity {
                // The observer navigates the open head onto the adopted tip
                // (which moves the name).
                Some(entity) => {
                    cmds.trigger(ForHead {
                        head: entity,
                        data: SyncRemoteTip {
                            remote: tip,
                            resolutions,
                            adopt_unrelated: true,
                        },
                    });
                }
                None => {
                    registry.set_head(name.clone(), tip);
                    cmds.trigger(bevy_gantz_egui::ResyncRefsEvent);
                }
            }
        }
        (Some(entity), plan) => {
            // Adoptions of received tips are not re-announced.
            if let ca::SyncStep::FastForward(t) | ca::SyncStep::Adopt(t) = plan {
                state.last_announced.insert(name.clone(), t);
            }
            cmds.trigger(ForHead {
                head: entity,
                data: SyncRemoteTip {
                    remote: tip,
                    resolutions,
                    adopt_unrelated: false,
                },
            });
        }
        (None, ca::SyncStep::FastForward(t) | ca::SyncStep::Adopt(t)) => {
            registry.set_head(name.clone(), t);
            state.last_announced.insert(name.clone(), t);
            cmds.trigger(bevy_gantz_egui::ResyncRefsEvent);
        }
        (None, ca::SyncStep::Merge { first, second }) => {
            match ca::merge_commits(registry, first, second, resolutions) {
                Ok(ca::MergeResolution::Diverged { outcome, .. }) => {
                    state.conflicts += outcome.conflicts.len();
                    let graph_ca = ca::graph_addr(&outcome.graph);
                    let mut branch_head = ca::Head::Branch(name.clone());
                    // Seed the minted merge commit's view from the parent
                    // tips' stored views before it can be announced: a
                    // viewless tip on the wire auto-layouts on every
                    // adopting peer. (An open head seeds from its live
                    // layout instead - see `on_sync_remote_tip`.)
                    let first_view = gantz_egui::section::view(registry, &first);
                    let second_view = gantz_egui::section::view(registry, &second);
                    let seeded = gantz_egui::ops::merged_view(
                        &outcome.node_srcs,
                        first_view.as_ref(),
                        second_view.as_ref(),
                    );
                    let graph = outcome.graph;
                    let minted = registry.commit_merge_canonical(
                        first,
                        second,
                        graph_ca,
                        || graph,
                        &mut branch_head,
                    );
                    bevy_gantz_egui::seed_view(registry, minted, seeded);
                    // A minted merge must be announced.
                    cmds.trigger(bevy_gantz_egui::ResyncRefsEvent);
                }
                Ok(_) => (),
                Err(e) => log::warn!("session: headless merge of '{name}' failed: {e}"),
            }
        }
    }
}
