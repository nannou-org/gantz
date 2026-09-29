//! The fetch and converge plane. It drains runtime events, runs the want and
//! fetch loop through `gantz_ca::sync::Staged` validation and converges each
//! name. What the host must do in response comes back as [`Effect`]s.

use crate::{PeerPointer, PendingTip, SessionState, Sessions, vault};
use gantz_ca as ca;
use gantz_collab::{
    Command, ConnState, Event, GossipMsg, Handle, Object, ObjectRef, Objects, PeerId, SessionId,
    Want, proto,
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Branch names the host has open as heads, with their live camera when the
/// host has one. A headless host passes an empty map.
pub type OpenHeads = BTreeMap<ca::Name, Option<gantz_egui::Camera>>;

/// What the host must do after a [`poll`], replayed in order.
#[derive(Debug)]
pub enum Effect {
    /// Open the session's shared branch.
    Open(ca::Name),
    /// A name the host has open must converge live on the remote tip. The
    /// host migrates state, layout and selection, then fires its committed
    /// machinery.
    RemoteTip {
        name: ca::Name,
        remote: ca::CommitAddr,
        resolutions: ca::merge::Resolutions,
        adopt_unrelated: bool,
    },
    /// Names moved headlessly. Referrers must resync and open heads refresh.
    ResyncRefs,
    /// A name moved headlessly, by adoption, fast-forward or a minted merge.
    Moved {
        session: SessionId,
        name: ca::Name,
        from: Option<ca::CommitAddr>,
        to: ca::CommitAddr,
    },
    /// The join snapshot applied.
    Joined {
        session: SessionId,
        commits: usize,
        graphs: usize,
        blobs: usize,
    },
    PeerUp {
        session: SessionId,
        peer: PeerId,
    },
    PeerDown {
        session: SessionId,
        peer: PeerId,
    },
    Error {
        session: Option<SessionId>,
        message: String,
    },
    /// An ephemeral action over the session's shared graph, for hosts with a
    /// live VM.
    Action {
        session: SessionId,
        origin: PeerId,
        seq: u64,
        timestamp: u64,
        name: ca::Name,
        graph: ca::GraphAddr,
        data: Vec<u8>,
    },
    /// The vault moved a name the host has open. Move the open head to `to`
    /// exactly, or detach it when `None` removed the name. The local head
    /// holds nothing `to` lacks.
    Reset {
        name: ca::Name,
        to: Option<ca::CommitAddr>,
    },
    /// A local graph moved aside to make way for the vault's graph of the
    /// same name. Open heads on `from` follow to `to`.
    Renamed {
        from: ca::Name,
        to: ca::Name,
    },
}

/// The context threaded through the fetch and converge call graph.
struct Cx<'a> {
    registry: &'a mut ca::Registry,
    handle: &'a Handle,
    open: &'a OpenHeads,
    effects: &'a mut Vec<Effect>,
    /// Set when a merge commit is minted, so the host announces it.
    dirty: &'a mut bool,
}

/// Drain the runtime's events. Fetch, validate, apply and converge. Then
/// sync every name out of step with the vault, if linked.
pub fn poll(
    sessions: &mut Sessions,
    registry: &mut ca::Registry,
    handle: &Handle,
    open: &OpenHeads,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    while let Ok(event) = handle.events.try_recv() {
        handle_event(sessions, registry, handle, open, event, &mut effects);
    }
    if let Some(link) = &mut sessions.vault {
        vault::sync(link, registry, handle, open, &mut effects);
    }
    effects
}

/// Handle one runtime event, appending the host's follow-ups to `effects`.
pub fn handle_event(
    sessions: &mut Sessions,
    registry: &mut ca::Registry,
    handle: &Handle,
    open: &OpenHeads,
    event: Event,
    effects: &mut Vec<Effect>,
) {
    if let Some(link) = sessions.vault.as_mut().filter(|l| vault::owns(l, &event)) {
        vault::handle_event(link, registry, handle, open, event);
        return;
    }
    let Sessions {
        sessions,
        dirty,
        relays,
        ..
    } = sessions;
    let mut cx = Cx {
        registry,
        handle,
        open,
        effects,
        dirty,
    };
    match event {
        Event::Ready { peer } => log::info!("collab endpoint ready: {peer}"),
        Event::TicketReady { session, ticket } => {
            if let Some(state) = sessions.get_mut(&session) {
                state.ticket = Some(ticket);
            }
        }
        Event::Joined {
            session,
            heads,
            objects,
        } => {
            if let Some(state) = sessions.get_mut(&session) {
                apply_join_snapshot(&mut cx, state, session, heads, objects);
            }
        }
        Event::Gossip { session, from, msg } => match msg {
            GossipMsg::Tips { changed, .. } => {
                if let Some(state) = sessions.get_mut(&session) {
                    for (name, tip, _graph) in changed {
                        start_fetch(&mut cx, state, session, from, name, tip);
                    }
                }
            }
            GossipMsg::Presence { origin, name } => {
                if let Some(state) = sessions.get_mut(&session) {
                    state.peers.insert(origin, name);
                }
            }
            // Gossip re-announcement covers transient losses, so
            // anti-entropy digests are ignored.
            GossipMsg::Digest { .. } => {}
            GossipMsg::Action {
                origin,
                seq,
                timestamp,
                name,
                graph,
                data,
            } => cx.effects.push(Effect::Action {
                session,
                origin,
                seq,
                timestamp,
                name,
                graph,
                data,
            }),
            // Keep the newest pointer per origin, scoped to the session's
            // shared branch. Gossip may reorder, so stale sequence numbers
            // drop.
            GossipMsg::Pointer {
                origin,
                seq,
                name,
                pos,
            } => {
                if let Some(state) = sessions.get_mut(&session) {
                    if name == state.branch_name()
                        && state.pointers.get(&origin).is_none_or(|p| p.seq < seq)
                    {
                        let at = web_time::Instant::now();
                        state.pointers.insert(origin, PeerPointer { pos, seq, at });
                    }
                }
            }
        },
        Event::Objects {
            session,
            want,
            objects,
            ..
        } => {
            if let Some(state) = sessions.get_mut(&session) {
                feed_objects(&mut cx, state, session, want, objects);
            }
        }
        Event::PeerUp { session, peer } => {
            if let Some(state) = sessions.get_mut(&session) {
                state.peers.entry(peer).or_insert(None);
                state.conn = ConnState::Live;
                state.error = None;
            }
            cx.effects.push(Effect::PeerUp { session, peer });
        }
        Event::RelayStatus { relays: current } => {
            *relays = current;
        }
        Event::PeerDown { session, peer } => {
            if let Some(state) = sessions.get_mut(&session) {
                state.peers.remove(&peer);
                state.pointers.remove(&peer);
                if state.peers.is_empty() {
                    state.conn = ConnState::Degraded;
                }
            }
            cx.effects.push(Effect::PeerDown { session, peer });
        }
        Event::FetchFailed {
            session,
            want,
            error,
            ..
        } => {
            // Nothing will answer this want, so free its names for the next
            // announcement.
            if let Some(state) = sessions.get_mut(&session) {
                state
                    .pending
                    .retain(|_, p| p.last_want.as_ref() != Some(&want.refs));
            }
            error_event(sessions, &mut cx, Some(session), error);
        }
        Event::Error { session, message } => error_event(sessions, &mut cx, session, message),
        // The vault host handles these itself, and a vault link not held
        // by these sessions has nothing to update.
        Event::VaultTicketReady { .. }
        | Event::DeviceSeen { .. }
        | Event::PushRequest { .. }
        | Event::LinkUp { .. }
        | Event::LinkChanged { .. }
        | Event::LinkDown { .. }
        | Event::Pushed { .. } => (),
    }
}

/// Surface a runtime error on its session, if any.
fn error_event(
    sessions: &mut HashMap<SessionId, SessionState>,
    cx: &mut Cx<'_>,
    session: Option<SessionId>,
    message: String,
) {
    log::warn!("collab: {message}");
    if let Some(state) = session.and_then(|s| sessions.get_mut(&s)) {
        if state.conn == ConnState::Connecting {
            state.conn = ConnState::Degraded;
        }
        state.error = Some(message.clone());
    }
    cx.effects.push(Effect::Error { session, message });
}

/// Apply a join snapshot. Validate it with grandfathered staging, reconcile
/// each name, fill the store and open the shared graph.
fn apply_join_snapshot(
    cx: &mut Cx<'_>,
    state: &mut SessionState,
    session: SessionId,
    heads: Vec<(ca::Name, ca::CommitAddr)>,
    objects: Objects,
) {
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
    let applied = match staged.apply(cx.registry) {
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
    let camera = local_camera(cx, state);
    apply_sections(cx.registry, sections, camera);

    let resolutions = state.session.resolutions;
    for (name, tip) in heads {
        match cx.registry.head(&name) {
            None => {
                cx.registry.set_head(name.clone(), tip);
                state.last_announced.insert(name.clone(), tip);
                cx.effects.push(Effect::Moved {
                    session,
                    name,
                    from: None,
                    to: tip,
                });
            }
            Some(local) if local == tip => {
                state.last_announced.insert(name, tip);
            }
            // Adopt over the placeholder minted at join time. The resolve
            // path recognises it and navigates the open head.
            Some(local) if state.placeholder == Some(local) => {
                resolve_tip(cx, state, session, &name, tip, resolutions);
            }
            Some(local) => match ca::plan_sync_step(cx.registry.commits(), local, tip) {
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
                    cx.registry.set_head(aside, local);
                    cx.registry.set_head(name.clone(), tip);
                    state.last_announced.insert(name.clone(), tip);
                    cx.effects.push(Effect::Moved {
                        session,
                        name,
                        from: Some(local),
                        to: tip,
                    });
                }
                _ => {
                    // Behind, ahead or diverged. Use the live convergence
                    // path.
                    resolve_tip(cx, state, session, &name, tip, resolutions);
                }
            },
        }
    }
    state.conn = ConnState::Live;
    state.error = None;

    // Serve the adopted closure onward and open the shared graph.
    let branch = state.branch_name();
    let scope = gantz_egui::sync::session_scope(cx.registry, &branch);
    crate::serve_scope(cx.handle, state, cx.registry, &scope);
    if !cx.open.contains_key(&branch) {
        cx.effects.push(Effect::Open(branch));
    }
    cx.effects.push(Effect::ResyncRefs);
    cx.effects.push(Effect::Joined {
        session,
        commits: applied.commits.len(),
        graphs: applied.graphs.len(),
        blobs: applied.blobs.len(),
    });
}

/// Apply received section entries to the local registry per their stamped
/// merge policy. This is advisory metadata. There are no addresses to
/// verify, and a decode failure upstream skips the entry.
///
/// When `local_camera` is given, adopted view entries have their camera
/// replaced with it. Peers' layouts are welcome, but adopting a view must
/// never move the local viewport to a peer's.
pub(crate) fn apply_sections(
    registry: &mut ca::Registry,
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
fn local_camera(cx: &Cx<'_>, state: &SessionState) -> Option<gantz_egui::Camera> {
    cx.open.get(&state.branch_name()).copied().flatten()
}

/// Begin or refresh fetching an announced tip's closure.
fn start_fetch(
    cx: &mut Cx<'_>,
    state: &mut SessionState,
    session: SessionId,
    from: PeerId,
    name: ca::Name,
    tip: ca::CommitAddr,
) {
    // Already known and contained. Drop silently.
    if cx.registry.commits().contains_key(&tip) {
        if let Some(local) = cx.registry.head(&name) {
            if ca::plan_sync_step(cx.registry.commits(), local, tip) == ca::SyncStep::UpToDate {
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
    let want = compute_want(cx.registry, &pending.staged, pending.tip);
    if want.is_empty() {
        let resolutions = state.session.resolutions;
        resolve_tip(cx, state, session, &name, tip, resolutions);
        return;
    }
    pending.last_want = Some(want.refs.clone());
    state.pending.insert(name, pending);
    let _ = cx.handle.cmds.try_send(Command::Fetch {
        session,
        from,
        want,
    });
}

/// Feed the objects fetched for `want` into the pending tips whose fetch it
/// answers, applying and converging those whose closure completed.
///
/// Only those tips see the response. A staged set is applied whole, so an
/// object staged for another name's closure would make it incomplete. Each
/// name fetches its own closure. A response for a fetch no longer pending
/// only contributes its section entries.
fn feed_objects(
    cx: &mut Cx<'_>,
    state: &mut SessionState,
    session: SessionId,
    want: Want,
    objects: Objects,
) {
    let resolutions = state.session.resolutions;
    let mut decoded = decode(objects);
    // Adopt the peer's layouts for incoming commits before any of them can
    // be navigated to or merged. Merged-in nodes seed their positions from
    // the other tip's view.
    let camera = local_camera(cx, state);
    apply_sections(cx.registry, std::mem::take(&mut decoded.sections), camera);
    let names: Vec<ca::Name> = state
        .pending
        .iter()
        .filter(|(_, p)| p.last_want.as_ref() == Some(&want.refs))
        .map(|(name, _)| name.clone())
        .collect();
    for name in names {
        let Some(mut pending) = state.pending.remove(&name) else {
            continue;
        };
        let next = match advance(cx.registry, &mut pending.staged, pending.tip, &decoded) {
            Ok(next) => next,
            Err(e) => {
                log::warn!("fetch: content for '{name}' {e}");
                continue;
            }
        };
        if next.is_empty() {
            resolve_tip(cx, state, session, &name, pending.tip, resolutions);
            continue;
        }
        // No progress means the peer cannot supply the closure. Drop it and
        // let a future announcement retry.
        if pending.last_want.as_ref() == Some(&next.refs) {
            log::warn!("fetch: no progress on '{name}'; dropping");
            continue;
        }
        pending.last_want = Some(next.refs.clone());
        let from = pending.from;
        state.pending.insert(name, pending);
        let _ = cx.handle.cmds.try_send(Command::Fetch {
            session,
            from,
            want: next,
        });
    }
}

/// Fetched objects decoded and split by kind. Graph and blob verification
/// happens when they are staged.
#[derive(Default)]
pub(crate) struct Decoded {
    pub commits: Vec<(ca::CommitAddr, ca::Commit)>,
    pub graphs: Vec<(ca::GraphAddr, ca::DataGraph)>,
    pub blobs: Vec<(ca::SectionId, ca::BlobLiveness, ca::ContentAddr, ca::Bytes)>,
    pub sections: Vec<(
        ca::SectionId,
        ca::MergePolicy,
        ca::Liveness,
        ca::Key,
        ca::Value,
    )>,
}

/// Why fetched or pushed content cannot apply. See [`advance`].
#[derive(Debug)]
pub(crate) enum Rejected {
    Verify(ca::sync::VerifyError),
    Apply(ca::sync::ApplyError),
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Verify(e) => write!(f, "does not verify: {e}"),
            Self::Apply(e) => write!(f, "does not apply: {e}"),
        }
    }
}

/// Decode fetched objects. Undecodable graphs and section entries are
/// skipped with a warning. A skipped graph is wanted again.
pub(crate) fn decode(objects: Objects) -> Decoded {
    let mut decoded = Decoded::default();
    for object in objects.objects {
        match object {
            Object::Commit(addr, wire) => decoded.commits.push((addr, wire.into())),
            Object::Graph(addr, blob) => match proto::decode_graph(&blob) {
                Ok(graph) => decoded.graphs.push((addr, graph)),
                Err(e) => log::warn!("fetch: undecodable graph {addr}: {e}"),
            },
            Object::Blob {
                section,
                liveness,
                addr,
                bytes,
            } => decoded
                .blobs
                .push((section, liveness, addr, ca::Bytes::from(bytes))),
            Object::Section {
                id,
                policy,
                liveness,
                key,
                value,
            } => match proto::decode_value(&value) {
                Ok(value) => decoded.sections.push((id, policy, liveness, key, value)),
                Err(e) => log::warn!("fetch: undecodable section entry: {e}"),
            },
        }
    }
    decoded
}

/// Stage decoded content strictly. Commits the registry already holds are
/// skipped. Any object that fails verification rejects the whole set.
pub(crate) fn stage(
    registry: &ca::Registry,
    staged: &mut ca::sync::Staged,
    decoded: &Decoded,
) -> Result<(), ca::sync::VerifyError> {
    for (addr, commit) in &decoded.commits {
        if !registry.commits().contains_key(addr) {
            staged.insert_commit(*addr, commit.clone())?;
        }
    }
    for (addr, graph) in &decoded.graphs {
        staged.insert_graph(*addr, graph.clone())?;
    }
    for (section, liveness, addr, bytes) in &decoded.blobs {
        staged.insert_blob(section.clone(), *liveness, *addr, bytes.clone())?;
    }
    Ok(())
}

/// Everything still needed for `tip`'s closure. That is its commit and graph
/// closure via [`ca::sync::Staged::missing`], plus the staged graphs'
/// outgoing references via [`ca::data_graph_out`] that neither the registry
/// nor the staging area holds yet. Those references are nested graphs and
/// content-referenced blobs.
pub(crate) fn compute_want(
    registry: &ca::Registry,
    staged: &ca::sync::Staged,
    tip: ca::CommitAddr,
) -> Want {
    let missing = staged.missing(registry, tip);
    let mut refs: Vec<ObjectRef> = missing.commits.into_iter().map(ObjectRef::Commit).collect();
    let mut graph_wants: Vec<ca::GraphAddr> = missing.graphs;
    let mut blob_wants: Vec<(ca::SectionId, ca::ContentAddr)> = Vec::new();
    let staged_graphs: HashSet<ca::GraphAddr> = staged.graphs().map(|(ga, _)| *ga).collect();
    let staged_blobs: HashSet<(ca::SectionId, ca::ContentAddr)> = staged.blobs().cloned().collect();
    for (_, graph) in staged.graphs() {
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

/// Stage `decoded` toward `tip`'s closure, then apply the closure once it is
/// complete. Returns what the closure still misses, which is empty once it
/// is applied.
pub(crate) fn advance(
    registry: &mut ca::Registry,
    staged: &mut ca::sync::Staged,
    tip: ca::CommitAddr,
    decoded: &Decoded,
) -> Result<Want, Rejected> {
    stage(registry, staged, decoded).map_err(Rejected::Verify)?;
    let next = compute_want(registry, staged, tip);
    if next.is_empty() {
        std::mem::take(staged)
            .apply(registry)
            .map_err(Rejected::Apply)?;
    }
    Ok(next)
}

/// Converge a scoped name with a fully applied remote tip.
///
/// A name the host has open converges through [`Effect::RemoteTip`], so the
/// host can migrate its live state. Background names move headlessly,
/// followed by a reference resync. A minted merge is local content peers
/// have not seen, so it marks the sessions dirty.
fn resolve_tip(
    cx: &mut Cx<'_>,
    state: &mut SessionState,
    session: SessionId,
    name: &ca::Name,
    tip: ca::CommitAddr,
    resolutions: ca::merge::Resolutions,
) {
    let moved = |cx: &mut Cx<'_>, from: Option<ca::CommitAddr>, to: ca::CommitAddr| {
        cx.effects.push(Effect::Moved {
            session,
            name: name.clone(),
            from,
            to,
        });
        cx.effects.push(Effect::ResyncRefs);
    };
    let Some(local) = cx.registry.head(name) else {
        // A name born on the remote side. Adopt it.
        cx.registry.set_head(name.clone(), tip);
        state.last_announced.insert(name.clone(), tip);
        moved(cx, None, tip);
        return;
    };
    // The join flow's placeholder is unrelated to the session content it
    // awaits by design. Adopt over it rather than surface `Unrelated`.
    let adopt_unrelated = state.placeholder == Some(local);
    let plan = ca::plan_sync_step(cx.registry.commits(), local, tip);
    let is_open = cx.open.contains_key(name);
    match (is_open, plan) {
        (_, ca::SyncStep::UpToDate) => (),
        (_, ca::SyncStep::Adopt(t)) if t == local => (),
        (is_open, ca::SyncStep::Unrelated) => {
            if !adopt_unrelated {
                log::warn!("session: remote tip for '{name}' shares no local history; ignoring");
                return;
            }
            state.placeholder = None;
            state.last_announced.insert(name.clone(), tip);
            if is_open {
                // The host navigates the open head onto the adopted tip,
                // which moves the name.
                cx.effects.push(Effect::RemoteTip {
                    name: name.clone(),
                    remote: tip,
                    resolutions,
                    adopt_unrelated: true,
                });
            } else {
                cx.registry.set_head(name.clone(), tip);
                moved(cx, Some(local), tip);
            }
        }
        (true, plan) => {
            // Adoptions of received tips are not re-announced.
            if let ca::SyncStep::FastForward(t) | ca::SyncStep::Adopt(t) = plan {
                state.last_announced.insert(name.clone(), t);
            }
            cx.effects.push(Effect::RemoteTip {
                name: name.clone(),
                remote: tip,
                resolutions,
                adopt_unrelated: false,
            });
        }
        (false, ca::SyncStep::FastForward(t) | ca::SyncStep::Adopt(t)) => {
            cx.registry.set_head(name.clone(), t);
            state.last_announced.insert(name.clone(), t);
            moved(cx, Some(local), t);
        }
        (false, ca::SyncStep::Merge { first, second }) => {
            match merge_headless(cx.registry, name, first, second, resolutions) {
                Ok(Some((minted, conflicts))) => {
                    state.conflicts += conflicts;
                    *cx.dirty = true;
                    moved(cx, Some(local), minted);
                }
                Ok(None) => (),
                Err(e) => log::warn!("session: headless merge of '{name}' failed: {e}"),
            }
        }
    }
}

/// Mint the canonical merge of two diverged tips as `name`'s head. Returns
/// the merge commit and its conflict count, or `None` if the tips did not
/// diverge.
///
/// The merge commit's view is seeded from the tips' stored views before it
/// can be announced. A viewless tip on the wire auto-layouts on every
/// adopting peer. An open head seeds from its live layout instead.
pub(crate) fn merge_headless(
    registry: &mut ca::Registry,
    name: &ca::Name,
    first: ca::CommitAddr,
    second: ca::CommitAddr,
    resolutions: ca::merge::Resolutions,
) -> Result<Option<(ca::CommitAddr, usize)>, ca::MergeError> {
    let ca::MergeResolution::Diverged { outcome, .. } =
        ca::merge_commits(registry, first, second, resolutions)?
    else {
        return Ok(None);
    };
    let conflicts = outcome.conflicts.len();
    let graph_ca = ca::graph_addr(&outcome.graph);
    let mut branch_head = ca::Head::Branch(name.clone());
    let first_view = gantz_egui::section::view(registry, &first);
    let second_view = gantz_egui::section::view(registry, &second);
    let seeded = gantz_egui::ops::merged_view(
        &outcome.node_srcs,
        first_view.as_ref(),
        second_view.as_ref(),
    );
    let graph = outcome.graph;
    let minted =
        registry.commit_merge_canonical(first, second, graph_ca, || graph, &mut branch_head);
    gantz_egui::section::seed_view(registry, minted, &seeded);
    Ok(Some((minted, conflicts)))
}
