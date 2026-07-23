//! Display and configuration types for collaborative sessions (#286).
//!
//! `gantz_egui` renders session UI from these plain types; the networking
//! layer (e.g. `bevy_gantz_collab`) fills them each frame. No network types
//! leak in here, keeping this crate framework- and transport-agnostic.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// User-editable collaboration configuration, persisted with the GUI state.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct CollabConfig {
    /// The username shared with session peers (empty = anonymous).
    #[serde(default)]
    pub username: String,
    /// A custom relay server URL routing this peer's traffic. `None` uses
    /// iroh's default (n0's public relays). Applied when the collab runtime
    /// starts, i.e. a change takes effect on app restart.
    #[serde(default)]
    pub custom_relay: Option<String>,
    /// The minimum interval in milliseconds between live-action sends per
    /// node path (0 = every frame). Values written faster than this batch
    /// into one message and replay in order on peers (see the
    /// `bevy_gantz_collab::action` module docs).
    #[serde(default = "default_action_rate_ms")]
    pub action_rate_ms: u64,
    /// Whether to show session peers' live pointers over shared graphs.
    /// Display-side only: this peer's own pointer broadcasts regardless.
    #[serde(default = "default_true")]
    pub show_pointers: bool,
}

impl Default for CollabConfig {
    fn default() -> Self {
        Self {
            username: String::new(),
            custom_relay: None,
            action_rate_ms: default_action_rate_ms(),
            show_pointers: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// The [`CollabConfig::action_rate_ms`] default: ~16ms sends roughly every
/// frame at 60 Hz, keeping peer interaction smooth on fast (e.g. LAN)
/// links. Raise it to cut the message rate on slow links - values batch
/// either way, so no step is ever lost.
fn default_action_rate_ms() -> u64 {
    16
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
    /// Whether the join is still showing its empty placeholder graph, i.e. the
    /// initial snapshot has not yet arrived. Drives the connecting overlay,
    /// which outlives `conn` reaching [`SessionConn::Live`] (a peer connects
    /// before the graph finishes syncing). Always false for a host, which
    /// never shows a placeholder.
    pub awaiting_snapshot: bool,
    /// Connected collaborators.
    pub peers: Vec<PeerDisplay>,
    /// The invite string, once minted.
    pub ticket: Option<String>,
    /// Conflicts auto-resolved by the session policy so far.
    pub conflicts: usize,
    /// The most recent session error (e.g. a failed join), cleared once the
    /// session progresses.
    pub error: Option<String>,
    /// Peers' live pointer positions over this session's graph (presence
    /// cursors), filled by the networking layer with expiry applied.
    pub pointers: Vec<PointerDisplay>,
}

/// One peer's live pointer over a shared graph, ready to render.
#[derive(Clone, Debug)]
pub struct PointerDisplay {
    /// The pointer position in graph-space coordinates (the scene maps them
    /// through the head's camera).
    pub pos: egui::Pos2,
    /// The peer's username or short id.
    pub label: String,
    /// A stable per-peer colour (derived from the peer's identity).
    pub color: egui::Color32,
}

impl PointerDisplay {
    /// Build a display pointer from raw parts: a graph-space position, the
    /// peer's label and their full identity bytes (for the stable colour) -
    /// so networking layers need no egui types.
    pub fn new(pos: (f32, f32), label: String, peer: &[u8; 32]) -> Self {
        Self {
            pos: egui::pos2(pos.0, pos.1),
            label,
            color: peer_color(peer),
        }
    }
}

/// A stable per-peer colour: a hue derived deterministically from the
/// peer's identity bytes, so every viewer colours a given peer identically.
pub fn peer_color(peer: &[u8; 32]) -> egui::Color32 {
    let hue = u16::from_le_bytes([peer[0], peer[1]]) as f32 / u16::MAX as f32;
    egui::ecolor::Hsva::new(hue, 0.75, 0.9, 1.0).into()
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
    /// A short line describing what a still-connecting join is waiting on, for
    /// the scene overlay.
    pub fn sync_status(&self) -> String {
        match self.peers.len() {
            0 => "waiting for a peer to connect".to_string(),
            1 => "receiving graph from 1 peer".to_string(),
            n => format!("receiving graph from {n} peers"),
        }
    }

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
