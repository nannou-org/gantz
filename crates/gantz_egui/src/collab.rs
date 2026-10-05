//! Display and configuration types for collaborative sessions.
//!
//! `gantz_egui` renders session UI from these plain types. The networking
//! layer fills them each frame. `bevy_gantz_collab` is one such layer. No
//! network types leak in here, which keeps this crate framework-agnostic and
//! transport-agnostic.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// User-editable collaboration configuration, persisted with the GUI state.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct CollabConfig {
    /// The username shared with session peers. An empty string means
    /// anonymous.
    #[serde(default)]
    pub username: String,
    /// A custom relay server URL routing this peer's traffic. `None` uses
    /// iroh's default, the n0 public relays. Applied when the collab runtime
    /// starts, so a change takes effect on app restart.
    #[serde(default)]
    pub custom_relay: Option<String>,
    /// The minimum interval in milliseconds between live-action sends per
    /// node path. `0` sends every frame. Values written faster than this
    /// batch into one message and replay in order on peers. See the
    /// `bevy_gantz_collab::action` module docs.
    #[serde(default = "default_action_rate_ms")]
    pub action_rate_ms: u64,
    /// Whether to show session peers' live pointers over shared graphs.
    /// Display-side only. This peer's own pointer broadcasts regardless.
    #[serde(default = "default_true")]
    pub show_pointers: bool,
    /// The ticket of the vault this device syncs all its named graphs with.
    /// `None` leaves the device unlinked. It carries the vault's pairing
    /// secret.
    #[serde(default)]
    pub vault: Option<String>,
}

impl Default for CollabConfig {
    fn default() -> Self {
        Self {
            username: String::new(),
            custom_relay: None,
            action_rate_ms: default_action_rate_ms(),
            show_pointers: true,
            vault: None,
        }
    }
}

fn default_true() -> bool {
    true
}

/// The [`CollabConfig::action_rate_ms`] default. 16ms sends roughly every
/// frame at 60 Hz, which keeps peer interaction smooth on fast links. Raise
/// it to cut the message rate on slow links. Values batch either way, so no
/// step is ever lost.
fn default_action_rate_ms() -> u64 {
    16
}

/// A line counting `n` graphs, with what is true of one graph or of many.
/// `None` for no graphs.
fn graphs(n: usize, one: &str, many: &str) -> Option<String> {
    match n {
        0 => None,
        1 => Some(format!("1 graph {one}")),
        n => Some(format!("{n} graphs {many}")),
    }
}

/// Everything the widgets need to render collaboration state.
#[derive(Clone, Debug, Default)]
pub struct CollabUiState {
    /// This user's public identity, as a displayable string.
    pub peer_id: Option<String>,
    /// Active sessions, keyed by the shared graph's name.
    pub sessions: HashMap<gantz_ca::Name, SessionDisplay>,
    /// The endpoint's home relays and their connection state. Empty until
    /// the runtime starts.
    pub relays: Vec<(String, bool)>,
    /// The vault link, while this device is linked to a vault.
    pub vault: Option<VaultDisplay>,
}

/// The vault link's displayable state.
#[derive(Clone, Debug, Default)]
pub struct VaultDisplay {
    /// A short displayable form of the vault's identity.
    pub vault: String,
    pub state: VaultState,
    /// The vault's app and protocols, such as `gantz 0.4.0, protocol 2`,
    /// once the vault has answered.
    pub vault_app: Option<String>,
    /// This app and its protocols, in the same form as `vault_app`.
    pub this_app: String,
    /// Each name that failed to sync, and why.
    pub failures: Vec<(String, String)>,
    /// The names that the vault synced from a newer gantz, whose graphs hold
    /// settings that this gantz locks.
    pub newer: Vec<String>,
}

/// The vault link's state, for display.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum VaultState {
    /// Waiting for the vault's first answer.
    #[default]
    Connecting,
    /// Linked, and names sync.
    Live,
    /// The vault cannot be reached, for this reason. The link retries by
    /// itself.
    Offline(String),
    /// The vault refused this device, for this reason.
    Denied(String),
    /// The vault only speaks older sync protocols.
    VaultOutdated,
    /// The vault only speaks newer sync protocols.
    DeviceOutdated,
    /// This device did not link, for this reason. For example, the ticket
    /// cannot be read.
    NotLinked(String),
}

/// One session's displayable state.
#[derive(Clone, Debug, Default)]
pub struct SessionDisplay {
    /// Whether this peer created the session.
    pub is_host: bool,
    /// The connection lifecycle.
    pub conn: SessionConn,
    /// Whether the join is still showing its empty placeholder graph because
    /// the initial snapshot has not yet arrived. Drives the connecting
    /// overlay. The overlay outlives `conn` reaching [`SessionConn::Live`],
    /// since a peer connects before the graph finishes syncing. Always false
    /// for a host, which never shows a placeholder.
    pub awaiting_snapshot: bool,
    /// Connected collaborators.
    pub peers: Vec<PeerDisplay>,
    /// The invite string, once minted.
    pub ticket: Option<String>,
    /// Conflicts auto-resolved by the session policy so far.
    pub conflicts: usize,
    /// The most recent session error, cleared once the session progresses. A
    /// failed join is one example.
    pub error: Option<String>,
    /// Peers' live pointer positions over this session's graph. The
    /// networking layer fills them with expiry applied.
    pub pointers: Vec<PointerDisplay>,
}

/// One peer's live pointer over a shared graph, ready to render.
#[derive(Clone, Debug)]
pub struct PointerDisplay {
    /// The pointer position in graph-space coordinates. The scene maps them
    /// through the head's camera.
    pub pos: egui::Pos2,
    /// The peer's username or short id.
    pub label: String,
    /// A stable per-peer colour derived from the peer's identity.
    pub color: egui::Color32,
}

impl PointerDisplay {
    /// Build a display pointer from raw parts. The parts are a graph-space
    /// position, the peer's label and the full identity bytes for the stable
    /// colour. Networking layers need no egui types.
    pub fn new(pos: (f32, f32), label: String, peer: &[u8; 32]) -> Self {
        Self {
            pos: egui::pos2(pos.0, pos.1),
            label,
            color: peer_color(peer),
        }
    }
}

/// A stable per-peer colour. The hue derives deterministically from the
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
    /// No peers reachable. Local edits continue and re-heal on reconnect.
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

    /// A hover summary of the connection state and connected peers.
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

impl VaultDisplay {
    /// A hover summary of the link state, its reason and the names that
    /// failed to sync.
    pub fn hover_text(&self) -> String {
        let mut text = format!("vault {}: {}", self.vault, self.state.guidance());
        if let Some(reason) = self.state.reason() {
            text.push_str(&format!("\n{reason}"));
        }
        let failed = graphs(self.failures.len(), "cannot sync", "cannot sync");
        let newer = graphs(
            self.newer.len(),
            "holds settings from a newer gantz",
            "hold settings from a newer gantz",
        );
        for line in [failed, newer].into_iter().flatten() {
            text.push_str(&format!("\n{line}"));
        }
        text
    }

    /// Both sides' app and protocols.
    pub fn versions(&self) -> String {
        let vault = self.vault_app.as_deref().unwrap_or("not heard yet");
        format!("vault: {vault}\nthis app: {}", self.this_app)
    }
}

impl VaultState {
    /// The indicator glyph colour for this state. States that only the user
    /// can fix share one colour.
    pub fn color(&self) -> egui::Color32 {
        match self {
            Self::Connecting => SessionConn::Connecting.color(),
            Self::Live => SessionConn::Live.color(),
            Self::Offline(_) => SessionConn::Degraded.color(),
            _ => egui::Color32::from_rgb(0xb0, 0x70, 0xe0),
        }
    }

    /// Whether only the user can fix the link, with an update or a new
    /// ticket.
    pub fn needs_action(&self) -> bool {
        matches!(
            self,
            Self::Denied(_) | Self::VaultOutdated | Self::DeviceOutdated | Self::NotLinked(_)
        )
    }

    /// What the state means for the user, and what to do about it.
    pub fn guidance(&self) -> &'static str {
        match self {
            Self::Connecting => "Connecting to the vault.",
            Self::Live => "Syncing all named graphs with the vault.",
            Self::Offline(_) => {
                "The vault is offline. Edits stay on this device and sync when it is back."
            }
            Self::Denied(_) => {
                "The vault refused this device. Paste a new vault ticket to pair it again."
            }
            Self::VaultOutdated => {
                "The vault needs an update. Edits stay on this device and sync after it updates."
            }
            Self::DeviceOutdated => {
                "Update gantz on this device to sync. Edits stay on this device until then."
            }
            Self::NotLinked(_) => "This device is not linked to the vault.",
        }
    }

    /// The reason behind the state, if it has one.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Offline(reason) | Self::Denied(reason) | Self::NotLinked(reason) => Some(reason),
            _ => None,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_hover_tells_the_state_its_reason_and_the_failures() {
        let vault = VaultDisplay {
            vault: "2af6d9e8".to_string(),
            state: VaultState::Offline("connect failed".to_string()),
            vault_app: None,
            this_app: "gantz 0.4.0, protocol 2".to_string(),
            failures: vec![("jam".to_string(), "push failed".to_string())],
            newer: vec![],
        };
        let hover = vault.hover_text();
        assert!(hover.starts_with("vault 2af6d9e8: The vault is offline."));
        assert!(hover.contains("\nconnect failed"));
        assert!(hover.ends_with("\n1 graph cannot sync"));
        let versions = vault.versions();
        assert_eq!(
            versions,
            "vault: not heard yet\nthis app: gantz 0.4.0, protocol 2"
        );
    }

    #[test]
    fn only_updates_and_new_tickets_need_the_user() {
        let offline = VaultState::Offline(String::new());
        let denied = VaultState::Denied(String::new());
        assert!(!offline.needs_action());
        assert!(denied.needs_action());
        assert!(VaultState::VaultOutdated.needs_action());
        assert!(VaultState::DeviceOutdated.needs_action());
        assert_ne!(offline.color(), denied.color());
        assert_eq!(denied.color(), VaultState::VaultOutdated.color());
    }
}
