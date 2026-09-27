//! `gantz join`: a headless session peer that mirrors the shared graphs to a
//! directory of `.gantz` files and commits the edits made to them.
//!
//! The peer drives [`gantz_collab_sync`] with no open heads, so every remote
//! tip converges headlessly. Files are rewritten when names move and read
//! back when they change on disk. See [`crate::mirror`].

use crate::mirror::Mirror;
use crate::{Conf, JoinArgs};
use gantz_ca as ca;
use gantz_collab::{Handle, Identity, Infra, PeerId, RuntimeConfig};
use gantz_collab_sync::{Effect, JoinError, OpenHeads, Sessions};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug, info, warn};

/// How often runtime events are drained.
const TICK: Duration = Duration::from_millis(50);
/// How often the directory is polled for edits.
const FILE_POLL: Duration = Duration::from_millis(250);
/// The identity file's length: the secret key bytes.
const IDENTITY_LEN: usize = 32;

/// A headless peer over one session.
pub struct Peer {
    handle: Handle,
    sessions: Sessions,
    registry: ca::Registry,
    mirror: Mirror,
    peer: PeerId,
    branch: Option<ca::Name>,
    /// The snapshot has landed. Files are written only from then on, so the
    /// join placeholder never reaches disk.
    joined: bool,
}

/// Why the peer stopped.
#[derive(Debug)]
pub enum Stop {
    /// The session could not be joined.
    Failed(String),
    /// The runtime closed its channels.
    RuntimeGone,
    Io(std::io::Error),
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(message) => write!(f, "join failed: {message}"),
            Self::RuntimeGone => write!(f, "collab runtime stopped"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<std::io::Error> for Stop {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl Peer {
    /// Spawn the runtime and prepare a registry holding the configured base
    /// sources, as every app has them, so edits can reference base graphs.
    pub fn new(identity: Identity, infra: Infra, dir: PathBuf, conf: &Conf) -> Self {
        let peer = identity.peer_id();
        let handle = gantz_collab::spawn(identity, RuntimeConfig { infra });
        let registry = crate::headless::load_sources(
            &crate::headless::base_sources(conf),
            gantz_egui::base::BASE_TIMESTAMP,
            &conf.codec,
        )
        .registry;
        Self {
            handle,
            sessions: Sessions::default(),
            registry,
            mirror: Mirror::new(dir, conf.codec),
            peer,
            branch: None,
            joined: false,
        }
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer
    }

    /// The shared branch, once joined.
    pub fn branch(&self) -> Option<&ca::Name> {
        self.branch.as_ref()
    }

    pub fn join(&mut self, ticket: &str, now: ca::Timestamp) -> Result<(), JoinError> {
        let (_, branch) = gantz_collab_sync::join(
            &mut self.sessions,
            &mut self.registry,
            &self.handle,
            ticket,
            now,
        )?;
        self.branch = Some(branch);
        Ok(())
    }

    /// One tick: drain the runtime, mirror moved names to disk, read edited
    /// files when `poll_files`, and announce local commits. Progress is
    /// logged at `info`, head moves at `debug`.
    pub fn step(&mut self, now: ca::Timestamp, poll_files: bool) -> Result<(), Stop> {
        if self.handle.events.is_closed() {
            return Err(Stop::RuntimeGone);
        }
        let effects = gantz_collab_sync::poll(
            &mut self.sessions,
            &mut self.registry,
            &self.handle,
            &OpenHeads::default(),
        );
        let mut changed = false;
        for effect in effects {
            match effect {
                // The plane logs the snapshot itself.
                Effect::Joined { .. } => {
                    self.joined = true;
                    changed = true;
                }
                Effect::Moved { name, to, .. } => {
                    changed = true;
                    debug!("{name} -> {}", to.display_short());
                }
                Effect::ResyncRefs => {
                    for m in gantz_collab_sync::resync_headless(&mut self.registry, now) {
                        changed = true;
                        debug!("{} follows -> {}", m.name, m.new_commit.display_short());
                    }
                }
                Effect::PeerUp { peer, .. } => info!("peer up {peer}"),
                Effect::PeerDown { peer, .. } => info!("peer down {peer}"),
                Effect::Error { message, .. } => {
                    if !self.joined {
                        return Err(Stop::Failed(message));
                    }
                    warn!("{message}");
                }
                // Nothing is open here, so the plane never routes a tip to
                // an open head.
                Effect::RemoteTip { name, .. } => {
                    warn!("remote tip for `{name}` routed to an open head with none open")
                }
                Effect::Open(_) | Effect::Action { .. } => {}
            }
        }
        if self.joined && changed {
            self.write()?;
        }
        if self.joined && poll_files {
            let mut edited = false;
            for (path, result) in self.mirror.poll(&mut self.registry, now, changed)? {
                let label = path.display().to_string();
                match result {
                    Ok(applied) => {
                        for (name, commit) in &applied.committed {
                            info!("{label}: committed {name} {}", commit.display_short());
                        }
                        for (name, commit) in &applied.layout_only {
                            info!("{label}: layout {name} {}", commit.display_short());
                        }
                        for m in &applied.moved {
                            debug!(
                                "{label}: {} follows -> {}",
                                m.name,
                                m.new_commit.display_short()
                            );
                        }
                        edited |= !applied.is_empty();
                    }
                    Err(e) => warn!("{}", crate::parse_diagnostic(&label, &e)),
                }
            }
            if edited {
                self.sessions.dirty = true;
                // Referrers a resync moved may live in other files.
                self.write()?;
            }
        }
        let announced = gantz_collab_sync::announce(
            &mut self.sessions,
            &self.registry,
            &self.handle,
            self.peer,
        );
        for (_, name, tip) in announced {
            info!("announced {name} {}", tip.display_short());
        }
        Ok(())
    }

    /// Rewrite the files of every scoped name that moved.
    fn write(&mut self) -> std::io::Result<()> {
        let Some(branch) = &self.branch else {
            return Ok(());
        };
        let scope = gantz_egui::sync::session_scope(&self.registry, branch);
        for path in self.mirror.write_all(&self.registry, &scope)? {
            info!("wrote {}", path.display());
        }
        Ok(())
    }
}

/// Run the subcommand until the runtime stops. Returns the exit code.
pub fn run(args: JoinArgs, conf: &Conf) -> i32 {
    // Parsed again by the join. Failing here spares spawning a runtime.
    let ticket = match args.ticket.trim().parse::<gantz_collab::SessionTicket>() {
        Ok(ticket) => ticket,
        Err(e) => {
            eprintln!("invalid ticket: {e}");
            return 2;
        }
    };
    let dir = match args.dir.clone().or_else(|| default_dir(conf, &ticket)) {
        Some(dir) => dir,
        None => {
            eprintln!("no data directory for this user; pass --dir");
            return 1;
        }
    };
    let identity = match identity(args.identity.as_deref()) {
        Ok(identity) => identity,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("{}: {e}", dir.display());
        return 1;
    }
    let infra = gantz_collab_sync::infra(args.relay.as_deref());
    let mut peer = Peer::new(identity, infra, dir.clone(), conf);
    info!("peer {}", peer.peer_id());
    if let Err(e) = peer.join(&args.ticket, now()) {
        eprintln!("{e}");
        return 2;
    }
    info!(
        "joining `{}` into {}",
        peer.branch().expect("joined"),
        dir.display()
    );
    let mut next_poll = Instant::now();
    loop {
        let poll_files = Instant::now() >= next_poll;
        if poll_files {
            next_poll = Instant::now() + FILE_POLL;
        }
        match peer.step(now(), poll_files) {
            Ok(()) => {}
            Err(e @ Stop::Failed(_)) => {
                eprintln!("{e}");
                return 2;
            }
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        }
        std::thread::sleep(TICK);
    }
}

/// The default working directory for a session: the app data directory,
/// then `sessions/<name>-<session>`. Joining the same session again lands
/// in the same directory, and sessions sharing a graph name do not collide.
fn default_dir(conf: &Conf, ticket: &gantz_collab::SessionTicket) -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", conf.org, conf.app)?;
    let session = format!("{}-{}", ticket.name, ticket.session);
    Some(dirs.data_dir().join("sessions").join(session))
}

/// The identity at `path`, created there when the file is absent. With no
/// path, a fresh identity for this run.
fn identity(path: Option<&Path>) -> std::io::Result<Identity> {
    let Some(path) = path else {
        return Ok(Identity::generate());
    };
    match std::fs::read(path) {
        Ok(bytes) => {
            let bytes: [u8; IDENTITY_LEN] = bytes.as_slice().try_into().map_err(|_| {
                std::io::Error::other(format!(
                    "{}: expected {IDENTITY_LEN} bytes, found {}",
                    path.display(),
                    bytes.len()
                ))
            })?;
            Ok(Identity::from_bytes(bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let identity = Identity::generate();
            std::fs::write(path, identity.to_bytes())?;
            Ok(identity)
        }
        Err(e) => Err(std::io::Error::other(format!("{}: {e}", path.display()))),
    }
}

/// The current time as a registry timestamp.
pub(crate) fn now() -> ca::Timestamp {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
}
