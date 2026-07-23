//! Persistence for the collab identity and session configurations, over the
//! app's key-value storage (see `bevy_gantz::storage`).

use bevy_gantz::storage::{Load, Save, load, save};
use gantz_collab::{Identity, Session};

/// The key holding the user's secret identity bytes.
pub const IDENTITY_KEY: &str = "collab-identity";

/// The key holding the persisted session configurations.
pub const SESSIONS_KEY: &str = "collab-sessions";

/// Persist the identity's secret bytes.
pub fn save_identity(storage: &mut impl Save, identity: &Identity) {
    save(storage, IDENTITY_KEY, &identity.to_bytes());
}

/// Load the persisted identity, if any.
pub fn load_identity(storage: &impl Load) -> Option<Identity> {
    load::<[u8; 32]>(storage, IDENTITY_KEY).map(Identity::from_bytes)
}

/// Persist the session configurations.
pub fn save_sessions(storage: &mut impl Save, sessions: &[Session]) {
    save(storage, SESSIONS_KEY, &sessions);
}

/// Load the persisted session configurations.
pub fn load_sessions(storage: &impl Load) -> Vec<Session> {
    load(storage, SESSIONS_KEY).unwrap_or_default()
}
