//! `gantz vault`: a headless vault that a user's devices sync all their
//! named graphs with.
//!
//! `serve` holds the registry in the vault directory, in the app's store
//! format, and serves it over the collab runtime. A device pairs once with
//! the printed ticket, then links on every start. See `gantz_collab::vault`
//! and `gantz_collab_sync::vault`.
//!
//! The vault moves a name only when a push is made against its current
//! head. It never merges, prunes or resyncs references, so it keeps every
//! graph's whole history. An accepted push is on disk before the device
//! hears it was accepted.
//!
//! The directory holds:
//!
//! - `bevy_pkv.redb`: the registry, the vault's identity and its config.
//! - `ticket`: the latest link ticket. It pairs devices, so only the owner
//!   may read it.
//! - `lock`: held by whichever vault command runs, so `devices` and
//!   `revoke` cannot race a running `serve`.

use crate::{Conf, DirArgs, RevokeArgs, ServeArgs, VaultArgs, VaultCommand};
use bevy_pkv::PkvStore;
use gantz_ca as ca;
use gantz_collab::{
    Command, Event, Handle, Identity, Infra, Outdated, PairingSecret, PeerId, RuntimeConfig,
    SessionId, VaultEntry,
};
use gantz_collab_sync::PushOutcome;
use gantz_store::{PersistedRegistry, Save};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// The vault's persisted configuration.
#[derive(Deserialize, Serialize)]
pub(crate) struct Config {
    id: SessionId,
    pairing: PairingSecret,
    pub(crate) devices: BTreeSet<PeerId>,
}

/// An open vault directory. Holds the lock while it lives.
pub(crate) struct Dir {
    path: PathBuf,
    store: PkvStore,
    _lock: File,
}

/// A running vault: its directory, runtime and registry.
pub struct Vault {
    dir: Dir,
    handle: Handle,
    pub(crate) config: Config,
    pub(crate) registry: ca::Registry,
    persisted: PersistedRegistry,
    /// The latest link ticket.
    pub(crate) ticket: Option<String>,
}

/// A [`Save`] that keeps the first failure, since the storage helpers only
/// log theirs.
struct Checked<'a> {
    store: &'a mut PkvStore,
    error: Option<String>,
}

/// The UDP port a vault binds unless told otherwise. A fixed port keeps the
/// addresses in its tickets valid across restarts.
pub const DEFAULT_PORT: u16 = 7447;

/// The store key holding the vault's [`Config`].
const CONFIG_KEY: &str = "vault";

/// The file holding the latest link ticket.
const TICKET_FILE: &str = "ticket";

impl Vault {
    /// Open the vault in `path`, creating it on first use, and start
    /// serving it.
    pub fn open(path: &Path, app: &str, infra: Infra, port: Option<u16>) -> Result<Self, String> {
        let mut dir = Dir::create(path)?;
        let identity = match gantz_collab::identity::load(&dir.store) {
            Some(identity) => identity,
            None => {
                let identity = Identity::generate();
                checked(&mut dir.store, |s| {
                    gantz_collab::identity::save(s, &identity)
                })?;
                identity
            }
        };
        let config = match gantz_store::load(&dir.store, CONFIG_KEY) {
            Some(config) => config,
            None => {
                let config = Config {
                    id: SessionId::generate(),
                    pairing: PairingSecret::generate(),
                    devices: BTreeSet::new(),
                };
                checked(&mut dir.store, |s| {
                    gantz_store::save(s, CONFIG_KEY, &config)
                })?;
                config
            }
        };
        let (registry, unreadable) = gantz_store::load_registry(&dir.store);
        let persisted = PersistedRegistry::from_registry(&registry, unreadable);
        info!(
            "vault {} in {}: {} graphs, {} paired devices",
            config.id,
            dir.path.display(),
            registry.heads().count(),
            config.devices.len(),
        );
        let runtime = RuntimeConfig {
            infra,
            port,
            app: app.to_string(),
        };
        let handle = gantz_collab::spawn(identity, runtime);
        let entry = VaultEntry {
            id: config.id,
            access: config.devices.clone(),
            pairing: config.pairing,
            store: registry.clone(),
        };
        handle
            .cmds
            .try_send(Command::HostVault(entry))
            .map_err(|_| "the collab runtime is gone".to_string())?;
        Ok(Self {
            dir,
            handle,
            config,
            registry,
            persisted,
            ticket: None,
        })
    }

    /// Serve until the runtime stops.
    pub fn serve(mut self) -> Result<(), String> {
        while let Ok(event) = self.handle.events.recv_blocking() {
            self.handle(event)?;
        }
        Err("the collab runtime stopped".to_string())
    }

    /// Handle the runtime events waiting now.
    #[cfg(test)]
    pub(crate) fn step(&mut self) -> Result<(), String> {
        while let Ok(event) = self.handle.events.try_recv() {
            self.handle(event)?;
        }
        Ok(())
    }

    /// Stop the runtime and wait for it to release its sockets.
    #[cfg(test)]
    pub(crate) fn stop(self) {
        let Handle { cmds, events } = self.handle;
        drop(cmds);
        while events.recv_blocking().is_ok() {}
    }

    /// Handle one runtime event. An error is fatal, since the vault must not
    /// run on with state it failed to persist.
    fn handle(&mut self, event: Event) -> Result<(), String> {
        match event {
            Event::Ready { peer } => info!("vault peer {peer}"),
            Event::VaultTicketReady { ticket, .. } => self.set_ticket(ticket)?,
            Event::DeviceSeen {
                peer,
                app,
                paired: true,
                ..
            } => {
                self.config.devices.insert(peer);
                let config = &self.config;
                checked(&mut self.dir.store, |s| {
                    gantz_store::save(s, CONFIG_KEY, config)
                })?;
                info!("paired device {} ({app})", peer.to_hex());
            }
            Event::Probed {
                peer,
                info,
                outdated: Some(outdated),
            } => {
                let update = match outdated {
                    Outdated::Us => "needs a newer vault",
                    Outdated::Them => "must update to sync",
                };
                warn!("device {} runs {}, which {update}", peer.to_hex(), info.app);
            }
            Event::PushRequest {
                from, push, reply, ..
            } => {
                let (name, tip) = (push.name.clone(), push.tip);
                match gantz_collab_sync::serve_push(&mut self.registry, self.config.id, push) {
                    PushOutcome::Accepted(update) => {
                        self.persist()?;
                        let _ = self.handle.cmds.try_send(update);
                        reply.send(tip);
                        match tip {
                            Some(tip) => info!("{from} moved {name} to {}", tip.display_short()),
                            None => info!("{from} removed {name}"),
                        }
                    }
                    PushOutcome::Stale(head) => reply.send(head),
                    PushOutcome::Invalid(reason) => {
                        warn!("{from}: {reason}");
                        reply.refuse(reason);
                    }
                }
            }
            Event::Error { message, .. } => warn!("{message}"),
            _ => (),
        }
        Ok(())
    }

    /// Write the registry's new content and moved heads to disk.
    fn persist(&mut self) -> Result<(), String> {
        let (registry, persisted) = (&self.registry, &mut self.persisted);
        checked(&mut self.dir.store, |s| {
            gantz_store::save_registry_incremental(s, registry, persisted)
        })
    }

    /// Record the latest ticket, print it and write it for the owner.
    fn set_ticket(&mut self, ticket: String) -> Result<(), String> {
        if self.ticket.as_ref() == Some(&ticket) {
            return Ok(());
        }
        let path = self.dir.path.join(TICKET_FILE);
        write_private(&path, ticket.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("Link a device with this vault ticket:\n{ticket}");
        self.ticket = Some(ticket);
        Ok(())
    }
}

impl Dir {
    /// Create the vault directory, owner-only, unless it exists. Then open
    /// it.
    pub(crate) fn create(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            let at = |e: std::io::Error| format!("{}: {e}", path.display());
            std::fs::create_dir_all(path).map_err(at)?;
            restrict(path, 0o700).map_err(at)?;
        }
        Self::open(path)
    }

    /// Open the vault directory and take its lock.
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        if !path.is_dir() {
            return Err(format!("no vault in {}", path.display()));
        }
        let at = |e: std::io::Error| format!("{}: {e}", path.display());
        let lock = File::create(path.join("lock")).map_err(at)?;
        lock.try_lock().map_err(|_| {
            format!(
                "the vault in {} is in use. Is `gantz vault serve` running?",
                path.display()
            )
        })?;
        let store = PkvStore::new_in_dir(path);
        Ok(Self {
            path: path.to_path_buf(),
            store,
            _lock: lock,
        })
    }
}

impl Save for Checked<'_> {
    type Err = bevy_pkv::SetError;
    fn set_string(&mut self, key: &str, value: &str) -> Result<(), Self::Err> {
        let result = self.store.set_string(key, value);
        if let Err(e) = &result {
            self.error.get_or_insert_with(|| format!("{key}: {e}"));
        }
        result
    }
}

/// Run a vault subcommand. Returns the exit code.
///
/// `conf` locates the default vault directory.
pub fn run(args: VaultArgs, conf: &Conf) -> i32 {
    let result = match args.command {
        VaultCommand::Serve(args) => serve(args, conf),
        VaultCommand::Devices(args) => devices(args, conf),
        VaultCommand::Revoke(args) => revoke(args, conf),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn serve(args: ServeArgs, conf: &Conf) -> Result<(), String> {
    let path = vault_dir(&args.dir, conf)?;
    let infra = gantz_collab_sync::infra(args.relay.as_deref());
    Vault::open(&path, conf.build, infra, Some(args.port))?.serve()
}

/// Print each paired device's id.
fn devices(args: DirArgs, conf: &Conf) -> Result<(), String> {
    let dir = Dir::open(&vault_dir(&args, conf)?)?;
    let config = load_config(&dir)?;
    for device in &config.devices {
        println!("{}", device.to_hex());
    }
    Ok(())
}

/// Unpair a device and rotate the pairing secret, so the old ticket no
/// longer pairs. Takes effect when the vault next starts.
fn revoke(args: RevokeArgs, conf: &Conf) -> Result<(), String> {
    let peer = args.peer.to_hex();
    let mut dir = Dir::open(&vault_dir(&args.dir, conf)?)?;
    let mut config = load_config(&dir)?;
    if !config.devices.remove(&args.peer) {
        return Err(format!("{peer} is not a paired device"));
    }
    config.pairing = PairingSecret::generate();
    checked(&mut dir.store, |s| {
        gantz_store::save(s, CONFIG_KEY, &config)
    })?;
    println!(
        "Revoked {peer}. The old ticket no longer pairs. Restart the vault to apply this and \
         print its new ticket."
    );
    Ok(())
}

/// The vault directory: `--dir`, or `vault` under the app data directory.
fn vault_dir(args: &DirArgs, conf: &Conf) -> Result<PathBuf, String> {
    match &args.dir {
        Some(dir) => Ok(dir.clone()),
        None => directories::ProjectDirs::from("", conf.org, conf.app)
            .map(|dirs| dirs.data_dir().join("vault"))
            .ok_or_else(|| "no data directory for this user; pass --dir".to_string()),
    }
}

fn load_config(dir: &Dir) -> Result<Config, String> {
    gantz_store::load(&dir.store, CONFIG_KEY)
        .ok_or_else(|| format!("no vault in {}", dir.path.display()))
}

/// Run `write` against `store`, and fail with its first failed write.
fn checked(store: &mut PkvStore, write: impl FnOnce(&mut Checked)) -> Result<(), String> {
    let mut checked = Checked { store, error: None };
    write(&mut checked);
    checked
        .error
        .map_or(Ok(()), |e| Err(format!("failed to write {e}")))
}

/// Write a file only its owner may read.
#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    // The mode applies only to a new file.
    restrict(path, 0o600)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// Set a path's permission bits.
#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}
