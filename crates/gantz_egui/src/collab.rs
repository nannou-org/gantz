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
    /// A custom relay server URL routing this peer's traffic. `None` uses
    /// iroh's default (n0's public relays). Applied when the collab runtime
    /// starts, i.e. a change takes effect on app restart.
    #[serde(default)]
    pub custom_relay: Option<String>,
}

/// Everything the widgets need to render collaboration state.
#[derive(Clone, Debug, Default)]
pub struct CollabUiState {
    /// This user's public identity, as a displayable string.
    pub peer_id: Option<String>,
    /// Active sessions, keyed by the shared graph's name.
    pub sessions: HashMap<gantz_ca::Name, SessionDisplay>,
    /// The endpoint's home relay(s) and their connection state (empty until
    /// the runtime starts).
    pub relays: Vec<(String, bool)>,
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
    /// The most recent session error (e.g. a failed join), cleared once the
    /// session progresses.
    pub error: Option<String>,
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

impl SessionDisplay {
    /// A hover summary: the connection state and connected peers.
    pub fn hover_text(&self) -> String {
        let mut text = format!("shared session: {}", self.conn.label());
        if self.peers.is_empty() {
            text.push_str("\nno peers connected");
        }
        for peer in &self.peers {
            match &peer.name {
                Some(name) => text.push_str(&format!("\n{name} ({})", peer.id)),
                None => text.push_str(&format!("\n{}", peer.id)),
            }
        }
        text
    }
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
