//! Capture and broadcast of ephemeral node-interaction actions.
//!
//! Durable actions are commits and ride the tips/fetch machinery; this
//! module handles the fire-and-forget rest (see [`gantz_egui::action`]):
//! live VM-state writes and eval triggers on session heads, mirrored to
//! peers as [`GossipMsg::Action`]s so a dialer drag or a `bang` click is
//! seen live everywhere.
//!
//! Capture happens by overriding two payload dispatchers (last registration
//! wins; `CollabPlugin` is added after `GantzEguiPlugin`):
//! [`gantz_egui::StateWritten`] (recorded by `NodeCtx::update_value` - all
//! nodes, builtin or custom, are covered with zero per-node work) and
//! [`gantz_egui::EvalEntry`] (which keeps its local behaviour byte-for-byte
//! and additionally captures). Only heads carrying a [`SessionRef`] are
//! captured; everything else stays local.
//!
//! The outbox fuses a state write with the push-eval it triggered in the
//! same frame (atomic set-then-evaluate remotely) and rate-limits value
//! sends per node path. Rate limiting BATCHES, it never drops: every value
//! written within a window ships (oldest first) and replays step-by-step
//! on peers, so accumulative downstream state (e.g. a scope plot sampling
//! per evaluation) stays in step with the emitting peer, and the final
//! drag value always ships on the trailing edge. Evals bypass the limit:
//! every click ships.

use crate::{CollabIdentity, CollabRuntime, CollabSessions, SessionRef};
use bevy_ecs::prelude::*;
use bevy_gantz::head;
use bevy_gantz::reg::Registry;
use bevy_gantz::vm::EvalEntryEvent;
use bevy_log as log;
use gantz_ca as ca;
use gantz_collab::{Command, GossipMsg, PeerId, SessionId, proto};
use gantz_core::compile::entrypoint::Entrypoint;
use gantz_core::node;
use gantz_egui::DynResponse;
use gantz_egui::action::{Action, Source, Value};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use web_time::Instant;

// The minimum interval between value sends for one node path is the
// user-configurable `CollabConfig::action_rate_ms` (default 50ms: a 60 Hz
// drag becomes ~20 msg/s). The pending slot batches the window's values
// meanwhile and flushes when the window elapses, so every step (and thus
// the final value) always ships regardless of the window length.

/// How long a received action waits for its graph anchor (a tip likely still
/// in flight) before it is dropped.
const RETRY_DEADLINE: Duration = Duration::from_secs(1);

/// The most values one pending slot batches; beyond it the OLDEST drops
/// (degrading gracefully to coalescing). A backstop for pathological frame
/// hitches - a 60 Hz drag batches 3-4 values per window - that also keeps
/// the encoded action far below [`proto::MAX_ACTION_DATA`] for the value
/// shapes interactive nodes write.
const MAX_BATCHED_WRITES: usize = 64;

/// The received-action queue bound; a backstop, not a working limit.
const INBOX_CAP: usize = 1024;

/// The action-history ring-buffer capacity.
const LOG_CAP: usize = 256;

// ----------------------------------------------------------------------------
// Capture events
// ----------------------------------------------------------------------------

/// A node UI on `head` wrote VM state this frame (dispatcher-captured).
#[derive(Debug, Event)]
pub struct CaptureWrite {
    pub head: Entity,
    pub write: gantz_egui::action::StateWrite,
}

/// A node UI on `head` triggered an entrypoint evaluation this frame.
#[derive(Debug, Event)]
pub struct CaptureEval {
    pub head: Entity,
    pub entrypoint: Entrypoint,
}

// ----------------------------------------------------------------------------
// Resources
// ----------------------------------------------------------------------------

/// The key addressing one node's pending value within a session.
type PathKey = (SessionId, ca::Name, Vec<node::Id>);

/// The fused, not-yet-sent state writes of one node's send window.
#[derive(Default)]
struct PendingWrite {
    /// Every value written this window, oldest first (bounded by
    /// [`MAX_BATCHED_WRITES`]).
    values: Vec<Value>,
    /// The push-eval fused with these writes, when the node triggered one.
    eval: Option<Source>,
}

/// Outbound ephemeral actions: per-path fusion, batching and rate state.
#[derive(Default, Resource)]
pub struct ActionOutbox {
    /// The unsent values per node path, oldest first.
    pending: HashMap<PathKey, PendingWrite>,
    /// When each path last shipped, for rate limiting.
    last_sent: HashMap<PathKey, Instant>,
    /// Standalone evals; all flushed every pass.
    evals: Vec<(SessionId, ca::Name, Vec<Source>)>,
    /// Per-session action sequence counters.
    seq: HashMap<SessionId, u64>,
}

/// A received action awaiting application (or its graph anchor).
#[derive(Clone, Debug)]
pub struct InboundAction {
    pub session: SessionId,
    pub origin: PeerId,
    /// Per-origin sequence number. Carried for debugging/future use: values
    /// converge via last-write-wins and evals apply per delivery
    /// (iroh-gossip dedups a broadcast, so duplicates are effectively
    /// absent), so no seq-based stale-drop is needed.
    pub seq: u64,
    /// Sender wall-clock milliseconds since the epoch.
    pub timestamp: u64,
    pub name: ca::Name,
    /// The graph the action's node paths are meaningful for.
    pub graph: ca::GraphAddr,
    /// The still-encoded [`Action`].
    pub data: Vec<u8>,
    /// When this peer received it, for the retry deadline.
    pub received: Instant,
}

/// Inbound ephemeral actions and the value-convergence bookkeeping.
#[derive(Default, Resource)]
pub struct ActionInbox {
    queue: Vec<InboundAction>,
    /// Last-write-wins: the newest applied `(timestamp, origin)` per node
    /// path, so reordered or concurrent value writes converge on the newest
    /// (ties broken by origin id).
    last_applied: HashMap<PathKey, (u64, PeerId)>,
}

impl ActionInbox {
    /// Queue a received action for application (bounded backstop).
    pub(crate) fn receive(&mut self, inbound: InboundAction) {
        if self.queue.len() >= INBOX_CAP {
            log::warn!("session action inbox full; dropping oldest");
            self.queue.remove(0);
        }
        self.queue.push(inbound);
    }
}

/// One entry in the session activity history.
#[derive(Clone, Debug)]
pub struct ActionLogEntry {
    /// Wall-clock milliseconds since the epoch.
    pub timestamp: u64,
    pub session: SessionId,
    /// The originating peer; `None` = this peer.
    pub peer: Option<PeerId>,
    /// The scoped name the action applied to.
    pub name: ca::Name,
    /// A compact human-readable summary.
    pub summary: String,
}

/// The session action history: a bounded ring buffer of sent and received
/// ephemeral actions, oldest first.
///
/// Maintained for a future dedicated activity widget; every entry is also
/// emitted as a `log::debug!` line so the Logs pane picks actions up today.
#[derive(Resource)]
pub struct ActionLog {
    entries: VecDeque<ActionLogEntry>,
}

impl Default for ActionLog {
    fn default() -> Self {
        Self {
            entries: VecDeque::with_capacity(LOG_CAP),
        }
    }
}

impl ActionLog {
    /// Record an entry (also emitted as a `log::debug!` line).
    pub fn push(&mut self, entry: ActionLogEntry) {
        let who = match &entry.peer {
            Some(peer) => format!("{peer}"),
            None => "local".to_string(),
        };
        log::debug!("session action [{}] {who}: {}", entry.name, entry.summary);
        if self.entries.len() == LOG_CAP {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// The recorded entries, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &ActionLogEntry> + '_ {
        self.entries.iter()
    }
}

// ----------------------------------------------------------------------------
// Dispatcher overrides (capture)
// ----------------------------------------------------------------------------

/// Dispatch a [`gantz_egui::StateWritten`] payload: forward it for capture.
///
/// Overrides `bevy_gantz_egui`'s no-op registration (last registration
/// wins). Non-session heads are filtered at the observer.
pub(crate) fn dispatch_state_written(
    entity: Option<Entity>,
    payload: DynResponse,
    cmds: &mut Commands,
) {
    let Some(head) = entity else {
        return;
    };
    let gantz_egui::StateWritten(write) = bevy_gantz_egui::downcast_payload(payload);
    cmds.trigger(CaptureWrite { head, write });
}

/// Dispatch a [`gantz_egui::EvalEntry`] payload: trigger the evaluation
/// exactly as `bevy_gantz_egui`'s own dispatcher would, and additionally
/// forward it for capture.
pub(crate) fn dispatch_eval_entry(
    entity: Option<Entity>,
    payload: DynResponse,
    cmds: &mut Commands,
) {
    let Some(head) = entity else {
        log::error!("EvalEntry payload has no open-head entity");
        return;
    };
    let gantz_egui::EvalEntry(entrypoint) = bevy_gantz_egui::downcast_payload(payload);
    cmds.trigger(EvalEntryEvent {
        head,
        entrypoint: entrypoint.clone(),
        // A user-driven push fires "now" (mirrors the wrapped dispatcher).
        time: None,
    });
    cmds.trigger(CaptureEval { head, entrypoint });
}

// ----------------------------------------------------------------------------
// Capture observers
// ----------------------------------------------------------------------------

/// Resolve the session and scoped name a captured head belongs to.
///
/// `None` for non-session heads (nothing is captured) and non-branch heads.
fn session_name(
    head: Entity,
    heads: &Query<(&head::HeadRef, &SessionRef)>,
) -> Option<(SessionId, ca::Name)> {
    let (head_ref, session_ref) = heads.get(head).ok()?;
    let ca::Head::Branch(name) = &head_ref.0 else {
        return None;
    };
    Some((session_ref.0, name.clone()))
}

/// When an entrypoint is a single push source, its source (for fusion with
/// the state write that triggered it).
fn single_push_source(ep: &Entrypoint) -> Option<Source> {
    if ep.0.len() != 1 {
        return None;
    }
    let src = ep.0.first()?;
    matches!(src.kind, gantz_core::compile::entrypoint::EvalKind::Push)
        .then(|| Source::from(src.clone()))
}

/// Record a captured state write into the outbox, fusing with a standalone
/// eval already queued for the same node this pass (payload order within a
/// frame is not guaranteed - the eval may have been captured first).
pub fn on_capture_write(
    trigger: On<CaptureWrite>,
    mut outbox: ResMut<ActionOutbox>,
    heads: Query<(&head::HeadRef, &SessionRef)>,
) {
    let ev = trigger.event();
    let Some((session, name)) = session_name(ev.head, &heads) else {
        return;
    };
    // A standalone push-eval already queued for the same node belongs to
    // this write: fuse it back in (payload order within a frame is not
    // guaranteed - the eval may have been captured first).
    let mut rescued = None;
    if let Some(ix) = outbox.evals.iter().position(|(s, n, sources)| {
        *s == session
            && *n == name
            && matches!(&sources[..], [src] if src.path == ev.write.path
                && src.kind == gantz_egui::action::Kind::Push)
    }) {
        let (_, _, mut sources) = outbox.evals.remove(ix);
        rescued = sources.pop();
    }
    // The slot updates in place: the window's earlier values stay batched
    // (rate limiting must never drop a step) and an eval fused earlier in
    // the window is retained. Nothing carries across flushes (shipping
    // removes the slot), so a window without a push-eval can never re-fire
    // an already-shipped eval.
    let key = (session, name, ev.write.path.clone());
    let pending = outbox.pending.entry(key).or_default();
    pending.values.push(ev.write.value.clone());
    if pending.values.len() > MAX_BATCHED_WRITES {
        log::debug!("session write batch full; dropping the oldest value");
        pending.values.remove(0);
    }
    if rescued.is_some() {
        pending.eval = rescued;
    }
}

/// Record a captured evaluation into the outbox: fused into the pending
/// write for the same node when one exists (the dialer's set-then-evaluate),
/// standalone otherwise (a bang).
pub fn on_capture_eval(
    trigger: On<CaptureEval>,
    mut outbox: ResMut<ActionOutbox>,
    heads: Query<(&head::HeadRef, &SessionRef)>,
) {
    let ev = trigger.event();
    let Some((session, name)) = session_name(ev.head, &heads) else {
        return;
    };
    if let Some(src) = single_push_source(&ev.entrypoint) {
        let key = (session, name.clone(), src.path.clone());
        if let Some(pending) = outbox.pending.get_mut(&key) {
            pending.eval = Some(src);
            return;
        }
    }
    let sources: Vec<Source> = ev.entrypoint.0.iter().cloned().map(Source::from).collect();
    outbox.evals.push((session, name, sources));
}

// ----------------------------------------------------------------------------
// Broadcast
// ----------------------------------------------------------------------------

/// Broadcast the outbox: evals immediately, pending values when their
/// rate-limit window allows (the newest value per path always ships
/// eventually). Runs after `VmSet` beside the tip announce.
pub fn broadcast_actions(
    runtime: Res<CollabRuntime>,
    identity: Option<Res<CollabIdentity>>,
    sessions: Res<CollabSessions>,
    registry: Res<Registry>,
    gui_state: Res<bevy_gantz_egui::GuiState>,
    mut outbox: ResMut<ActionOutbox>,
    mut activity: ResMut<ActionLog>,
) {
    let (Some(handle), Some(identity)) = (runtime.0.as_ref(), identity) else {
        outbox.pending.clear();
        outbox.evals.clear();
        return;
    };
    let origin = identity.0.peer_id();

    // Drop entries for sessions that no longer exist.
    let live = |s: &SessionId| sessions.sessions.contains_key(s);
    outbox.pending.retain(|(s, ..), _| live(s));
    outbox.last_sent.retain(|(s, ..), _| live(s));
    outbox.evals.retain(|(s, ..)| live(s));

    // The anchor: the committed graph addr the action was issued against.
    let anchor = |name: &ca::Name| {
        registry
            .head_commit(&ca::Head::Branch(name.clone()))
            .map(|c| c.graph)
    };

    // Standalone evals ship immediately.
    for (session, name, sources) in std::mem::take(&mut outbox.evals) {
        let Some(graph) = anchor(&name) else {
            continue;
        };
        let summary = format!("eval {}", sources_summary(&sources));
        let action = Action::Eval { sources };
        send(
            handle,
            &mut outbox,
            &mut activity,
            origin,
            session,
            name,
            graph,
            action,
            summary,
        );
    }

    // Pending value batches ship when their window allows (the window is
    // the user-configurable send rate; batching means no step is lost
    // however long it is).
    let rate = Duration::from_millis(gui_state.0.collab.action_rate_ms);
    let now = Instant::now();
    let due: Vec<PathKey> = outbox
        .pending
        .keys()
        .filter(|key| {
            outbox
                .last_sent
                .get(*key)
                .is_none_or(|&at| now.duration_since(at) >= rate)
        })
        .cloned()
        .collect();
    for key in due {
        let Some(PendingWrite { mut values, eval }) = outbox.pending.remove(&key) else {
            continue;
        };
        let Some(last) = values.last().cloned() else {
            continue;
        };
        let (session, name, path) = key.clone();
        let Some(graph) = anchor(&name) else {
            continue;
        };
        let summary = format!(
            "set {path:?} = {}{}{}",
            value_summary(&last),
            match values.len() {
                1 => String::new(),
                n => format!(" (x{n})"),
            },
            if eval.is_some() { " +eval" } else { "" },
        );
        let mut action = Action::SetState {
            path: path.clone(),
            values: std::mem::take(&mut values),
            eval: eval.clone(),
        };
        if !send_fits(&action) {
            // Degrade to the newest value alone rather than losing the
            // window outright (large `Str`/`List` values can overflow the
            // envelope even uncoalesced; that final drop keeps its warning).
            log::debug!("session write batch oversized; coalescing to the newest value");
            action = Action::SetState {
                path,
                values: vec![last],
                eval,
            };
        }
        if send(
            handle,
            &mut outbox,
            &mut activity,
            origin,
            session,
            name,
            graph,
            action,
            summary,
        ) {
            outbox.last_sent.insert(key, now);
        }
    }
}

/// Whether the encoded action fits the gossip envelope's size cap.
fn send_fits(action: &Action) -> bool {
    proto::encode(action).len() <= proto::MAX_ACTION_DATA
}

/// Encode and broadcast one action; returns whether it shipped.
#[allow(clippy::too_many_arguments)]
fn send(
    handle: &gantz_collab::Handle,
    outbox: &mut ActionOutbox,
    activity: &mut ActionLog,
    origin: PeerId,
    session: SessionId,
    name: ca::Name,
    graph: ca::GraphAddr,
    action: Action,
    summary: String,
) -> bool {
    let data = proto::encode(&action);
    if data.len() > proto::MAX_ACTION_DATA {
        log::warn!(
            "dropping oversized session action for '{name}' ({} bytes > {})",
            data.len(),
            proto::MAX_ACTION_DATA,
        );
        return false;
    }
    let seq = outbox.seq.entry(session).or_default();
    *seq += 1;
    let timestamp = bevy_gantz::reg::timestamp().as_millis() as u64;
    let msg = GossipMsg::Action {
        origin,
        seq: *seq,
        timestamp,
        name: name.clone(),
        graph,
        data,
    };
    let _ = handle.cmds.try_send(Command::Broadcast { session, msg });
    activity.push(ActionLogEntry {
        timestamp,
        session,
        peer: None,
        name,
        summary,
    });
    true
}

// ----------------------------------------------------------------------------
// Remote application
// ----------------------------------------------------------------------------

/// Apply received actions to the matching open heads' VMs.
///
/// Runs after `poll_collab_events` and before
/// `VmSet`, so writes land before evaluation systems observe the frame. The
/// apply path uses `gantz_core::node::state::update_value` and
/// `EvalEntryEvent` directly - never a `NodeCtx`, never the payload bus - so
/// nothing here can re-broadcast (the capture and apply channels are
/// physically disjoint).
///
/// Each action applies only while the local tip holds the IDENTICAL graph it
/// was issued against: anchor equality guarantees node-index identity, which
/// is what makes bare index paths safe (`state::update_value` would happily
/// create state at any path on a diverged graph). A mismatch is usually a
/// tip in flight, so actions retry briefly before dropping; an action for a
/// just-deleted node expires the same way (deletion moved the anchor).
pub fn apply_remote_actions(
    identity: Option<Res<CollabIdentity>>,
    registry: Res<Registry>,
    mut inbox: ResMut<ActionInbox>,
    mut vms: NonSendMut<head::HeadVms>,
    open: Query<(Entity, &head::HeadRef), With<head::OpenHead>>,
    mut activity: ResMut<ActionLog>,
    mut cmds: Commands,
) {
    if inbox.queue.is_empty() {
        return;
    }
    let self_id = identity.as_ref().map(|i| i.0.peer_id());
    let now = Instant::now();
    let mut retry = Vec::new();
    let queue = std::mem::take(&mut inbox.queue);
    for inbound in queue {
        // Gossip broadcasts don't self-deliver; guard anyway.
        if Some(inbound.origin) == self_id {
            continue;
        }
        let head = ca::Head::Branch(inbound.name.clone());
        let anchor = registry.head_commit(&head).map(|c| c.graph);
        if anchor != Some(inbound.graph) {
            if now.duration_since(inbound.received) < RETRY_DEADLINE {
                retry.push(inbound);
            } else {
                log::debug!(
                    "dropping session action for '{}': graph anchor mismatch",
                    inbound.name,
                );
            }
            continue;
        }
        // An ephemeral action for a closed tab is meaningless: no VM runs it.
        let Some(entity) = open
            .iter()
            .find(|(_, hr)| hr.0 == head)
            .map(|(entity, _)| entity)
        else {
            continue;
        };
        let Some(vm) = vms.get_mut(&entity) else {
            continue;
        };
        let action: Action = match proto::decode(&inbound.data) {
            Ok(action) => action,
            Err(e) => {
                log::debug!("undecodable session action from {}: {e}", inbound.origin);
                continue;
            }
        };
        let node_count = registry.head_graph(&head).map(|g| g.node_count());
        match action {
            Action::SetState { path, values, eval } => {
                let Some(last) = values.last() else {
                    continue;
                };
                // Last-write-wins per path on the batch's stamp:
                // reordered/concurrent batches converge on the newest,
                // origin id breaking ties.
                let key = (inbound.session, inbound.name.clone(), path.clone());
                let stamp = (inbound.timestamp, inbound.origin);
                if inbox
                    .last_applied
                    .get(&key)
                    .is_some_and(|&(ts, origin)| (ts, origin.0) >= (stamp.0, stamp.1.0))
                {
                    continue;
                }
                // Anchor equality already guarantees the path came from a
                // real node of this exact graph; bound-check the root index
                // as defense-in-depth against the lazy-map hazard.
                if path
                    .first()
                    .zip(node_count)
                    .is_none_or(|(&ix, count)| ix >= count)
                {
                    continue;
                }
                let summary = format!(
                    "set {path:?} = {}{}{}",
                    value_summary(last),
                    match values.len() {
                        1 => String::new(),
                        n => format!(" (x{n})"),
                    },
                    if eval.is_some() { " +eval" } else { "" },
                );
                let entrypoint = eval
                    .map(|src| gantz_egui::action::entrypoint([src]))
                    .filter(|ep| entry_fn_exists(vm, ep));
                // Replay the batch through the command queue: writes as
                // queued world closures, evals as triggers. FIFO command
                // application interleaves them w1,e1,w2,e2,... so each eval
                // observes its own step's value - a direct write here would
                // land before ANY deferred eval fired, collapsing every
                // step onto the final value.
                for value in values {
                    let path = path.clone();
                    cmds.queue(move |world: &mut World| {
                        let mut vms = world.non_send_mut::<head::HeadVms>();
                        let Some(vm) = vms.0.get_mut(&entity) else {
                            return;
                        };
                        if let Err(e) =
                            gantz_core::node::state::update_value(vm, &path, value.into())
                        {
                            log::warn!("failed to apply remote state write: {e}");
                        }
                    });
                    if let Some(ep) = &entrypoint {
                        // A remote push fires "now" on this peer's clock
                        // (matching local user-driven pushes).
                        cmds.trigger(EvalEntryEvent {
                            head: entity,
                            entrypoint: ep.clone(),
                            time: None,
                        });
                    }
                }
                inbox.last_applied.insert(key, stamp);
                activity.push(ActionLogEntry {
                    timestamp: inbound.timestamp,
                    session: inbound.session,
                    peer: Some(inbound.origin),
                    name: inbound.name,
                    summary,
                });
            }
            Action::Eval { sources } => {
                let summary = format!("eval {}", sources_summary(&sources));
                let ep = gantz_egui::action::entrypoint(sources);
                if trigger_guarded_eval(vm, entity, ep, &mut cmds) {
                    activity.push(ActionLogEntry {
                        timestamp: inbound.timestamp,
                        session: inbound.session,
                        peer: Some(inbound.origin),
                        name: inbound.name,
                        summary,
                    });
                }
            }
            Action::Custom { tag, .. } => {
                log::debug!("dropping custom session action '{tag}': no codec routed");
            }
        }
    }
    inbox.queue = retry;
}

/// Whether the entrypoint's generated entry fn exists in the VM: a
/// config-divergent peer (e.g. `emit_all_node_fns` differences) logs one
/// debug line instead of pushing a spurious runtime diagnostic through the
/// eval error path.
fn entry_fn_exists(vm: &steel::steel_vm::engine::Engine, entrypoint: &Entrypoint) -> bool {
    let fn_name = gantz_core::compile::entry_fn_name(&entrypoint.id());
    let exists = vm.extract_value(&fn_name).is_ok();
    if !exists {
        log::debug!("remote eval skipped: entry fn {fn_name} is not compiled locally");
    }
    exists
}

/// Trigger an entrypoint evaluation iff its generated entry fn exists in
/// the VM (see [`entry_fn_exists`]). Returns whether the eval was
/// triggered.
fn trigger_guarded_eval(
    vm: &steel::steel_vm::engine::Engine,
    head: Entity,
    entrypoint: Entrypoint,
    cmds: &mut Commands,
) -> bool {
    if !entry_fn_exists(vm, &entrypoint) {
        return false;
    }
    // A remote push fires "now" on this peer's clock (matching local
    // user-driven pushes).
    cmds.trigger(EvalEntryEvent {
        head,
        entrypoint,
        time: None,
    });
    true
}

/// A compact display form for a value (log/history lines).
pub(crate) fn value_summary(value: &Value) -> String {
    match value {
        Value::Unit => "()".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Num(n) => format!("{n:.4}"),
        Value::Char(c) => format!("{c:?}"),
        Value::Str(s) if s.chars().count() <= 16 => format!("{s:?}"),
        Value::Str(s) => format!("{:?}…", s.chars().take(16).collect::<String>()),
        Value::List(l) => format!("[{} items]", l.len()),
    }
}

/// A compact display form for eval sources (log/history lines).
pub(crate) fn sources_summary(sources: &[Source]) -> String {
    let paths: Vec<String> = sources.iter().map(|s| format!("{:?}", s.path)).collect();
    paths.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::compile::entrypoint::{Entrypoint, EvalKind, EvalSource};

    /// A capture world with one open session head; returns the world, the
    /// head entity and the outbox key parts.
    fn capture_world() -> (World, Entity, SessionId, ca::Name) {
        let mut world = World::new();
        world.init_resource::<ActionOutbox>();
        world.add_observer(on_capture_write);
        world.add_observer(on_capture_eval);
        let session = SessionId::generate();
        let name: ca::Name = "jam".parse().unwrap();
        let head = world
            .spawn((
                head::HeadRef(ca::Head::Branch(name.clone())),
                SessionRef(session),
            ))
            .id();
        (world, head, session, name)
    }

    fn push_entrypoint(path: &[node::Id]) -> Entrypoint {
        let source = EvalSource {
            path: path.to_vec(),
            kind: EvalKind::Push,
            conns: gantz_core::node::Conns::empty(),
        };
        Entrypoint([source].into_iter().collect())
    }

    fn write(path: &[node::Id], v: f64) -> gantz_egui::action::StateWrite {
        gantz_egui::action::StateWrite {
            path: path.to_vec(),
            value: Value::Num(v),
        }
    }

    // Regression: during a drag, rate limiting can hold an unshipped pending
    // while the next frame's payloads arrive eval-first (payload order
    // within a frame is not guaranteed). The eval fuses into the held
    // pending; the frame's write must keep both the fused eval and the
    // earlier values - otherwise the flushed `SetState` ships without the
    // eval (the remote value updates but downstream never re-evaluates) or
    // with steps missing (accumulative downstream state drifts).
    #[test]
    fn writes_batch_and_keep_the_fused_eval_in_either_order() {
        let (mut world, head, session, name) = capture_world();
        let path = vec![0usize];
        let entrypoint = push_entrypoint(&path);

        // Frame 1: write + eval -> fused pending, held by the rate limit.
        world.trigger(CaptureWrite {
            head,
            write: write(&path, 1.0),
        });
        world.trigger(CaptureEval {
            head,
            entrypoint: entrypoint.clone(),
        });
        // Frame 2, hazardous order: the eval fuses into the held pending
        // before the frame's own write lands.
        world.trigger(CaptureEval {
            head,
            entrypoint: entrypoint.clone(),
        });
        world.trigger(CaptureWrite {
            head,
            write: write(&path, 2.0),
        });

        let outbox = world.resource::<ActionOutbox>();
        let key = (session, name, path);
        let pending = outbox.pending.get(&key).expect("a pending write");
        assert_eq!(
            pending.values,
            vec![Value::Num(1.0), Value::Num(2.0)],
            "every step of the window must ship, oldest first"
        );
        assert!(
            pending.eval.is_some(),
            "the fused eval must survive the newer write"
        );
        assert!(outbox.evals.is_empty(), "no standalone eval may leak");
    }

    // The batch bound degrades gracefully to coalescing: the OLDEST value
    // drops, so the final value always ships.
    #[test]
    fn batch_cap_drops_the_oldest_value() {
        let (mut world, head, session, name) = capture_world();
        let path = vec![0usize];
        for i in 0..(MAX_BATCHED_WRITES + 2) {
            world.trigger(CaptureWrite {
                head,
                write: write(&path, i as f64),
            });
        }
        let outbox = world.resource::<ActionOutbox>();
        let key = (session, name, path);
        let pending = outbox.pending.get(&key).expect("a pending write");
        assert_eq!(pending.values.len(), MAX_BATCHED_WRITES);
        assert_eq!(pending.values.first(), Some(&Value::Num(2.0)));
        assert_eq!(
            pending.values.last(),
            Some(&Value::Num((MAX_BATCHED_WRITES + 1) as f64))
        );
    }
}
