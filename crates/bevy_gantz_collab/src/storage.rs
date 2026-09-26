//! Persistence for the collab session configurations and the vault
//! agreement over the app's key-value storage in `gantz_store`.
//! `gantz_collab::identity` persists the identity.

use gantz_collab::{Session, VaultId};
use gantz_store::{Load, Save, load, save};

/// What each name was last agreed at with a vault, with the vault. See
/// `gantz_collab_sync::VaultLink::synced`.
pub type VaultSynced = (VaultId, gantz_collab_sync::Synced);

/// The key holding the persisted session configurations.
pub const SESSIONS_KEY: &str = "collab-sessions";

/// The key holding the persisted [`VaultSynced`].
const VAULT_SYNCED_KEY: &str = "vault-synced";

/// Persist the session configurations.
pub fn save_sessions(storage: &mut impl Save, sessions: &[Session]) {
    save(storage, SESSIONS_KEY, &sessions);
}

/// Load the persisted session configurations.
pub fn load_sessions(storage: &impl Load) -> Vec<Session> {
    load(storage, SESSIONS_KEY).unwrap_or_default()
}

/// Persist the vault agreement. Write it after the registry in the same
/// batch, so a crash can only leave it behind the registry, never ahead.
pub fn save_vault_synced(storage: &mut impl Save, synced: &VaultSynced) {
    save(storage, VAULT_SYNCED_KEY, synced);
}

/// Load the persisted vault agreement, if any.
pub fn load_vault_synced(storage: &impl Load) -> Option<VaultSynced> {
    load(storage, VAULT_SYNCED_KEY)
}
