//! Display and configuration types for collaborative sessions (#286).
//!
//! `gantz_egui` renders session UI from these plain types; the networking
//! layer (e.g. `bevy_gantz_collab`) fills them each frame. No network types
//! leak in here, keeping this crate framework- and transport-agnostic.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// User-editable collaboration configuration, persisted with the GUI state.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct CollabConfig {
    /// The username shared with session peers (empty = anonymous).
    #[serde(default)]
    pub username: String,
}

/// Everything the widgets need to render collaboration state.
#[derive(Clone, Debug, Default)]
pub struct CollabUiState {
    /// This user's public identity, as a displayable string.
    pub peer_id: Option<String>,
    /// Active sessions, keyed by the shared graph's name.
    pub sessions: HashMap<gantz_ca::Name, SessionDisplay>,
}

/// One session's displayable state.
#[derive(Clone, Debug, Default)]
pub struct SessionDisplay {
    /// Whether this peer created the session.
    pub is_host: bool,
    /// The connection lifecycle.
    pub conn: SessionConn,
    /// Connected collaborators.
    pub peers: Vec<PeerDisplay>,
    /// The invite string, once minted.
    pub ticket: Option<String>,
    /// Conflicts auto-resolved by the session policy so far.
    pub conflicts: usize,
}

/// A session's connection lifecycle.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SessionConn {
    /// Awaiting the first peer or the join snapshot.
    #[default]
    Connecting,
    /// At least one peer is connected.
    Live,
    /// No peers reachable; local edits continue and re-heal on reconnect.
    Degraded,
}

/// A collaborator as displayed in session UIs.
#[derive(Clone, Debug, Default)]
pub struct PeerDisplay {
    /// A short displayable form of the peer's public key.
    pub id: String,
    /// The peer's self-reported username, if any.
    pub name: Option<String>,
}

impl SessionConn {
    /// The indicator glyph colour for this state.
    pub fn color(&self) -> egui::Color32 {
        match self {
            Self::Connecting => egui::Color32::from_rgb(0xd0, 0xa0, 0x30),
            Self::Live => egui::Color32::from_rgb(0x50, 0xc0, 0x50),
            Self::Degraded => egui::Color32::from_rgb(0xc0, 0x50, 0x50),
        }
    }

    /// A short human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Live => "live",
            Self::Degraded => "offline",
        }
    }
}
