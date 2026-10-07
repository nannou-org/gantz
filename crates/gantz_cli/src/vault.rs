//! `gantz vault`: a headless vault that a user's devices sync all their
//! named graphs with.
//!
//! `serve` holds the registry in the vault directory, in the app's store
//! format, and serves it over the collab runtime. A device pairs once with
//! the ticket that `gantz vault ticket` prints, then links on every start.
//! See `gantz_collab::vault` and `gantz_collab_sync::vault`.
//!
//! The ticket pairs devices, so the vault never logs it and never writes it
//! to disk. It hands the ticket only to a process that connects to its
//! socket, and logs only that it did. The socket is in the vault directory,
//! which only its owner can enter.
//!
//! The vault moves a name only when a push is made against its current
//! head, and replaces a name's metadata, such as its description, only
//! against its current metadata. It never merges, prunes or resyncs
//! references, so it keeps every graph's whole history. An accepted push is
//! on disk before the device hears it was accepted.
//!
//! The directory holds:
//!
//! - `bevy_pkv.redb`: the registry, the vault's identity and its config.
//! - `ticket.sock`: while `serve` runs, the socket that hands out the link
//!   ticket. Unix only.
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
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
    /// Before `dir`, so the socket is removed before the lock is released.
    tickets: TicketServer,
    dir: Dir,
    handle: Handle,
    pub(crate) config: Config,
    pub(crate) registry: ca::Registry,
    persisted: PersistedRegistry,
}

/// Hands the latest link ticket to each process that connects to
/// `ticket.sock`, then closes the connection. Connecting is the whole
/// request. Removes the socket when dropped.
pub(crate) struct TicketServer {
    path: PathBuf,
    /// The latest link ticket, once the runtime has minted one.
    ticket: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
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

/// The socket in the vault directory that hands out the link ticket.
const TICKET_SOCKET: &str = "ticket.sock";

/// The file where gantz 0.5 kept the link ticket.
const OLD_TICKET_FILE: &str = "ticket";

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
        remove_old_ticket(&dir.path)?;
        let tickets = TicketServer::start(&dir.path)?;
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
            tickets,
            dir,
            handle,
            config,
            registry,
            persisted,
        })
    }

    /// Serve until the runtime stops or the process is interrupted.
    ///
    /// An interrupt stops the runtime, which closes every connection, so
    /// devices see at once that the vault is offline. A second interrupt
    /// exits without waiting for that.
    pub fn serve(mut self) -> Result<(), String> {
        let interrupted = Arc::new(AtomicBool::new(false));
        let on_interrupt = Arc::clone(&interrupted);
        let cmds = self.handle.cmds.clone();
        ctrlc::set_handler(move || {
            if on_interrupt.swap(true, Ordering::Relaxed) {
                std::process::exit(130);
            }
            info!("stopping. Interrupt again to exit at once");
            cmds.close();
        })
        .map_err(|e| format!("cannot handle interrupts: {e}"))?;
        while let Ok(event) = self.handle.events.recv_blocking() {
            self.handle(event)?;
        }
        if interrupted.load(Ordering::Relaxed) {
            info!("stopped");
            Ok(())
        } else {
            Err("the collab runtime stopped".to_string())
        }
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
            Event::VaultTicketReady { ticket, .. } => self.tickets.set(ticket),
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
            Event::DeviceRefused { peer, app, .. } => warn!(
                "refused device {} ({app}). Its ticket is old, or its pairing was revoked",
                peer.to_hex()
            ),
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
                let head = self.registry.head(&name);
                match gantz_collab_sync::serve_push(&mut self.registry, self.config.id, push) {
                    PushOutcome::Accepted(update) => {
                        self.persist()?;
                        let _ = self.handle.cmds.try_send(update);
                        reply.send(gantz_collab::store::name_state(&self.registry, &name));
                        match tip {
                            _ if tip == head => info!("{from} changed the metadata of {name}"),
                            Some(tip) => info!("{from} moved {name} to {}", tip.display_short()),
                            None => info!("{from} removed {name}"),
                        }
                    }
                    PushOutcome::Stale(state) => reply.send(state),
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
}

impl TicketServer {
    /// Listen on `ticket.sock` in `dir`. The vault lock must be held, so a
    /// socket left there by an earlier run is stale.
    #[cfg(unix)]
    pub(crate) fn start(dir: &Path) -> Result<Self, String> {
        let path = dir.join(TICKET_SOCKET);
        let at = |e: std::io::Error| format!("{}: {e}", path.display());
        remove_if_present(&path).map_err(at)?;
        let listener = std::os::unix::net::UnixListener::bind(&path).map_err(at)?;
        restrict(&path, 0o600).map_err(at)?;
        let ticket = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (ticket, stop) = (Arc::clone(&ticket), Arc::clone(&stop));
            std::thread::spawn(move || serve_tickets(&listener, &ticket, &stop))
        };
        Ok(Self {
            path,
            ticket,
            stop,
            thread: Some(thread),
        })
    }

    #[cfg(not(unix))]
    pub(crate) fn start(_dir: &Path) -> Result<Self, String> {
        Err("the vault hands out its ticket over a Unix socket, so it runs on Unix only".into())
    }

    /// Hand out `ticket` from now on. The first one means that devices can
    /// link.
    pub(crate) fn set(&self, ticket: String) {
        if lock(&self.ticket).replace(ticket).is_none() {
            info!("ready to link devices. `gantz vault ticket` prints the link ticket");
        }
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

impl Drop for TicketServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // A connection wakes the listener so that it sees the stop. Without
        // one, the thread would never end, so it is left running.
        if wake(&self.path)
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Run a vault subcommand. Returns the exit code.
///
/// `conf` locates the default vault directory.
pub fn run(args: VaultArgs, conf: &Conf) -> i32 {
    let result = match args.command {
        VaultCommand::Serve(args) => serve(args, conf),
        VaultCommand::Ticket(args) => ticket(args, conf),
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

/// Print the link ticket of the running vault.
fn ticket(args: DirArgs, conf: &Conf) -> Result<(), String> {
    println!("{}", request_ticket(&vault_dir(&args, conf)?)?);
    Ok(())
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
        "Revoked {peer}. The old ticket no longer pairs. Restart the vault to apply this, then \
         run `gantz vault ticket` for its new ticket."
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

/// Ask the vault serving `dir` for its link ticket.
#[cfg(unix)]
pub(crate) fn request_ticket(dir: &Path) -> Result<String, String> {
    use std::io::{ErrorKind, Read};
    if !dir.is_dir() {
        return Err(format!("no vault in {}", dir.display()));
    }
    let path = dir.join(TICKET_SOCKET);
    let at = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut stream =
        std::os::unix::net::UnixStream::connect(&path).map_err(|e| match e.kind() {
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => {
                format!("the vault in {} is not running", dir.display())
            }
            ErrorKind::PermissionDenied => format!(
                "{}: {e}. Only the user that runs the vault can ask it for its ticket",
                path.display()
            ),
            _ => at(e),
        })?;
    let mut ticket = String::new();
    stream.read_to_string(&mut ticket).map_err(at)?;
    if ticket.is_empty() {
        return Err("the vault has no ticket yet. Try again in a moment".to_string());
    }
    Ok(ticket)
}

#[cfg(not(unix))]
pub(crate) fn request_ticket(_dir: &Path) -> Result<String, String> {
    Err("the vault hands out its ticket over a Unix socket, so it runs on Unix only".into())
}

/// Hand the latest ticket to each process that connects, until `stop`.
/// Logs only that it handed one out.
#[cfg(unix)]
fn serve_tickets(
    listener: &std::os::unix::net::UnixListener,
    ticket: &Mutex<Option<String>>,
    stop: &AtomicBool,
) {
    use std::io::Write;
    for stream in listener.incoming() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        // A request before the first ticket gets none, and the requester
        // reports that.
        let (Ok(mut stream), Some(ticket)) = (stream, lock(ticket).clone()) else {
            continue;
        };
        match stream.write_all(ticket.as_bytes()) {
            Ok(()) => info!("issued a link ticket"),
            Err(e) => warn!("failed to issue a link ticket: {e}"),
        }
    }
}

/// Connect to the ticket socket at `path`, so that its listener wakes.
/// Returns whether that worked.
#[cfg(unix)]
fn wake(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

#[cfg(not(unix))]
fn wake(_path: &Path) -> bool {
    false
}

/// Delete the ticket file that gantz 0.5 wrote, so that no ticket stays on
/// disk.
pub(crate) fn remove_old_ticket(dir: &Path) -> Result<(), String> {
    let path = dir.join(OLD_TICKET_FILE);
    let removed = remove_if_present(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if removed {
        info!(
            "deleted {}. `gantz vault ticket` prints the ticket",
            path.display()
        );
    }
    Ok(())
}

/// Remove the file at `path`, if there is one. Returns whether there was.
fn remove_if_present(path: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Lock `mutex`, even if a thread panicked while it held the lock.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
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
