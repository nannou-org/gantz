//! Session lifecycle: sharing, joining and leaving, plus the headless
//! reference resync that follows a moved name.

use crate::{SessionState, Sessions};
use gantz_ca as ca;
use gantz_collab::{
    Access, Command, Handle, Infra, PROTO_VERSION, Role, Session, SessionEntry, SessionId,
    SessionRegistry, SessionTicket,
};
use std::fmt;

/// Why a ticket could not be joined.
#[derive(Debug)]
pub enum JoinError {
    /// The ticket string did not parse.
    Ticket(String),
    /// The ticket was minted by an incompatible protocol version.
    Proto { ticket: u32, ours: u32 },
    /// The runtime's command channel is closed.
    RuntimeGone,
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ticket(e) => write!(f, "invalid ticket: {e}"),
            Self::Proto { ticket, ours } => {
                write!(
                    f,
                    "protocol mismatch (ticket v{ticket}, this build v{ours})"
                )
            }
            Self::RuntimeGone => write!(f, "collab runtime is gone"),
        }
    }
}

impl std::error::Error for JoinError {}

/// The fixed conflict policy for shared sessions. The last edit wins and
/// edits beat deletes. It is symmetric, so independently merging peers
/// converge.
pub fn session_resolutions() -> ca::merge::Resolutions {
    ca::merge::Resolutions {
        both_modified: ca::merge::BothModified::KeepNewest,
        delete_modify: ca::merge::EditOrDelete::KeepEdit,
    }
}

/// The runtime infrastructure for an optional custom relay.
///
/// A custom relay means self-hosted infrastructure with nothing from n0.
/// Peers reach each other via invite-ticket addresses and the relay itself,
/// so no address-lookup service is required.
pub fn infra(custom_relay: Option<&str>) -> Infra {
    match custom_relay {
        Some(url) => Infra::Custom {
            relays: vec![url.to_string()],
            pkarr: None,
        },
        None => Infra::N0,
    }
}

/// Mint a session for `branch`, fill its served store and start gossiping.
/// Returns the new session's id.
pub fn share(
    sessions: &mut Sessions,
    registry: &ca::Registry,
    handle: &Handle,
    branch: &ca::Name,
    access: Access,
) -> SessionId {
    let session = Session {
        id: SessionId::generate(),
        branch: branch.to_string(),
        access,
        resolutions: session_resolutions(),
        role: Role::Host,
    };
    let id = session.id;
    let scope = gantz_egui::sync::session_scope(registry, branch);
    let mut state = SessionState::new(session.clone());
    let _ = handle.cmds.try_send(Command::Register(SessionEntry {
        session,
        store: SessionRegistry::default(),
    }));
    crate::serve_scope(handle, &mut state, registry, &scope);
    state.last_announced = state.served_heads.clone();
    sessions.sessions.insert(id, state);
    if handle.cmds.try_send(Command::Share(id)).is_err() {
        log::error!("share: collab runtime is gone");
    }
    id
}

/// Parse the ticket and ask the runtime to join. The snapshot lands via
/// [`crate::poll`].
///
/// When the branch is unknown locally, an empty placeholder graph is minted
/// for it and recorded, so the snapshot adopts over it rather than renaming
/// it aside. An existing local graph reconciles when the snapshot lands.
/// Returns the session id and the shared branch.
pub fn join(
    sessions: &mut Sessions,
    registry: &mut ca::Registry,
    handle: &Handle,
    ticket: &str,
    now: ca::Timestamp,
) -> Result<(SessionId, ca::Name), JoinError> {
    let ticket: SessionTicket = ticket
        .trim()
        .parse()
        .map_err(|e| JoinError::Ticket(format!("{e}")))?;
    if ticket.proto != PROTO_VERSION {
        return Err(JoinError::Proto {
            ticket: ticket.proto,
            ours: PROTO_VERSION,
        });
    }
    let session = Session {
        id: ticket.session,
        branch: ticket.name.clone(),
        access: ticket.access.clone(),
        resolutions: ticket.resolutions,
        role: Role::Guest,
    };
    let id = session.id;
    let _ = handle.cmds.try_send(Command::Register(SessionEntry {
        session: session.clone(),
        store: SessionRegistry::default(),
    }));
    let mut state = SessionState::new(session);
    let branch: ca::Name = ticket.name.parse().expect("names parse infallibly");
    if registry.head(&branch).is_none() {
        let graph = ca::DataGraph::default();
        let graph_ca = ca::graph_addr(&graph);
        let placeholder = registry.commit_graph(now, None, graph_ca, || graph);
        registry.set_head(branch.clone(), placeholder);
        state.placeholder = Some(placeholder);
    }
    sessions.sessions.insert(id, state);
    handle
        .cmds
        .try_send(Command::Join(ticket))
        .map_err(|_| JoinError::RuntimeGone)?;
    Ok((id, branch))
}

/// Stop gossiping and forget the session.
pub fn leave(sessions: &mut Sessions, handle: &Handle, id: SessionId) {
    sessions.sessions.remove(&id);
    let _ = handle.cmds.try_send(Command::Leave(id));
    let _ = handle.cmds.try_send(Command::Forget(id));
}

/// Resync named references after names moved, carrying each moved graph's
/// view forward to its new commit. For hosts with no open heads to refresh.
pub fn resync_headless(
    registry: &mut ca::Registry,
    now: ca::Timestamp,
) -> Vec<gantz_egui::sync::Moved> {
    let moves = gantz_egui::sync::resync(registry, now);
    for m in &moves {
        if gantz_egui::section::view(registry, &m.new_commit).is_none() {
            if let Some(view) = gantz_egui::section::view(registry, &m.old_commit) {
                gantz_egui::section::set_view(registry, m.new_commit, &view);
            }
        }
    }
    moves
}
