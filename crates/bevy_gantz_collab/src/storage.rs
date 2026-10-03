//! Persistence for the collab session configurations over the app's
//! key-value storage in `gantz_store`. `gantz_collab::identity` persists the
//! identity.

use gantz_collab::Session;
use gantz_store::{Load, Save, load, save};

/// The key holding the persisted session configurations.
pub const SESSIONS_KEY: &str = "collab-sessions";

/// Persist the session configurations.
pub fn save_sessions(storage: &mut impl Save, sessions: &[Session]) {
    save(storage, SESSIONS_KEY, &sessions);
}

/// Load the persisted session configurations.
pub fn load_sessions(storage: &impl Load) -> Vec<Session> {
    load(storage, SESSIONS_KEY).unwrap_or_default()
}
