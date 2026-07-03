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
//! same frame (atomic set-then-evaluate remotely), keeps only the latest
//! value per node path (a drag coalesces), and rate-limits value sends per
//! path - the final drag value always ships on the trailing edge. Evals
//! bypass the limit: every click ships.

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

/// The minimum interval between value sends for one node path: a 60 Hz drag
/// becomes ~20 msg/s. The pending slot keeps the newest value meanwhile and
/// flushes when the window elapses, so the final value always ships.
const RATE_LIMIT: Duration = Duration::from_millis(50);

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

/// A fused, not-yet-sent state write.
struct PendingWrite {
    value: Value,
    /// The push-eval fused with this write, when the node triggered one in
    /// the same frame.
    eval: Option<Source>,
}

/// Outbound ephemeral actions: per-path fusion, coalescing and rate state.
#[derive(Default, Resource)]
pub struct ActionOutbox {
    /// The newest unsent value per node path (latest wins during a drag).
    pending: HashMap<PathKey, PendingWrite>,
    /// When each path last shipped, for rate limiting.
    last_sent: HashMap<PathKey, Instant>,
    /// Standalone evals; all flushed every pass.
    evals: Vec<(SessionId, ca::Name, Vec<Source>)>,
    /// Per-session action sequence counters.
    seq: HashMap<SessionId, u64>,
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
    // A newer write RESETS any fused eval (a write without push-eval must
    // not re-fire a stale one)...
    let mut eval = None;
    // ...unless this pass already queued a standalone push-eval for the same
    // node, in which case it belongs to this write: fuse it back in.
    if let Some(ix) = outbox.evals.iter().position(|(s, n, sources)| {
        *s == session
            && *n == name
            && matches!(&sources[..], [src] if src.path == ev.write.path
                && src.kind == gantz_egui::action::Kind::Push)
    }) {
        let (_, _, mut sources) = outbox.evals.remove(ix);
        eval = sources.pop();
    }
    let key = (session, name, ev.write.path.clone());
    outbox.pending.insert(
        key,
        PendingWrite {
            value: ev.write.value.clone(),
            eval,
        },
    );
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

    // Pending values ship when their window allows.
    let now = Instant::now();
    let due: Vec<PathKey> = outbox
        .pending
        .keys()
        .filter(|key| {
            outbox
                .last_sent
                .get(*key)
                .is_none_or(|&at| now.duration_since(at) >= RATE_LIMIT)
        })
        .cloned()
        .collect();
    for key in due {
        let Some(PendingWrite { value, eval }) = outbox.pending.remove(&key) else {
            continue;
        };
        let (session, name, path) = key.clone();
        let Some(graph) = anchor(&name) else {
            continue;
        };
        let summary = format!(
            "set {path:?} = {}{}",
            value_summary(&value),
            if eval.is_some() { " +eval" } else { "" },
        );
        let action = Action::SetState { path, value, eval };
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
