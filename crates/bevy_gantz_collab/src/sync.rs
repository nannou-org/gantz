//! The fetch and converge plane. It drains runtime events, runs the want and
//! fetch loop through `gantz_ca::sync::Staged` validation, converges each
//! name, and announces local tips into the runtime-owned served stores.

use crate::{CollabIdentity, CollabRuntime, CollabSessions, PendingTip, SessionState, action};
use bevy_ecs::prelude::*;
use bevy_gantz::head;
use bevy_gantz::reg::Registry;
use bevy_gantz_egui::{ForHead, SyncRemoteTip};
use bevy_log as log;
use gantz_ca as ca;
use gantz_collab::{
    Command, ConnState, Event as CollabEvent, GossipMsg, Handle, Object, ObjectRef, Objects,
    PeerId, SessionId, Want, proto,
};
use std::collections::{BTreeSet, HashSet};

/// The maximum tip entries per gossip `Tips` message, keeping each message
/// well under iroh-gossip's size limit.
const TIPS_PER_MSG: usize = 16;

/// The world access threaded through the fetch and converge call graph.
///
/// It holds the registry the fetched closures apply to, the open heads to
/// converge, the live graph views that supply the local camera when adopting
/// stored views, and commands for the follow-up triggers.
#[derive(bevy_ecs::system::SystemParam)]
pub(crate) struct SyncCtx<'w, 's> {
    registry: ResMut<'w, Registry>,
    open: Query<'w, 's, (Entity, &'static head::HeadRef), With<head::OpenHead>>,
    graph_views: Query<'w, 's, &'static bevy_gantz_egui::GraphView, With<head::OpenHead>>,
    cmds: Commands<'w, 's>,
}

/// Drain the runtime's events. Fetch, validate, apply and converge.
pub(crate) fn poll_collab_events(
    runtime: Res<CollabRuntime>,
    mut sessions: ResMut<CollabSessions>,
    mut inbox: ResMut<action::ActionInbox>,
    mut ctx: SyncCtx,
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
                apply_join_snapshot(handle, &mut sessions, &mut ctx, session, heads, objects);
            }
            CollabEvent::Gossip { session, from, msg } => match msg {
                GossipMsg::Tips { changed, .. } => {
                    for (name, tip, _graph) in changed {
                        start_fetch(handle, &mut sessions, &mut ctx, session, from, name, tip);
                    }
                }
                GossipMsg::Presence { origin, name } => {
                    if let Some(state) = sessions.sessions.get_mut(&session) {
                        state.peers.insert(origin, name);
                    }
                }
                // Gossip re-announcement covers transient losses, so
                // anti-entropy digests are ignored.
                GossipMsg::Digest { .. } => {}
                // Queue ephemeral actions for `action::apply_remote_actions`,
                // which runs after this system and before `VmSet`.
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
                // Keep the newest pointer per origin, scoped to the session's
                // shared branch. Gossip may reorder, so stale sequence
                // numbers drop.
                GossipMsg::Pointer {
                    origin,
                    seq,
                    name,
                    pos,
                } => {
                    if let Some(state) = sessions.sessions.get_mut(&session) {
                        if name == state.branch_name()
                            && state.pointers.get(&origin).is_none_or(|p| p.seq < seq)
                        {
                            let at = web_time::Instant::now();
                            state
                                .pointers
                                .insert(origin, crate::PeerPointer { pos, seq, at });
                        }
                    }
                }
            },
            CollabEvent::Objects {
                session, objects, ..
            } => {
                feed_objects(handle, &mut sessions, &mut ctx, session, objects);
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
                    state.pointers.remove(&peer);
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
/// [`Command::Update`]. One reachability walk from the scoped tips via
/// [`ca::closure_from`] surfaces every required commit, graph and
/// content-referenced blob, nested references included. In-scope metadata
/// section entries ride along, so peers place synced nodes where their
/// author put them.
///
/// The runtime owns the store, so `state`'s served-content shadows keep the
/// update incremental without reading it back. Inserts are content-addressed
/// and idempotent, so shadow loss only costs a re-send.
pub(crate) fn serve_scope(
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
    // In-scope metadata is the section entries keyed by scoped names or
    // content in the closure. The `heads` section travels as the head list.
    // Address-keyed entries have no scoping rule and stay local.
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

/// Apply a join snapshot. Validate it with grandfathered staging, reconcile
/// each name, fill the store and open the shared graph.
#[allow(clippy::too_many_arguments)]
fn apply_join_snapshot(
    handle: &Handle,
    sessions: &mut CollabSessions,
    ctx: &mut SyncCtx<'_, '_>,
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
    let applied = match staged.apply(&mut ctx.registry) {
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
    // Adopt the host's node layouts before opening the head, so the shared
    // graph opens with its nodes where the host placed them.
    let camera = local_camera(state, &ctx.open, &ctx.graph_views);
    apply_sections(&mut ctx.registry, sections, camera);

    let resolutions = state.session.resolutions;
    for (name, tip) in heads {
        match ctx.registry.head(&name) {
            None => {
                ctx.registry.set_head(name.clone(), tip);
                state.last_announced.insert(name, tip);
            }
            Some(local) if local == tip => {
                state.last_announced.insert(name, tip);
            }
            // Adopt over the placeholder minted at join time. The resolve
            // path recognises it and navigates the open head.
            Some(local) if state.placeholder == Some(local) => {
                if resolve_tip(state, ctx, &name, tip, resolutions) {
                    sessions.dirty = true;
                }
            }
            Some(local) => match ca::plan_sync_step(ctx.registry.commits(), local, tip) {
                ca::SyncStep::Unrelated => {
                    // The session owns the name. Rename the local graph aside
                    // rather than lose it or deadlock the join.
                    let aside: ca::Name = format!("{name}-local-{}", local.display_short())
                        .parse()
                        .expect("names parse infallibly");
                    log::warn!(
                        "join: local '{name}' is unrelated to the session's; \
                         renamed aside as '{aside}'"
                    );
                    ctx.registry.set_head(aside, local);
                    ctx.registry.set_head(name.clone(), tip);
                    state.last_announced.insert(name, tip);
                }
                _ => {
                    // Behind, ahead or diverged. Use the live convergence
                    // path.
                    if resolve_tip(state, ctx, &name, tip, resolutions) {
                        sessions.dirty = true;
                    }
                }
            },
        }
    }
    state.conn = ConnState::Live;
    state.error = None;

    // Serve the adopted closure onward and open the shared graph.
    let branch = state.branch_name();
    let scope = gantz_egui::sync::session_scope(&ctx.registry, &branch);
    serve_scope(handle, state, &ctx.registry, &scope);
    let branch_head = ca::Head::Branch(branch);
    let already_open = ctx.open.iter().any(|(_, hr)| hr.0 == branch_head);
    if !already_open {
        ctx.cmds.trigger(head::OpenEvent(branch_head));
    }
    ctx.cmds.trigger(bevy_gantz_egui::ResyncRefsEvent);
}

/// Apply received section entries to the local registry per their stamped
/// merge policy. This is advisory metadata. There are no addresses to
/// verify, and a decode failure upstream skips the entry.
///
/// When `local_camera` is given, adopted view entries have their camera
/// replaced with it. Peers' layouts are welcome, but adopting a view must
/// never move the local viewport to a peer's.
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

/// Begin or refresh fetching an announced tip's closure.
#[allow(clippy::too_many_arguments)]
fn start_fetch(
    handle: &Handle,
    sessions: &mut CollabSessions,
    ctx: &mut SyncCtx<'_, '_>,
    session: SessionId,
    from: PeerId,
    name: ca::Name,
    tip: ca::CommitAddr,
) {
    let Some(state) = sessions.sessions.get_mut(&session) else {
        return;
    };
    // Already known and contained. Drop silently.
    if ctx.registry.commits().contains_key(&tip) {
        if let Some(local) = ctx.registry.head(&name) {
            if ca::plan_sync_step(ctx.registry.commits(), local, tip) == ca::SyncStep::UpToDate {
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
    let want = compute_want(&ctx.registry, &mut pending);
    if want.is_empty() {
        let resolutions = state.session.resolutions;
        if resolve_tip(state, ctx, &name, tip, resolutions) {
            sessions.dirty = true;
        }
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
fn feed_objects(
    handle: &Handle,
    sessions: &mut CollabSessions,
    ctx: &mut SyncCtx<'_, '_>,
    session: SessionId,
    objects: Objects,
) {
    let Some(state) = sessions.sessions.get_mut(&session) else {
        return;
    };
    let resolutions = state.session.resolutions;
    // Decode graphs once and split by kind. Verification happens per staged
    // insert. Section entries apply directly, because advisory metadata
    // rides no closure.
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
    // be navigated to or merged. Merged-in nodes seed their positions from
    // the other tip's view.
    let camera = local_camera(state, &ctx.open, &ctx.graph_views);
    apply_sections(&mut ctx.registry, sections, camera);
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
        let want = compute_want(&ctx.registry, &mut pending);
        if want.is_empty() {
            let tip = pending.tip;
            match pending.staged.apply(&mut ctx.registry) {
                Ok(_) => {
                    if resolve_tip(state, ctx, &name, tip, resolutions) {
                        sessions.dirty = true;
                    }
                }
                Err(e) => log::warn!("fetch: closure for '{name}' failed to apply: {e}"),
            }
            continue;
        }
        // No progress means the peer cannot supply the closure. Drop it and
        // let a future announcement retry.
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

/// Everything still needed for a pending tip. That is its commit and graph
/// closure via [`ca::sync::Staged::missing`], plus the staged graphs'
/// outgoing references via [`ca::data_graph_out`] that neither the registry
/// nor the staging area holds yet. Those references are nested graphs and
/// content-referenced blobs.
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

/// Converge a scoped name with a fully applied remote tip.
///
/// A name open as a head goes through the [`SyncRemoteTip`] observer, which
/// migrates VM state, layout and selection and fires the committed
/// machinery. Background names move headlessly, followed by a reference
/// resync.
///
/// Returns whether a merge commit was minted. A minted merge is local
/// content peers have not seen, so the caller marks the sessions dirty.
fn resolve_tip(
    state: &mut SessionState,
    ctx: &mut SyncCtx<'_, '_>,
    name: &ca::Name,
    tip: ca::CommitAddr,
    resolutions: ca::merge::Resolutions,
) -> bool {
    let SyncCtx {
        registry,
        open,
        cmds,
        ..
    } = ctx;
    let Some(local) = registry.head(name) else {
        // A name born on the remote side. Adopt it.
        registry.set_head(name.clone(), tip);
        state.last_announced.insert(name.clone(), tip);
        cmds.trigger(bevy_gantz_egui::ResyncRefsEvent);
        return false;
    };
    // The join flow's placeholder is unrelated to the session content it
    // awaits by design. Adopt over it rather than surface `Unrelated`.
    let adopt_unrelated = state.placeholder == Some(local);
    let plan = ca::plan_sync_step(registry.commits(), local, tip);
    let open_entity = open
        .iter()
        .find(|(_, hr)| matches!(&hr.0, ca::Head::Branch(n) if n == name))
        .map(|(entity, _)| entity);
    match (open_entity, plan) {
        (_, ca::SyncStep::UpToDate) => false,
        (_, ca::SyncStep::Adopt(t)) if t == local => false,
        (open_entity, ca::SyncStep::Unrelated) => {
            if !adopt_unrelated {
                log::warn!("session: remote tip for '{name}' shares no local history; ignoring");
                return false;
            }
            state.placeholder = None;
            state.last_announced.insert(name.clone(), tip);
            match open_entity {
                // The observer navigates the open head onto the adopted tip,
                // which moves the name.
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
            false
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
            false
        }
        (None, ca::SyncStep::FastForward(t) | ca::SyncStep::Adopt(t)) => {
            registry.set_head(name.clone(), t);
            state.last_announced.insert(name.clone(), t);
            cmds.trigger(bevy_gantz_egui::ResyncRefsEvent);
            false
        }
        (None, ca::SyncStep::Merge { first, second }) => {
            match ca::merge_commits(registry, first, second, resolutions) {
                Ok(ca::MergeResolution::Diverged { outcome, .. }) => {
                    state.conflicts += outcome.conflicts.len();
                    let graph_ca = ca::graph_addr(&outcome.graph);
                    let mut branch_head = ca::Head::Branch(name.clone());
                    // Seed the minted merge commit's view from the parent
                    // tips' stored views before it can be announced. A
                    // viewless tip on the wire auto-layouts on every adopting
                    // peer. An open head seeds from its live layout instead,
                    // in `on_sync_remote_tip`.
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
                    cmds.trigger(bevy_gantz_egui::ResyncRefsEvent);
                    true
                }
                Ok(_) => false,
                Err(e) => {
                    log::warn!("session: headless merge of '{name}' failed: {e}");
                    false
                }
            }
        }
    }
}
