//! Capture and broadcast of ephemeral node-interaction actions.
//!
//! Durable actions are commits and ride the tips and fetch machinery. This
//! module handles the fire-and-forget rest described in
//! [`gantz_egui::action`]. Live VM-state writes and eval triggers on session
//! heads are mirrored to peers as [`GossipMsg::Action`]s, so a dialer drag or
//! a `bang` click is seen live everywhere.
//!
//! Capture overrides two payload dispatchers. The last registration wins, and
//! `CollabPlugin` is added after `GantzEguiPlugin`.
//! [`gantz_egui::StateWritten`] is recorded by `NodeCtx::update_value`, so
//! every node is covered with no per-node work. [`gantz_egui::EvalEntry`]
//! keeps its local behaviour and also captures. Only heads that carry a
//! [`SessionRef`] are captured. Everything else stays local.
//!
//! The outbox fuses a state write with the push-eval it triggered in the same
//! frame, so peers set then evaluate atomically. It rate-limits value sends
//! per node path. Rate limiting batches and never drops. Every value written
//! within a window ships oldest first and replays step by step on peers, so
//! accumulative downstream state stays in step with the emitting peer. The
//! final drag value always ships on the trailing edge. Evals bypass the
//! limit. Every click ships.

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
// user-configurable `CollabConfig::action_rate_ms`. The pending slot batches
// the window's values and flushes when the window elapses, so every step
// ships whatever the window length.

/// How long a received action waits for its graph anchor before it is
/// dropped. The anchor is usually a tip still in flight.
const RETRY_DEADLINE: Duration = Duration::from_secs(1);

/// The most values one pending slot batches. Beyond it the oldest drops, so
/// the slot degrades to coalescing. This is a backstop for pathological
/// frame hitches. At the default rate a 60 Hz drag batches about one value
/// per window. The bound also keeps the encoded action far below
/// [`proto::MAX_ACTION_DATA`] for the value shapes interactive nodes write.
const MAX_BATCHED_WRITES: usize = 64;

/// The received-action queue bound. A backstop, not a working limit.
const INBOX_CAP: usize = 1024;

/// The action-history ring-buffer capacity.
const LOG_CAP: usize = 256;

/// A node UI on `head` wrote VM state this frame.
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

/// The key addressing one node's pending value within a session.
type PathKey = (SessionId, ca::Name, Vec<node::Id>);

/// The fused, not-yet-sent state writes of one node's send window.
#[derive(Default)]
struct PendingWrite {
    /// Every value written this window, oldest first. Bounded by
    /// [`MAX_BATCHED_WRITES`].
    values: Vec<Value>,
    /// The push-eval fused with these writes, when the node triggered one.
    eval: Option<Source>,
}

/// Outbound ephemeral actions with per-path fusion, batching and rate state.
#[derive(Default, Resource)]
pub struct ActionOutbox {
    /// The unsent values per node path, oldest first.
    pending: HashMap<PathKey, PendingWrite>,
    /// When each path last shipped, for rate limiting.
    last_sent: HashMap<PathKey, Instant>,
    /// Standalone evals. All are flushed every pass.
    evals: Vec<(SessionId, ca::Name, Vec<Source>)>,
    /// Per-session action sequence counters.
    seq: HashMap<SessionId, u64>,
}

/// A received action awaiting application or its graph anchor.
#[derive(Clone, Debug)]
pub struct InboundAction {
    pub session: SessionId,
    pub origin: PeerId,
    /// Per-origin sequence number, carried for debugging. Values converge
    /// via last-write-wins and evals apply per delivery, so no seq-based
    /// stale-drop is needed. iroh-gossip dedups a broadcast, so duplicates
    /// are absent.
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
    /// The newest applied `(timestamp, origin)` per node path. Reordered or
    /// concurrent value writes converge on the newest. Ties break by origin
    /// id.
    last_applied: HashMap<PathKey, (u64, PeerId)>,
}

impl ActionInbox {
    /// Queue a received action for application. The queue is bounded.
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
    /// The originating peer. `None` means this peer.
    pub peer: Option<PeerId>,
    /// The scoped name the action applied to.
    pub name: ca::Name,
    /// A compact human-readable summary.
    pub summary: String,
}

/// The session action history, a bounded ring buffer of sent and received
/// ephemeral actions, oldest first.
///
/// Every entry is also emitted as a `log::debug!` line, so the Logs pane
/// shows actions.
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
    /// Record an entry and emit it as a `log::debug!` line.
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

// Dispatcher overrides

/// Dispatch a [`gantz_egui::StateWritten`] payload by forwarding it for
/// capture.
///
/// Overrides `bevy_gantz_egui`'s no-op registration. The observer filters
/// out non-session heads.
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

/// Dispatch a [`gantz_egui::EvalEntry`] payload. Trigger the evaluation
/// exactly as `bevy_gantz_egui`'s own dispatcher would, then forward it for
/// capture.
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
        // A user-driven push fires now, as in the wrapped dispatcher.
        time: None,
    });
    cmds.trigger(CaptureEval { head, entrypoint });
}

// Capture observers

/// Resolve the session and scoped name a captured head belongs to.
///
/// `None` for non-session heads and non-branch heads. Nothing is captured
/// for them.
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

/// The source of an entrypoint that is a single push source, for fusion with
/// the state write that triggered it.
fn single_push_source(ep: &Entrypoint) -> Option<Source> {
    if ep.0.len() != 1 {
        return None;
    }
    let src = ep.0.first()?;
    matches!(src.kind, gantz_core::compile::entrypoint::EvalKind::Push)
        .then(|| Source::from(src.clone()))
}

/// Record a captured state write into the outbox. A standalone eval already
/// queued for the same node this pass is fused back in, because payload
/// order within a frame is not guaranteed.
pub fn on_capture_write(
    trigger: On<CaptureWrite>,
    mut outbox: ResMut<ActionOutbox>,
    heads: Query<(&head::HeadRef, &SessionRef)>,
) {
    let ev = trigger.event();
    let Some((session, name)) = session_name(ev.head, &heads) else {
        return;
    };
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
    // The slot updates in place. The window's earlier values stay batched,
    // because rate limiting must never drop a step. An eval fused earlier in
    // the window is retained. Shipping removes the slot, so a window without
    // a push-eval can never re-fire an already-shipped eval.
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

/// Record a captured evaluation into the outbox. It fuses into the pending
/// write for the same node when one exists, as in a dialer's
/// set-then-evaluate. Otherwise it is standalone, as in a bang.
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

// Broadcast

/// Broadcast the outbox. Evals ship immediately. Pending values ship when
/// their rate-limit window allows. Runs after `VmSet` beside the tip
/// announce.
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

    // Drop entries for sessions that have ended.
    let live = |s: &SessionId| sessions.sessions.contains_key(s);
    outbox.pending.retain(|(s, ..), _| live(s));
    outbox.last_sent.retain(|(s, ..), _| live(s));
    outbox.evals.retain(|(s, ..)| live(s));

    // The anchor is the committed graph addr the action was issued against.
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

    // Pending value batches ship when their window allows. The window is the
    // user-configurable send rate.
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
            // Degrade to the newest value alone rather than lose the window.
            // A large `Str` or `List` value can overflow the envelope even
            // alone. That final drop keeps its warning.
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

/// Encode and broadcast one action. Returns whether it shipped.
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

// Remote application

/// Apply received actions to the matching open heads' VMs.
///
/// Runs after `poll_collab_events` and before `VmSet`, so writes land before
/// evaluation systems observe the frame. The apply path uses
/// `gantz_core::node::state::update_value` and `EvalEntryEvent` directly. It
/// never uses a `NodeCtx` or the payload bus, so nothing here can
/// re-broadcast. The capture and apply channels are disjoint.
///
/// Each action applies only while the local tip holds the identical graph it
/// was issued against. Anchor equality guarantees node-index identity, which
/// makes bare index paths safe. `state::update_value` would create state at
/// any path on a diverged graph. A mismatch is usually a tip in flight, so
/// actions retry briefly before dropping. An action for a just-deleted node
/// expires the same way, because deletion moved the anchor.
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
        // Gossip broadcasts do not self-deliver. Guard anyway.
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
        // No VM runs an ephemeral action for a closed tab.
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
                // Last-write-wins per path on the batch's stamp.
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
                // real node of this graph. The root index bound-check is
                // defense in depth.
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
                // Replay the batch through the command queue. Writes are
                // queued world closures and evals are triggers. FIFO command
                // application interleaves them, so each eval observes its own
                // step's value. A direct write here would land before every
                // deferred eval fired and collapse every step onto the final
                // value.
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
                        // A remote push fires now on this peer's clock, like
                        // a local push.
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

/// Whether the entrypoint's generated entry fn exists in the VM. A
/// config-divergent peer logs one debug line instead of pushing a spurious
/// runtime diagnostic through the eval error path.
fn entry_fn_exists(vm: &steel::steel_vm::engine::Engine, entrypoint: &Entrypoint) -> bool {
    let fn_name = gantz_core::compile::entry_fn_name(&entrypoint.id());
    let exists = vm.extract_value(&fn_name).is_ok();
    if !exists {
        log::debug!("remote eval skipped: entry fn {fn_name} is not compiled locally");
    }
    exists
}

/// Trigger an entrypoint evaluation only if [`entry_fn_exists`]. Returns
/// whether the eval was triggered.
fn trigger_guarded_eval(
    vm: &steel::steel_vm::engine::Engine,
    head: Entity,
    entrypoint: Entrypoint,
    cmds: &mut Commands,
) -> bool {
    if !entry_fn_exists(vm, &entrypoint) {
        return false;
    }
    // A remote push fires now on this peer's clock, like a local push.
    cmds.trigger(EvalEntryEvent {
        head,
        entrypoint,
        time: None,
    });
    true
}

/// A compact display form for a value in log and history lines.
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

/// A compact display form for eval sources in log and history lines.
pub(crate) fn sources_summary(sources: &[Source]) -> String {
    let paths: Vec<String> = sources.iter().map(|s| format!("{:?}", s.path)).collect();
    paths.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::compile::entrypoint::{Entrypoint, EvalKind, EvalSource};

    /// A capture world with one open session head. Returns the world, the
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

    // During a drag, rate limiting can hold an unshipped pending while the
    // next frame's payloads arrive eval-first. The eval fuses into the held
    // pending. The frame's write must keep both the fused eval and the
    // earlier values. Otherwise the flushed `SetState` ships without the
    // eval or with steps missing.
    #[test]
    fn writes_batch_and_keep_the_fused_eval_in_either_order() {
        let (mut world, head, session, name) = capture_world();
        let path = vec![0usize];
        let entrypoint = push_entrypoint(&path);

        // Frame 1. The write and eval fuse into a pending held by the rate
        // limit.
        world.trigger(CaptureWrite {
            head,
            write: write(&path, 1.0),
        });
        world.trigger(CaptureEval {
            head,
            entrypoint: entrypoint.clone(),
        });
        // Frame 2 in the hazardous order. The eval fuses into the held
        // pending before the frame's own write lands.
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

    // The batch bound drops the oldest value, so the final value always
    // ships.
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
