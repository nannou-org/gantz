//! Per-session local state. Plain data with no host types.

use gantz_ca as ca;
use gantz_collab::{ConnState, ObjectRef, PeerId, Session, SessionId};
use std::collections::{BTreeMap, HashMap, HashSet};

/// All local session state, keyed by session id.
#[derive(Default)]
pub struct Sessions {
    pub sessions: HashMap<SessionId, SessionState>,
    /// Set on any local commit. Consumed by [`crate::announce`].
    pub dirty: bool,
    /// The endpoint's home relays and their connection state.
    pub relays: Vec<(String, bool)>,
}

/// One session's local runtime state. It is not persisted.
pub struct SessionState {
    /// The persisted configuration.
    pub session: Session,
    /// The invite string, once the runtime has minted it.
    pub ticket: Option<String>,
    /// The connection lifecycle, for a host's indicator.
    pub conn: ConnState,
    /// Connected peers and their self-reported usernames.
    pub peers: BTreeMap<PeerId, Option<String>>,
    /// The tip most recently announced or adopted per scoped name, for echo
    /// suppression.
    pub last_announced: HashMap<ca::Name, ca::CommitAddr>,
    /// The per-origin gossip sequence number.
    pub seq: u64,
    /// In-flight fetches, per scoped name.
    pub pending: HashMap<ca::Name, PendingTip>,
    /// Auto-resolved conflicts since the session started.
    pub conflicts: usize,
    /// The most recent session error, cleared once the session progresses.
    pub error: Option<String>,
    /// The empty-graph commit minted at join time so the shared name exists
    /// immediately. The snapshot adopts over it.
    pub placeholder: Option<ca::CommitAddr>,
    /// Commits already mirrored into the runtime-owned served store, so
    /// `serve_scope`'s updates stay incremental without reading it back.
    pub served_commits: HashSet<ca::CommitAddr>,
    /// Graphs already mirrored into the served store.
    pub served_graphs: HashSet<ca::GraphAddr>,
    /// Blobs already mirrored into the served store.
    pub served_blobs: HashSet<(ca::SectionId, ca::ContentAddr)>,
    /// Section entries already mirrored into the served store. Entries are
    /// recorded only once sent, so metadata seeded after its subject is
    /// caught by a later pass.
    pub served_sections: HashSet<(ca::SectionId, ca::Key)>,
    /// The served name to tip map as last mirrored.
    pub served_heads: HashMap<ca::Name, ca::CommitAddr>,
    /// Peers' live pointers over the session's shared graph, keyed by
    /// origin. Entries persist through `pos: None`, so reordered stale
    /// updates still drop by `seq`. Freshness is filtered at display time.
    pub pointers: HashMap<PeerId, PeerPointer>,
}

/// One peer's last-known pointer state from `GossipMsg::Pointer`.
pub struct PeerPointer {
    /// Graph-space position. `None` means the pointer left the scene.
    pub pos: Option<(f32, f32)>,
    /// The origin's latest sequence number, for stale-drop.
    pub seq: u64,
    /// When the update arrived, for display-time expiry.
    pub at: web_time::Instant,
}

/// An announced tip whose closure is still being fetched.
pub struct PendingTip {
    pub tip: ca::CommitAddr,
    pub from: PeerId,
    pub staged: ca::sync::Staged,
    /// The previous want, to detect a peer that cannot make progress.
    pub last_want: Option<Vec<ObjectRef>>,
}

impl SessionState {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            ticket: None,
            conn: ConnState::default(),
            peers: BTreeMap::new(),
            last_announced: HashMap::new(),
            seq: 0,
            pending: HashMap::new(),
            conflicts: 0,
            error: None,
            placeholder: None,
            served_commits: HashSet::new(),
            served_graphs: HashSet::new(),
            served_blobs: HashSet::new(),
            served_sections: HashSet::new(),
            served_heads: HashMap::new(),
            pointers: HashMap::new(),
        }
    }

    /// The session's shared branch as a structured name.
    pub fn branch_name(&self) -> ca::Name {
        self.session.branch.parse().expect("names parse infallibly")
    }
}
