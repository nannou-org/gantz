//! Linking the app to a vault. See `gantz_collab_sync::vault`.
//!
//! The persisted [`gantz_egui::collab::CollabConfig::vault`] ticket is the
//! desired link. [`sync_vault_link`] links, relinks or unlinks to match it.
//! The link then syncs through the usual `poll_collab_events`.
//!
//! While linked, open named heads outside the base names take the
//! [`bevy_gantz_egui::SessionHead`] marker, so undo mints forward reverts
//! that sync like any edit.
//!
//! The app persists [`PersistedVaultSynced`] after the registry whenever it
//! changes.

use crate::storage::VaultSynced;
use crate::{
    AppVersion, CollabIdentity, CollabRuntime, CollabSessions, SessionRef, session::ensure_runtime,
};
use bevy_ecs::prelude::*;
use bevy_gantz::head;
use bevy_gantz_egui::SessionHead;
use gantz_ca as ca;
use gantz_collab::VaultTicket;

/// The vault agreement as persisted. The app loads it at startup, and it
/// follows the link from then on. It seeds a link to the vault it names.
#[derive(Default, Resource)]
pub struct PersistedVaultSynced(pub Option<VaultSynced>);

/// The vault link as made, for comparing with the desired ticket.
#[derive(Default, Resource)]
pub struct VaultLinkState {
    /// The ticket of the current link, or of the last failed attempt.
    ticket: Option<String>,
    /// Why the last attempt to link failed.
    pub error: Option<String>,
}

/// Link, relink or unlink to match the configured vault ticket.
///
/// A store opened read-only does not link. Its edits are not saved, and a
/// vault would spread them.
pub(crate) fn sync_vault_link(
    mut runtime: ResMut<CollabRuntime>,
    identity: Option<Res<CollabIdentity>>,
    app: Res<AppVersion>,
    mut sessions: ResMut<CollabSessions>,
    mut state: ResMut<VaultLinkState>,
    persisted: Res<PersistedVaultSynced>,
    base_names: Res<bevy_gantz_egui::BaseNames>,
    gui_state: Res<bevy_gantz_egui::GuiState>,
    read_only: Option<Res<bevy_gantz::storage::StoreReadOnly>>,
) {
    let desired = gui_state.0.collab.vault.as_ref();
    if state.ticket.as_ref() == desired {
        return;
    }
    let Some(identity) = identity else {
        return;
    };
    if let Some(handle) = runtime.0.as_ref() {
        gantz_collab_sync::vault::unlink(&mut sessions.0, handle);
    }
    state.ticket = desired.cloned();
    state.error = None;
    let Some(ticket) = desired else {
        return;
    };
    if read_only.is_some() {
        state.error = Some("this gantz opened its store read-only".to_string());
        return;
    }
    let ticket = match ticket.trim().parse::<VaultTicket>() {
        Ok(ticket) => ticket,
        Err(e) => {
            state.error = Some(format!("invalid ticket: {e}"));
            return;
        }
    };
    let handle = ensure_runtime(&mut runtime, &identity.0, &app, &gui_state.0.collab);
    let synced = persisted
        .0
        .as_ref()
        .filter(|(id, _)| *id == ticket.vault)
        .map(|(_, synced)| synced.clone())
        .unwrap_or_default();
    let local_only = base_names.0.keys().cloned().collect();
    if let Err(e) =
        gantz_collab_sync::vault::link(&mut sessions.0, handle, ticket, synced, local_only)
    {
        state.error = Some(e.to_string());
    }
}

/// Dispatch the [`gantz_egui::CheckVault`] payload. Forgetting the link as
/// made makes [`sync_vault_link`] link again on its next run.
pub(crate) fn dispatch_check_vault(
    _entity: Option<Entity>,
    _payload: gantz_egui::DynResponse,
    cmds: &mut Commands,
) {
    cmds.queue(|world: &mut World| world.resource_mut::<VaultLinkState>().ticket = None);
}

/// Follow the link's agreement into [`PersistedVaultSynced`]. The resource
/// changes only when the agreement does, which tells the app to persist it.
pub(crate) fn track_vault_synced(
    mut sessions: ResMut<CollabSessions>,
    mut persisted: ResMut<PersistedVaultSynced>,
) {
    let Some(link) = sessions.vault.as_mut().filter(|l| l.synced_changed) else {
        return;
    };
    link.synced_changed = false;
    persisted.0 = Some((link.id, link.synced.clone()));
}

/// Keep [`SessionHead`] on exactly the open heads that sync, through a
/// session or the vault.
pub(crate) fn mark_vault_heads(
    sessions: Res<CollabSessions>,
    heads: Query<(Entity, &head::HeadRef, Has<SessionHead>, Has<SessionRef>), With<head::OpenHead>>,
    mut cmds: Commands,
) {
    let link = sessions.vault.as_ref();
    for (entity, head_ref, session_head, in_session) in &heads {
        let syncs = match (&head_ref.0, link) {
            (ca::Head::Branch(name), Some(link)) => !link.local_only.contains(name),
            _ => false,
        };
        match (in_session || syncs, session_head) {
            (true, false) => {
                cmds.entity(entity).insert(SessionHead);
            }
            (false, true) => {
                cmds.entity(entity).remove::<SessionHead>();
            }
            _ => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{JoinSessionEvent, session::on_join_session};
    use bevy_ecs::system::RunSystemOnce;
    use gantz_collab::{Handle, Identity, PairingSecret, SessionId};
    use std::collections::{BTreeMap, BTreeSet};

    /// A runtime stand-in, so no test binds a socket. Keep the receiver
    /// alive, or the handle counts as closed.
    fn fake() -> (Handle, async_channel::Receiver<gantz_collab::Command>) {
        let (cmds, cmd_rx) = async_channel::unbounded();
        let (_events_tx, events) = async_channel::unbounded();
        (Handle { cmds, events }, cmd_rx)
    }

    fn ticket() -> VaultTicket {
        let peer = Identity::generate().peer_id();
        let host = iroh::EndpointId::from_bytes(&peer.0).unwrap();
        VaultTicket {
            vault: SessionId::generate(),
            pairing: PairingSecret::generate(),
            host: host.into(),
        }
    }

    fn name(s: &str) -> ca::Name {
        s.parse().unwrap()
    }

    fn branch(world: &mut World, n: &str) -> Entity {
        let head = head::HeadRef(ca::Head::Branch(name(n)));
        world.spawn((head::OpenHead, head)).id()
    }

    #[test]
    fn synced_heads_take_session_heads_until_unlinked() {
        let mut world = World::new();
        let (handle, _cmds) = fake();
        let mut sessions = CollabSessions::default();
        let local_only = BTreeSet::from([name("base")]);
        gantz_collab_sync::vault::link(
            &mut sessions.0,
            &handle,
            ticket(),
            Default::default(),
            local_only,
        )
        .unwrap();
        world.insert_resource(sessions);
        let user = branch(&mut world, "jam");
        let base = branch(&mut world, "base");
        let shared = branch(&mut world, "riff");
        world
            .entity_mut(shared)
            .insert((SessionRef(SessionId::generate()), SessionHead));
        world.run_system_once(mark_vault_heads).unwrap();
        let has = |world: &World, e: Entity| world.entity(e).contains::<SessionHead>();
        assert!(has(&world, user));
        assert!(!has(&world, base));
        assert!(has(&world, shared));

        let mut sessions = world.resource_mut::<CollabSessions>();
        gantz_collab_sync::vault::unlink(&mut sessions.0, &handle);
        world.run_system_once(mark_vault_heads).unwrap();
        assert!(!has(&world, user));
        // A session still needs its marker.
        assert!(has(&world, shared));
    }

    #[test]
    fn a_changed_agreement_is_tracked_for_persistence() {
        let mut world = World::new();
        let (handle, _cmds) = fake();
        let mut sessions = CollabSessions::default();
        let id = gantz_collab_sync::vault::link(
            &mut sessions.0,
            &handle,
            ticket(),
            Default::default(),
            BTreeSet::new(),
        )
        .unwrap();
        let tip = ca::CommitAddr::from(ca::ContentAddr::from([1; 32]));
        let link = sessions.vault.as_mut().unwrap();
        link.synced.heads.insert(name("jam"), tip);
        link.synced_changed = true;
        world.insert_resource(sessions);
        world.init_resource::<PersistedVaultSynced>();
        world.run_system_once(track_vault_synced).unwrap();
        let persisted = &world.resource::<PersistedVaultSynced>().0;
        let heads = BTreeMap::from([(name("jam"), tip)]);
        let synced = gantz_collab_sync::Synced {
            heads,
            metas: BTreeMap::new(),
        };
        assert_eq!(persisted, &Some((id, synced)));
        let link = world.resource::<CollabSessions>().vault.as_ref().unwrap();
        assert!(!link.synced_changed);
    }

    #[test]
    fn a_vault_ticket_pasted_to_join_becomes_the_configured_vault() {
        let mut world = World::new();
        world.init_resource::<CollabRuntime>();
        world.init_resource::<CollabSessions>();
        world.init_resource::<bevy_gantz::reg::Registry>();
        world.insert_resource(AppVersion("gantz test".to_string()));
        world.init_resource::<bevy_gantz_egui::GuiState>();
        world.add_observer(on_join_session);
        let ticket = ticket().to_string();
        world.trigger(JoinSessionEvent {
            ticket: format!(" {ticket} "),
        });
        world.flush();
        let config = &world.resource::<bevy_gantz_egui::GuiState>().0.collab;
        assert_eq!(config.vault.as_ref(), Some(&ticket));
        assert!(world.resource::<CollabRuntime>().0.is_none());
    }

    #[test]
    fn a_read_only_store_never_links() {
        let mut world = World::new();
        world.init_resource::<CollabRuntime>();
        world.init_resource::<CollabSessions>();
        world.init_resource::<VaultLinkState>();
        world.init_resource::<PersistedVaultSynced>();
        world.init_resource::<bevy_gantz_egui::BaseNames>();
        world.insert_resource(CollabIdentity(Identity::generate()));
        world.insert_resource(AppVersion("gantz test".to_string()));
        let reason = "a newer gantz wrote the store".to_string();
        world.insert_resource(bevy_gantz::storage::StoreReadOnly(reason));
        let mut gui_state = bevy_gantz_egui::GuiState::default();
        gui_state.0.collab.vault = Some(ticket().to_string());
        world.insert_resource(gui_state);
        world.run_system_once(sync_vault_link).unwrap();
        let state = world.resource::<VaultLinkState>();
        assert_eq!(
            state.error.as_deref(),
            Some("this gantz opened its store read-only")
        );
        assert!(world.resource::<CollabSessions>().vault.is_none());
        assert!(world.resource::<CollabRuntime>().0.is_none());
    }

    #[test]
    fn an_unreadable_ticket_starts_no_runtime() {
        let mut world = World::new();
        world.init_resource::<CollabRuntime>();
        world.init_resource::<CollabSessions>();
        world.init_resource::<VaultLinkState>();
        world.init_resource::<PersistedVaultSynced>();
        world.init_resource::<bevy_gantz_egui::BaseNames>();
        world.insert_resource(CollabIdentity(Identity::generate()));
        world.insert_resource(AppVersion("gantz test".to_string()));
        let mut gui_state = bevy_gantz_egui::GuiState::default();
        gui_state.0.collab.vault = Some("gantzvaultnotaticket".to_string());
        world.insert_resource(gui_state);
        world.run_system_once(sync_vault_link).unwrap();
        let error = world.resource::<VaultLinkState>().error.clone();
        assert!(error.is_some_and(|e| e.starts_with("invalid ticket")));
        assert!(world.resource::<CollabRuntime>().0.is_none());
    }
}
