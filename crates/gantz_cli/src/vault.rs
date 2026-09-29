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
//!
//! Every vault command refuses a directory that a newer gantz wrote, and
//! refuses to replace an identity or config it cannot read. Either would
//! cut off the paired devices. Update gantz to open such a vault.

use crate::{Conf, DirArgs, RevokeArgs, ServeArgs, VaultArgs, VaultCommand};
use bevy_pkv::PkvStore;
use gantz_ca as ca;
use gantz_collab::{
    Command, Event, Handle, Identity, Infra, Outdated, PROTO_MAX, PROTO_MIN, PairingSecret, PeerId,
    RuntimeConfig, SessionId, VaultEntry,
};
use gantz_collab_sync::PushOutcome;
use gantz_store::{PersistedRegistry, STORE_FORMAT, Save, Unwritable};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::{info, warn};

/// The vault's persisted configuration.
#[derive(Deserialize, Serialize)]
pub(crate) struct Config {
    id: SessionId,
    pairing: PairingSecret,
    pub(crate) devices: BTreeMap<PeerId, Device>,
}

/// A paired device, as last seen.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Device {
    /// The device's app and version, such as `gantz 0.4.0`.
    pub(crate) app: String,
    /// The newest sync protocol version the device speaks.
    pub(crate) proto: u32,
    /// When the device paired, in seconds since the Unix epoch.
    paired: u64,
    /// When the device last linked or probed, in seconds since the Unix
    /// epoch.
    seen: u64,
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
pub(crate) const CONFIG_KEY: &str = "vault";

/// The file holding the latest link ticket.
const TICKET_FILE: &str = "ticket";

impl Vault {
    /// Open the vault in `path`, creating it on first use, and start
    /// serving it. `app` is the app and version this vault runs, such as
    /// `gantz 0.4.0`.
    pub fn open(path: &Path, app: &str, infra: Infra, port: Option<u16>) -> Result<Self, String> {
        let mut dir = Dir::create(path)?;
        let identity = gantz_collab::identity::load(&dir.store).map_err(|e| refused(path, e))?;
        let config = load_config(&dir)?;
        let (mut registry, unreadable) = gantz_store::load_registry(&dir.store);
        let meta =
            gantz_store::writable_meta(&dir.store, &unreadable).map_err(|e| refused(path, e))?;
        let unread = unreadable.graphs.len() + unreadable.commits.len() + unreadable.sections.len();
        if unread > 0 {
            warn!("{unread} stored entries cannot be read. They stay as they are.");
        }
        let identity = match identity {
            Some(identity) => identity,
            None => {
                let identity = Identity::generate();
                checked(&mut dir.store, |s| {
                    gantz_collab::identity::save(s, &identity)
                })?;
                identity
            }
        };
        let config = match config {
            Some(config) => config,
            None => {
                let config = Config {
                    id: SessionId::generate(),
                    pairing: PairingSecret::generate(),
                    devices: BTreeMap::new(),
                };
                checked(&mut dir.store, |s| {
                    gantz_store::save(s, CONFIG_KEY, &config)
                })?;
                config
            }
        };
        let mut persisted = PersistedRegistry::from_registry(&registry, unreadable);
        checked(&mut dir.store, |s| {
            gantz_store::upgrade(s, meta.format, &mut registry, &mut persisted);
            gantz_store::save_store_meta(s, app);
        })?;
        let unverified = registry
            .graphs()
            .iter()
            .filter(|(ga, graph)| ca::sync::verify_graph(**ga, graph).is_err())
            .count();
        if unverified > 0 {
            warn!(
                "{unverified} stored graphs do not match their addresses. Devices cannot fetch them."
            );
        }
        info!("{app}: sync protocols {PROTO_MIN} to {PROTO_MAX}, store format {STORE_FORMAT}");
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
            access: config.devices.keys().copied().collect(),
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
                proto,
                paired,
                ..
            } => {
                if paired {
                    info!("paired device {} ({app})", peer.to_hex());
                }
                self.seen(peer, app, proto)?;
            }
            // Only an incompatible device stops at the probe. A compatible
            // one says hello next.
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
                if self.config.devices.contains_key(&peer) {
                    self.seen(peer, info.app, info.proto_max)?;
                }
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

    /// Record that `peer` linked or probed, running `app` on `proto`.
    fn seen(&mut self, peer: PeerId, app: String, proto: u32) -> Result<(), String> {
        let seen = unix_now();
        let paired = self.config.devices.get(&peer).map_or(seen, |d| d.paired);
        let device = Device {
            app,
            proto,
            paired,
            seen,
        };
        self.config.devices.insert(peer, device);
        let config = &self.config;
        checked(&mut self.dir.store, |s| {
            gantz_store::save(s, CONFIG_KEY, config)
        })
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
        let meta =
            gantz_store::load_store_meta(&store).map_err(|e| refused(path, Unwritable::Meta(e)))?;
        if meta.format > STORE_FORMAT {
            let newer = Unwritable::Newer(meta);
            return Err(refused(path, format!("{newer}. Update gantz to open it")));
        }
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

/// Print each paired device, what it last ran and when.
fn devices(args: DirArgs, conf: &Conf) -> Result<(), String> {
    let dir = Dir::open(&vault_dir(&args, conf)?)?;
    let config = load_config(&dir)?.ok_or_else(|| no_vault(&dir))?;
    if config.devices.is_empty() {
        println!("No paired devices.");
    }
    let now = unix_now();
    for (peer, device) in &config.devices {
        let app = match device.app.as_str() {
            "" => "an unknown app",
            app => app,
        };
        println!(
            "{}  {app} on sync protocol {}, paired {}, last seen {}",
            peer.to_hex(),
            device.proto,
            ago(now, device.paired),
            ago(now, device.seen),
        );
    }
    Ok(())
}

/// Unpair a device and rotate the pairing secret, so the old ticket no
/// longer pairs. Takes effect when the vault next starts.
fn revoke(args: RevokeArgs, conf: &Conf) -> Result<(), String> {
    let peer = args.peer.to_hex();
    let mut dir = Dir::open(&vault_dir(&args.dir, conf)?)?;
    let mut config = load_config(&dir)?.ok_or_else(|| no_vault(&dir))?;
    if config.devices.remove(&args.peer).is_none() {
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
            .ok_or_else(|| "no data directory for this user, so pass --dir".to_string()),
    }
}

/// The vault's config, or `None` in a new directory. Fails if a stored
/// config cannot be read.
fn load_config(dir: &Dir) -> Result<Option<Config>, String> {
    gantz_store::load_strict(&dir.store, CONFIG_KEY).map_err(|e| refused(&dir.path, e))
}

fn no_vault(dir: &Dir) -> String {
    format!("no vault in {}", dir.path.display())
}

/// Run `write` against `store`, and fail with its first failed write.
fn checked(store: &mut PkvStore, write: impl FnOnce(&mut Checked)) -> Result<(), String> {
    let mut checked = Checked { store, error: None };
    write(&mut checked);
    checked
        .error
        .map_or(Ok(()), |e| Err(format!("failed to write {e}")))
}

/// Why the vault in `path` will not open. Nothing in it is changed.
fn refused(path: &Path, reason: impl std::fmt::Display) -> String {
    format!(
        "cannot open the vault in {}: {reason}. It is left untouched.",
        path.display()
    )
}

/// How long before `now` the Unix time `then` was, roughly.
fn ago(now: u64, then: u64) -> String {
    let secs = now.saturating_sub(then);
    let (n, unit) = match secs {
        0..60 => return "just now".to_string(),
        60..3_600 => (secs / 60, "minute"),
        3_600..86_400 => (secs / 3_600, "hour"),
        _ => (secs / 86_400, "day"),
    };
    let plural = if n == 1 { "" } else { "s" };
    format!("{n} {unit}{plural} ago")
}

/// Seconds since the Unix epoch.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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
