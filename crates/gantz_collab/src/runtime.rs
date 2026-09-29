//! The network runtime. An iroh endpoint driven on its own executor and
//! bridged to the application through [`Command`] and [`Event`] channels.
//!
//! Natively the driver runs a current-thread tokio runtime on a dedicated
//! thread. iroh requires a tokio reactor and the rest of gantz has none. On
//! wasm it runs on the browser's event loop via `wasm-bindgen-futures`. The
//! channels are `async-channel`, so the application side polls with plain
//! `try_send` and `try_recv` from its update loop on both targets.
//!
//! The runtime is deliberately dumb plumbing. It subscribes gossip topics,
//! forwards messages both ways, fetches objects on request, and serves the
//! [`crate::SessionRegistry`] to peers. All convergence decisions live with
//! the application. That covers what to announce, what to fetch and how to
//! merge.
//!
//! Vaults ride the same endpoint and request plane. See [`crate::vault`].
//! A hosting runtime pairs devices, serves watch streams and forwards pushes
//! to the application. A linked device's runtime holds the link open and
//! reconnects with backoff.
//!
//! # Infrastructure
//!
//! [`RuntimeConfig::infra`] chooses the endpoint's relay and address-lookup
//! infrastructure. The default, [`Infra::N0`], uses n0's public services.
//! They are free but rate-limited with no SLA, so they suit development and
//! jamming. [`Infra::Custom`] runs entirely on self-hosted or third-party
//! infrastructure. Relays come from `iroh-relay` and address lookup from a
//! pkarr relay such as `iroh-dns-server`. Nothing n0 is baked in. Native
//! peers usually upgrade to direct hole-punched paths. Browser peers are
//! relay-only by design.

use crate::{
    identity::Identity,
    proto::{self, GossipMsg, Objects, SyncRequest, SyncResponse, Want},
    session::{PeerId, SessionId},
    store::{self, ServedVault, SessionEntry, Shared, SharedState},
    ticket::{SessionTicket, VaultTicket},
    vault::{Push, PushReply, VaultEntry, VaultId, WatchMsg},
};
use gantz_ca::{
    BlobLiveness, Bytes, Commit, CommitAddr, ContentAddr, DataGraph, GraphAddr, Key, Liveness,
    MergePolicy, Name, SectionId, Value,
};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, Watcher,
    address_lookup::{PkarrPublisher, PkarrResolver},
    endpoint::{Connection, RecvStream, SendStream, presets},
    protocol::{AcceptError, Router},
};
use iroh_gossip::{
    api::{Event as TopicEvent, GossipSender},
    net::Gossip,
    proto::TopicId,
};
use n0_future::{StreamExt, task::AbortOnDropHandle};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The ALPN identifying gantz's sync request protocol. The version is part
/// of the string, so incompatible revisions are distinct protocols.
pub const SYNC_ALPN: &[u8] = b"gantz/sync/3";

/// The application-level protocol version negotiated in
/// [`SyncRequest::Hello`].
pub const PROTO_VERSION: u32 = 3;

/// The domain-separation tag hashed with a session id to derive its gossip
/// topic id. The raw session id never appears on the gossip wire. Versioned
/// alongside the protocol. Changing it partitions old and new peers onto
/// disjoint topics.
pub const TOPIC_DOMAIN: &[u8] = b"gantz/session/v1";

/// Endpoint configuration for [`spawn`].
#[derive(Clone, Debug, Default)]
pub struct RuntimeConfig {
    /// The relay and address-lookup infrastructure the endpoint binds with.
    pub infra: Infra,
    /// A fixed UDP port for the endpoint's sockets. `None` binds a random
    /// port. A vault fixes its port so that the addresses in its tickets
    /// stay valid across restarts. Ignored in the browser.
    pub port: Option<u16>,
    /// The host app and its version, such as `gantz 0.4.0`. Peers see it
    /// for display only. Compatibility is decided on [`PROTO_VERSION`].
    pub app: String,
}

/// The relay and address-lookup infrastructure, for peer discovery.
///
/// Nothing n0-specific is baked into the protocol. [`Infra::Custom`] runs
/// entirely on self-hosted or third-party services, and an invalid custom
/// URL fails the runtime rather than silently falling back to n0.
#[derive(Clone, Debug, Default)]
pub enum Infra {
    /// n0's public defaults. Their relay servers and the `iroh.link` pkarr
    /// and DNS address-lookup service. Free but rate-limited with no SLA, so
    /// right for development and jamming. Heavier use should bring its own
    /// infrastructure via [`Infra::Custom`].
    #[default]
    N0,
    /// Explicit infrastructure. Nothing contacts n0.
    Custom {
        /// Relay server URLs, for example a self-hosted [`iroh-relay`].
        /// Empty disables relaying entirely. Peers must then be reachable
        /// directly via ticket bootstrap addresses or address lookup.
        /// Browser peers are relay-routed by design, so a session with web
        /// participants needs at least one relay.
        ///
        /// [`iroh-relay`]: https://github.com/n0-computer/iroh/tree/main/iroh-relay
        relays: Vec<String>,
        /// A pkarr relay URL for publishing and resolving peer addresses,
        /// for example a self-hosted [`iroh-dns-server`]'s `/pkarr`
        /// endpoint. `None` skips address lookup entirely. Peers are then
        /// dialable only via ticket bootstrap addresses, paths learnt over
        /// gossip, and the relays above.
        ///
        /// [`iroh-dns-server`]: https://github.com/n0-computer/iroh/tree/main/iroh-dns-server
        pkarr: Option<String>,
    },
}

/// The read limit for a request. Want lists scale with missing objects.
const REQUEST_LIMIT: usize = 1024 * 1024;

/// The read limit for a response. A snapshot carries whole graph histories.
/// Also the limit for a push from a paired device, which carries history.
const RESPONSE_LIMIT: usize = 64 * 1024 * 1024;

/// The first delay before a dropped vault link retries. It doubles per
/// failed attempt, up to [`LINK_RETRY_MAX`].
const LINK_RETRY_MIN: Duration = Duration::from_secs(1);

/// The longest delay between vault link attempts.
const LINK_RETRY_MAX: Duration = Duration::from_secs(30);

/// An instruction from the application to the runtime.
///
/// Commands apply in send order on one channel, so a [`Register`] reliably
/// precedes the [`Share`] or [`Join`] that needs it. The channel is unbounded
/// and sending never blocks. That keeps the application's frame loop free of
/// runtime locks. The runtime owns the served stores and mutates them only
/// here.
///
/// [`Register`]: Command::Register
/// [`Share`]: Command::Share
/// [`Join`]: Command::Join
#[derive(Debug)]
pub enum Command {
    /// Register or replace a session. Its configuration plus the initially
    /// served content. That is a filled store for a host and an empty one
    /// for a guest.
    Register(SessionEntry),
    /// Merge served content into a registered session's store. See
    /// [`store::merge`]. A failed verification drops the whole update with a
    /// warning. Unknown sessions are ignored with a warning.
    Update {
        session: SessionId,
        heads: Vec<(Name, CommitAddr)>,
        commits: Vec<(CommitAddr, Commit)>,
        graphs: Vec<(GraphAddr, DataGraph)>,
        sections: Vec<(SectionId, MergePolicy, Liveness, Key, Value)>,
        blobs: Vec<(SectionId, BlobLiveness, ContentAddr, Bytes)>,
    },
    /// Start serving and gossiping a session. The session must already be
    /// registered via [`Command::Register`]. Emits [`Event::TicketReady`].
    Share(SessionId),
    /// Join a session from a ticket. The application registers the guest
    /// entry first via [`Command::Register`]. This fetches the snapshot from
    /// the ticket's hosts and subscribes the gossip topic. Emits
    /// [`Event::Joined`] or [`Event::Error`].
    Join(SessionTicket),
    /// Stop gossiping a session. Its content stays served until
    /// [`Command::Forget`].
    Leave(SessionId),
    /// Drop a session entirely and stop serving its content.
    Forget(SessionId),
    /// Broadcast a message on a session's gossip topic.
    Broadcast { session: SessionId, msg: GossipMsg },
    /// Fetch objects from a peer over the request plane. Emits
    /// [`Event::Objects`] or [`Event::FetchFailed`]. For a linked vault,
    /// `session` is the vault's id.
    Fetch {
        session: SessionId,
        from: PeerId,
        want: Want,
    },
    /// Host a vault. Serves its store, pairs devices that present its
    /// secret and emits [`Event::VaultTicketReady`], again whenever the
    /// endpoint's address changes.
    HostVault(VaultEntry),
    /// Apply an accepted change to a hosted vault's served store, then
    /// notify its watch streams of the moved heads. `None` removes a head.
    /// See [`store::update_vault`].
    UpdateVault {
        vault: VaultId,
        heads: Vec<(Name, Option<CommitAddr>)>,
        commits: Vec<(CommitAddr, Commit)>,
        graphs: Vec<(GraphAddr, DataGraph)>,
        sections: Vec<(SectionId, MergePolicy, Liveness, Key, Value)>,
        blobs: Vec<(SectionId, BlobLiveness, ContentAddr, Bytes)>,
    },
    /// Link this device to a vault and hold the link open, reconnecting with
    /// backoff. Emits [`Event::LinkUp`], [`Event::LinkChanged`] and
    /// [`Event::LinkDown`].
    Link(VaultTicket),
    /// Drop the link to a vault.
    Unlink(VaultId),
    /// Push one name to a linked vault. Emits [`Event::Pushed`].
    Push { vault: VaultId, push: Push },
}

/// A notification from the runtime to the application.
#[derive(Debug)]
pub enum Event {
    /// The endpoint is bound and dialable.
    Ready { peer: PeerId },
    /// The invite ticket for a shared session.
    TicketReady { session: SessionId, ticket: String },
    /// A join completed. The host's scoped heads and snapshot objects,
    /// ready for staged validation.
    Joined {
        session: SessionId,
        heads: Vec<(Name, CommitAddr)>,
        objects: Objects,
    },
    /// A gossip message from a session peer.
    Gossip {
        session: SessionId,
        from: PeerId,
        msg: GossipMsg,
    },
    /// Objects fetched from a peer, answering the [`Command::Fetch`] for
    /// `want`. The response holds the wanted objects the peer had.
    Objects {
        session: SessionId,
        from: PeerId,
        want: Want,
        objects: Objects,
    },
    /// The [`Command::Fetch`] for `want` failed, so nothing will answer it.
    FetchFailed {
        session: SessionId,
        from: PeerId,
        want: Want,
        error: String,
    },
    /// The link ticket for a hosted vault, minted from the endpoint's
    /// current address.
    VaultTicketReady { vault: VaultId, ticket: String },
    /// A device greeted a hosted vault with a compatible protocol. `paired`
    /// is true when it just presented the pairing secret and joined the
    /// vault's allowlist. The application persists that, and may record the
    /// device's `app` and `proto` for display.
    DeviceSeen {
        vault: VaultId,
        peer: PeerId,
        app: String,
        proto: u32,
        paired: bool,
    },
    /// A paired device asks to move a name on a hosted vault. The
    /// application decides, persists, then answers through `reply`.
    PushRequest {
        vault: VaultId,
        from: PeerId,
        push: Push,
        reply: PushReply,
    },
    /// A link to a vault opened. Every head the vault holds.
    LinkUp {
        vault: VaultId,
        heads: Vec<(Name, CommitAddr)>,
    },
    /// Names that moved on a linked vault. `None` means removed.
    LinkChanged {
        vault: VaultId,
        changes: Vec<(Name, Option<CommitAddr>)>,
    },
    /// A link to a vault dropped or failed to open. It retries by itself
    /// until [`Command::Unlink`].
    LinkDown { vault: VaultId, error: String },
    /// A vault answered a [`Command::Push`]. `Ok` holds the vault's head for
    /// the name after the push, which is `tip` exactly when it was accepted.
    Pushed {
        vault: VaultId,
        name: Name,
        tip: Option<CommitAddr>,
        result: Result<Option<CommitAddr>, String>,
    },
    /// A peer became a direct gossip neighbour for a session.
    PeerUp { session: SessionId, peer: PeerId },
    /// A gossip neighbour was dropped.
    PeerDown { session: SessionId, peer: PeerId },
    /// The endpoint's home relays changed. `(url, connected)` per relay.
    RelayStatus { relays: Vec<(String, bool)> },
    /// A recoverable failure the application may surface.
    Error {
        session: Option<SessionId>,
        message: String,
    },
}

/// The application's handle to the runtime.
///
/// Both channels are unbounded. `cmds.try_send` never blocks and never
/// drops, and `events.try_recv` polls without waiting, so a per-frame
/// application loop touches no locks and never parks.
#[derive(Clone, Debug)]
pub struct Handle {
    /// Instructions into the runtime.
    pub cmds: async_channel::Sender<Command>,
    /// Notifications out of the runtime.
    pub events: async_channel::Receiver<Event>,
}

/// The request-plane server. It answers [`SyncRequest`]s from the shared
/// session and vault stores and gates access by peer identity.
#[derive(Clone, Debug)]
struct SyncServer {
    shared: Shared,
    /// For events raised while serving, such as pairings and pushes.
    events: async_channel::Sender<Event>,
}

/// Cached peer connections for the request plane, keyed by peer.
///
/// iroh does not pool connections. A fresh QUIC handshake per request is
/// typically relay-routed until holepunching completes, and it dominates
/// sync latency. `Connection` is a cheap clonable handle. Holding one here
/// also keeps the connection alive between requests, and the server side
/// serves any number of streams per connection. Shared because request
/// tasks are spawned off the driver.
type ConnCache = Arc<Mutex<HashMap<EndpointId, Connection>>>;

impl SyncServer {
    /// Serve one bi-stream. Most requests get one response. A watch holds
    /// the stream open, and a push waits on the application's decision.
    async fn serve_stream(self, remote: PeerId, mut send: SendStream, mut recv: RecvStream) {
        let paired = self
            .shared
            .lock()
            .vaults
            .values()
            .any(|v| v.entry.access.contains(&remote));
        let limit = if paired {
            RESPONSE_LIMIT
        } else {
            REQUEST_LIMIT
        };
        let Ok(bytes) = recv.read_to_end(limit).await else {
            return;
        };
        let Ok(req) = proto::decode::<SyncRequest>(&bytes) else {
            return;
        };
        let resp = match req {
            SyncRequest::Watch { session } => return self.watch(remote, session, send).await,
            SyncRequest::Push { session, push } => self.push(remote, session, push).await,
            req => self.respond(remote, req),
        };
        if send.write_all(&proto::encode(&resp)).await.is_ok() {
            let _ = send.finish();
        }
    }

    /// Answer one request. Runs under the shared lock, so lookups only.
    fn respond(&self, remote: PeerId, req: SyncRequest) -> SyncResponse {
        let mut state = self.shared.lock();
        let session = req.session();
        if let Some(vault) = state.vaults.get_mut(&session) {
            return self.respond_vault(vault, remote, req);
        }
        let Some(entry) = state.sessions.get(&session) else {
            return denied("unknown session");
        };
        if !entry.allows(remote) {
            return denied("access denied");
        }
        match req {
            SyncRequest::Hello { proto, .. } => hello(proto),
            SyncRequest::Snapshot { .. } => {
                let (heads, objects) = store::snapshot(&entry.store);
                SyncResponse::Snapshot { heads, objects }
            }
            SyncRequest::Heads { .. } => {
                let heads = entry.store.heads().map(|(n, ca)| (n.clone(), ca)).collect();
                SyncResponse::Heads { heads }
            }
            SyncRequest::Want { want, .. } => {
                SyncResponse::Objects(store::objects(&entry.store, &want))
            }
            SyncRequest::Watch { .. } | SyncRequest::Push { .. } => denied("not a vault"),
        }
    }

    /// Answer one request to a hosted vault. A `Hello` from an unknown peer
    /// that presents the pairing secret pairs it, but only when it speaks
    /// this protocol version.
    fn respond_vault(
        &self,
        vault: &mut ServedVault,
        remote: PeerId,
        req: SyncRequest,
    ) -> SyncResponse {
        if let SyncRequest::Hello {
            proto,
            pairing,
            app,
            ..
        } = &req
        {
            if *proto != PROTO_VERSION {
                return hello(*proto);
            }
            let paired = !vault.entry.access.contains(&remote);
            if paired {
                if !pairing
                    .as_ref()
                    .is_some_and(|p| vault.entry.pairing.matches(p))
                {
                    return denied("access denied");
                }
                vault.entry.access.insert(remote);
            }
            let seen = Event::DeviceSeen {
                vault: vault.entry.id,
                peer: remote,
                app: app.clone(),
                proto: *proto,
                paired,
            };
            let _ = self.events.try_send(seen);
            return hello(*proto);
        }
        if !vault.entry.access.contains(&remote) {
            return denied("access denied");
        }
        match req {
            SyncRequest::Want { want, .. } => {
                SyncResponse::Objects(store::objects(&vault.entry.store, &want))
            }
            _ => denied("unsupported by a vault"),
        }
    }

    /// Follow a hosted vault's heads. Writes every head, then each change
    /// until the vault stops or the peer goes away. The watcher registers
    /// under the same lock that applies updates, so no change is missed.
    async fn watch(&self, remote: PeerId, session: SessionId, mut send: SendStream) {
        let (tx, rx) = async_channel::unbounded();
        let heads = {
            let mut state = self.shared.lock();
            let Ok(vault) = paired_vault(&mut state, session, remote) else {
                return;
            };
            vault.watchers.push(tx);
            vault
                .entry
                .store
                .heads()
                .map(|(n, ca)| (n.clone(), ca))
                .collect()
        };
        let mut msg = WatchMsg::Heads(heads);
        loop {
            if write_frame(&mut send, &msg).await.is_err() {
                return;
            }
            let stopped = send.stopped();
            let next = async { rx.recv().await.ok() };
            let stopped = async {
                let _ = stopped.await;
                None
            };
            let Some(next) = n0_future::future::or(next, stopped).await else {
                let _ = send.finish();
                return;
            };
            msg = next;
        }
    }

    /// Forward a push to the application and wait for its decision.
    async fn push(&self, remote: PeerId, session: SessionId, push: Push) -> SyncResponse {
        if let Err(refusal) = paired_vault(&mut self.shared.lock(), session, remote) {
            return refusal;
        }
        let (tx, rx) = async_channel::bounded(1);
        let request = Event::PushRequest {
            vault: session,
            from: remote,
            push,
            reply: PushReply(tx),
        };
        if self.events.send(request).await.is_err() {
            return denied("the vault stopped");
        }
        match rx.recv().await {
            Ok(Ok(head)) => SyncResponse::Pushed { head },
            Ok(Err(reason)) => denied(&reason),
            Err(_) => denied("the vault dropped the push"),
        }
    }
}

impl iroh::protocol::ProtocolHandler for SyncServer {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = PeerId(*conn.remote_id().as_bytes());
        // Each bi-stream is served on its own task, since a watch stays open
        // and a push waits on the application. The connection serves until
        // the peer closes it.
        loop {
            let Ok((send, recv)) = conn.accept_bi().await else {
                return Ok(());
            };
            n0_future::task::spawn(self.clone().serve_stream(remote, send, recv));
        }
    }
}

/// Spawn the runtime for the given identity, returning the application's
/// handle. Emits [`Event::Ready`] once the endpoint is bound.
pub fn spawn(identity: Identity, config: RuntimeConfig) -> Handle {
    let (cmd_tx, cmd_rx) = async_channel::unbounded();
    let (evt_tx, evt_rx) = async_channel::unbounded();
    let handle = Handle {
        cmds: cmd_tx,
        events: evt_rx,
    };
    let drive = drive(identity, config, Shared::default(), cmd_rx, evt_tx);
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt.block_on(drive),
            Err(e) => log::error!("collab runtime failed to start tokio: {e}"),
        }
    });
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(drive);
    handle
}

/// The driver. It binds the endpoint, serves the sync protocol, and loops
/// over application commands until the command channel closes.
async fn drive(
    identity: Identity,
    config: RuntimeConfig,
    shared: Shared,
    cmd_rx: async_channel::Receiver<Command>,
    evt_tx: async_channel::Sender<Event>,
) {
    let send_evt = |evt: Event| {
        let evt_tx = evt_tx.clone();
        async move {
            let _ = evt_tx.send(evt).await;
        }
    };
    let error = |session: Option<SessionId>, message: String| {
        log::warn!("collab: {message}");
        send_evt(Event::Error { session, message })
    };
    let app: Arc<str> = config.app.as_str().into();
    let builder = match infra_builder(&config.infra).and_then(|b| bind_port(b, config.port)) {
        Ok(builder) => builder,
        Err(e) => {
            error(None, e).await;
            return;
        }
    };
    let endpoint = match builder
        .secret_key(identity.secret_key())
        .alpns(vec![SYNC_ALPN.to_vec(), iroh_gossip::ALPN.to_vec()])
        .bind()
        .await
    {
        Ok(endpoint) => endpoint,
        Err(e) => {
            error(None, format!("failed to bind endpoint: {e}")).await;
            return;
        }
    };
    // Surface the home relays and their connection state to the app.
    {
        let evt_tx = evt_tx.clone();
        let mut statuses = endpoint.home_relay_status().stream();
        n0_future::task::spawn(async move {
            while let Some(statuses) = statuses.next().await {
                let relays = statuses
                    .iter()
                    .map(|s| (s.url().to_string(), s.is_connected()))
                    .collect();
                if evt_tx.send(Event::RelayStatus { relays }).await.is_err() {
                    break;
                }
            }
        });
    }
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let router = Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.clone())
        .accept(
            SYNC_ALPN,
            SyncServer {
                shared: shared.clone(),
                events: evt_tx.clone(),
            },
        )
        .spawn();
    send_evt(Event::Ready {
        peer: identity.peer_id(),
    })
    .await;

    // Gossip senders per subscribed session. Receivers live in forwarders.
    let mut senders: HashMap<SessionId, GossipSender> = HashMap::new();
    // Bootstrap addresses learnt from tickets, as a dial fallback.
    let mut bootstrap: HashMap<SessionId, Vec<EndpointAddr>> = HashMap::new();
    // Cached request-plane connections, shared with the request tasks.
    let conns: ConnCache = ConnCache::default();
    // Vault link tasks. Dropping one ends its link.
    let mut links: HashMap<VaultId, AbortOnDropHandle<()>> = HashMap::new();
    // Ticket re-minting tasks per hosted vault.
    let mut tickets: HashMap<VaultId, AbortOnDropHandle<()>> = HashMap::new();

    while let Ok(cmd) = cmd_rx.recv().await {
        match cmd {
            Command::Register(entry) => {
                let mut state = shared.lock();
                state.sessions.insert(entry.session.id, entry);
            }
            Command::Update {
                session,
                heads,
                commits,
                graphs,
                sections,
                blobs,
            } => {
                let mut state = shared.lock();
                let Some(entry) = state.sessions.get_mut(&session) else {
                    log::warn!("collab: update for an unregistered session");
                    continue;
                };
                let result =
                    store::merge(&mut entry.store, heads, commits, graphs, sections, blobs);
                if let Err(e) = result {
                    log::warn!("collab: update rejected: {e}");
                }
            }
            Command::Forget(session) => {
                let mut state = shared.lock();
                state.sessions.remove(&session);
            }
            Command::Share(session) => {
                match subscribe(&gossip, &evt_tx, session, vec![]).await {
                    Ok(sender) => {
                        senders.insert(session, sender);
                    }
                    Err(e) => {
                        error(Some(session), format!("failed to subscribe gossip: {e}")).await;
                        continue;
                    }
                }
                let ticket = {
                    let state = shared.lock();
                    state.sessions.get(&session).map(|entry| {
                        SessionTicket::new(
                            session,
                            entry.session.branch.clone(),
                            entry.session.access.clone(),
                            entry.session.resolutions,
                            vec![endpoint.addr()],
                        )
                    })
                };
                let Some(ticket) = ticket else {
                    error(Some(session), "share of an unknown session".to_string()).await;
                    continue;
                };
                send_evt(Event::TicketReady {
                    session,
                    ticket: iroh_tickets::Ticket::encode_string(&ticket),
                })
                .await;
            }
            Command::Join(ticket) => {
                let session = ticket.session;
                let host_ids = ticket.hosts.iter().map(|a| a.id).collect();
                bootstrap.insert(session, ticket.hosts.clone());
                match subscribe(&gossip, &evt_tx, session, host_ids).await {
                    Ok(sender) => {
                        senders.insert(session, sender);
                    }
                    Err(e) => {
                        error(Some(session), format!("failed to subscribe gossip: {e}")).await;
                        continue;
                    }
                }
                // Snapshot from the first host that answers.
                let endpoint = endpoint.clone();
                let evt_tx = evt_tx.clone();
                let conns = conns.clone();
                let app = app.clone();
                n0_future::task::spawn(async move {
                    let evt = join_snapshot(&endpoint, &conns, &app, &ticket).await;
                    let _ = evt_tx.send(evt).await;
                });
            }
            Command::Leave(session) => {
                senders.remove(&session);
                bootstrap.remove(&session);
            }
            Command::Broadcast { session, msg } => {
                let Some(sender) = senders.get_mut(&session) else {
                    continue;
                };
                let bytes = proto::encode(&msg);
                if let Err(e) = sender.broadcast(bytes.into()).await {
                    error(Some(session), format!("gossip broadcast failed: {e}")).await;
                }
            }
            Command::Fetch {
                session,
                from,
                want,
            } => {
                let addr = dial_addr(&bootstrap, session, from);
                let endpoint = endpoint.clone();
                let evt_tx = evt_tx.clone();
                let conns = conns.clone();
                n0_future::task::spawn(async move {
                    let req = SyncRequest::Want {
                        session,
                        want: want.clone(),
                    };
                    let error = match request(&endpoint, &conns, addr, &req).await {
                        Ok(SyncResponse::Objects(objects)) => {
                            let objects = Event::Objects {
                                session,
                                from,
                                want,
                                objects,
                            };
                            let _ = evt_tx.send(objects).await;
                            return;
                        }
                        Ok(SyncResponse::Denied { reason }) => format!("fetch denied: {reason}"),
                        Ok(_) => "unexpected fetch response".to_string(),
                        Err(message) => message,
                    };
                    let failed = Event::FetchFailed {
                        session,
                        from,
                        want,
                        error,
                    };
                    let _ = evt_tx.send(failed).await;
                });
            }
            Command::HostVault(entry) => {
                let vault = entry.id;
                let pairing = entry.pairing;
                let served = ServedVault {
                    entry,
                    watchers: Vec::new(),
                };
                shared.lock().vaults.insert(vault, served);
                // The first address is the current one. Later ones follow
                // network changes, for example a relay connecting.
                let mut addrs = endpoint.watch_addr().stream();
                let evt_tx = evt_tx.clone();
                let task = n0_future::task::spawn(async move {
                    while let Some(addr) = addrs.next().await {
                        let ticket = VaultTicket {
                            vault,
                            pairing,
                            host: addr,
                        }
                        .to_string();
                        let ready = Event::VaultTicketReady { vault, ticket };
                        if evt_tx.send(ready).await.is_err() {
                            return;
                        }
                    }
                });
                tickets.insert(vault, AbortOnDropHandle::new(task));
            }
            Command::UpdateVault {
                vault,
                heads,
                commits,
                graphs,
                sections,
                blobs,
            } => {
                let mut state = shared.lock();
                let Some(served) = state.vaults.get_mut(&vault) else {
                    log::warn!("collab: update for an unhosted vault");
                    continue;
                };
                let result = store::update_vault(
                    &mut served.entry.store,
                    &heads,
                    commits,
                    graphs,
                    sections,
                    blobs,
                );
                match result {
                    Ok(()) if heads.is_empty() => (),
                    Ok(()) => served.notify(&WatchMsg::Changed(heads)),
                    Err(e) => log::warn!("collab: vault update rejected: {e}"),
                }
            }
            Command::Link(ticket) => {
                let vault = ticket.vault;
                bootstrap.insert(vault, vec![ticket.host.clone()]);
                let task = link(
                    endpoint.clone(),
                    conns.clone(),
                    evt_tx.clone(),
                    app.clone(),
                    ticket,
                );
                let task = n0_future::task::spawn(task);
                links.insert(vault, AbortOnDropHandle::new(task));
            }
            Command::Unlink(vault) => {
                links.remove(&vault);
                bootstrap.remove(&vault);
            }
            Command::Push { vault, push } => {
                let host = bootstrap
                    .get(&vault)
                    .and_then(|hosts| hosts.first())
                    .cloned();
                let endpoint = endpoint.clone();
                let evt_tx = evt_tx.clone();
                let conns = conns.clone();
                n0_future::task::spawn(async move {
                    let (name, tip) = (push.name.clone(), push.tip);
                    let result = match host {
                        None => Err("not linked to the vault".to_string()),
                        Some(host) => {
                            let req = SyncRequest::Push {
                                session: vault,
                                push,
                            };
                            match request(&endpoint, &conns, host, &req).await {
                                Ok(SyncResponse::Pushed { head }) => Ok(head),
                                Ok(SyncResponse::Denied { reason }) => {
                                    Err(format!("push denied: {reason}"))
                                }
                                Ok(_) => Err("unexpected push response".to_string()),
                                Err(e) => Err(e),
                            }
                        }
                    };
                    let pushed = Event::Pushed {
                        vault,
                        name,
                        tip,
                        result,
                    };
                    let _ = evt_tx.send(pushed).await;
                });
            }
        }
    }
    // The application dropped its handle, so shut the endpoint down.
    router.shutdown().await.ok();
    endpoint.close().await;
}

/// The endpoint builder for the configured [`Infra`].
///
/// Custom infrastructure starts from iroh's minimal preset, so nothing n0
/// remains. An unparsable URL is an error rather than a silent fallback. A
/// self-hosted deployment must not leak onto n0's services by accident.
fn infra_builder(infra: &Infra) -> Result<iroh::endpoint::Builder, String> {
    match infra {
        Infra::N0 => Ok(Endpoint::builder(presets::N0)),
        Infra::Custom { relays, pkarr } => {
            let mut builder = Endpoint::builder(presets::Minimal);
            builder = if relays.is_empty() {
                builder.relay_mode(RelayMode::Disabled)
            } else {
                let map = RelayMap::try_from_iter(relays.iter().map(|s| s.as_str()))
                    .map_err(|e| format!("invalid relay url: {e}"))?;
                builder.relay_mode(RelayMode::Custom(map))
            };
            if let Some(pkarr) = pkarr {
                let url: url::Url = pkarr
                    .parse()
                    .map_err(|e| format!("invalid pkarr url: {e}"))?;
                builder = builder
                    .address_lookup(PkarrPublisher::builder(url.clone()))
                    .address_lookup(PkarrResolver::builder(url));
            }
            Ok(builder)
        }
    }
}

/// Fix the endpoint's UDP port on both address families, as iroh's defaults
/// bind them. IPv6 may fail to bind, as by default.
#[cfg(not(target_arch = "wasm32"))]
fn bind_port(
    builder: iroh::endpoint::Builder,
    port: Option<u16>,
) -> Result<iroh::endpoint::Builder, String> {
    use std::net::{Ipv4Addr, Ipv6Addr};
    let Some(port) = port else {
        return Ok(builder);
    };
    let invalid = |e: iroh::endpoint::InvalidSocketAddr| format!("invalid bind port: {e}");
    let optional = iroh::endpoint::BindOpts::default().set_is_required(false);
    builder
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::UNSPECIFIED, port))
        .map_err(invalid)?
        .bind_addr_with_opts((Ipv6Addr::UNSPECIFIED, port), optional)
        .map_err(invalid)
}

/// The browser binds no sockets of its own, so there is no port to fix.
#[cfg(target_arch = "wasm32")]
fn bind_port(
    builder: iroh::endpoint::Builder,
    _port: Option<u16>,
) -> Result<iroh::endpoint::Builder, String> {
    Ok(builder)
}

/// The hosted vault `session`, if `remote` is paired with it. Else the
/// refusal to answer with.
fn paired_vault(
    state: &mut SharedState,
    session: SessionId,
    remote: PeerId,
) -> Result<&mut ServedVault, SyncResponse> {
    let vault = state
        .vaults
        .get_mut(&session)
        .ok_or_else(|| denied("unknown vault"))?;
    if !vault.entry.access.contains(&remote) {
        return Err(denied("access denied"));
    }
    Ok(vault)
}

/// A refusal carrying its reason.
fn denied(reason: &str) -> SyncResponse {
    SyncResponse::Denied {
        reason: reason.to_string(),
    }
}

/// The answer to an accepted peer's `Hello`.
fn hello(proto: u32) -> SyncResponse {
    SyncResponse::Hello {
        proto: PROTO_VERSION,
        accepted: proto == PROTO_VERSION,
    }
}

/// The session's gossip topic id. A hash of the session id under
/// [`TOPIC_DOMAIN`], so the raw session id never appears on the gossip wire.
fn topic_id(session: SessionId) -> TopicId {
    let mut hasher = gantz_ca::Hasher::new();
    hasher.update(TOPIC_DOMAIN);
    hasher.update(&session.0);
    TopicId::from_bytes(hasher.finalize().into())
}

/// The best known dial target for a peer. Its id, which iroh's discovery and
/// learnt paths resolve, enriched with any ticket bootstrap addresses for
/// the same peer.
fn dial_addr(
    bootstrap: &HashMap<SessionId, Vec<EndpointAddr>>,
    session: SessionId,
    peer: PeerId,
) -> EndpointAddr {
    let id = endpoint_id(peer);
    bootstrap
        .get(&session)
        .into_iter()
        .flatten()
        .find(|addr| addr.id == id)
        .cloned()
        .unwrap_or_else(|| EndpointAddr::from(id))
}

/// A [`PeerId`] as iroh's key type.
fn endpoint_id(peer: PeerId) -> EndpointId {
    // An invalid key can only come from a corrupted allowlist entry. Fall
    // back to a valueless dial target that fails to connect.
    EndpointId::from_bytes(&peer.0).unwrap_or_else(|_| {
        log::warn!("invalid peer key {peer}");
        EndpointId::from_bytes(&Identity::generate().peer_id().0).expect("a generated key is valid")
    })
}

/// Subscribe a session's gossip topic, spawning a forwarder that turns topic
/// events into [`Event`]s. Returns the topic's sender for broadcasts.
async fn subscribe(
    gossip: &Gossip,
    evt_tx: &async_channel::Sender<Event>,
    session: SessionId,
    bootstrap: Vec<EndpointId>,
) -> Result<GossipSender, String> {
    let topic = gossip
        .subscribe(topic_id(session), bootstrap)
        .await
        .map_err(|e| e.to_string())?;
    let (sender, mut receiver) = topic.split();
    let evt_tx = evt_tx.clone();
    n0_future::task::spawn(async move {
        while let Some(event) = receiver.next().await {
            let evt = match event {
                Ok(TopicEvent::Received(message)) => {
                    match proto::decode::<GossipMsg>(&message.content) {
                        Ok(msg) => Event::Gossip {
                            session,
                            from: PeerId(*message.delivered_from.as_bytes()),
                            msg,
                        },
                        Err(e) => Event::Error {
                            session: Some(session),
                            message: format!("undecodable gossip message: {e}"),
                        },
                    }
                }
                Ok(TopicEvent::NeighborUp(id)) => Event::PeerUp {
                    session,
                    peer: PeerId(*id.as_bytes()),
                },
                Ok(TopicEvent::NeighborDown(id)) => Event::PeerDown {
                    session,
                    peer: PeerId(*id.as_bytes()),
                },
                Ok(TopicEvent::Lagged) => Event::Error {
                    session: Some(session),
                    // Dropped tips re-heal on the next `Tips` announcement.
                    // Anti-entropy `Digest` and `Heads` pulls are reserved
                    // wire slots, not yet implemented.
                    message: "gossip lagged; dropped messages re-heal on the next announce"
                        .to_string(),
                },
                Err(e) => Event::Error {
                    session: Some(session),
                    message: format!("gossip stream error: {e}"),
                },
            };
            if evt_tx.send(evt).await.is_err() {
                break;
            }
        }
    });
    Ok(sender)
}

/// Lock the connection cache. A poisoned lock still yields the map, since
/// entries are validated before use.
fn lock_conns(conns: &ConnCache) -> std::sync::MutexGuard<'_, HashMap<EndpointId, Connection>> {
    conns
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Send `req` on a new bi-stream on the given connection. Returns the
/// stream that carries the reply.
async fn send_request(conn: &Connection, req: &SyncRequest) -> Result<RecvStream, String> {
    let (mut send, recv) = conn
        .open_bi()
        .await
        .map_err(|e| format!("stream failed: {e}"))?;
    send.write_all(&proto::encode(req))
        .await
        .map_err(|e| format!("send failed: {e}"))?;
    send.finish().map_err(|e| format!("finish failed: {e}"))?;
    Ok(recv)
}

/// One request and response over a bi-stream on the given connection.
async fn exchange(conn: &Connection, req: &SyncRequest) -> Result<SyncResponse, String> {
    let bytes = send_request(conn, req)
        .await?
        .read_to_end(RESPONSE_LIMIT)
        .await
        .map_err(|e| format!("receive failed: {e}"))?;
    proto::decode(&bytes).map_err(|e| format!("undecodable response ({} bytes): {e}", bytes.len()))
}

/// The cached connection to a peer, if it is still open.
fn cached(conns: &ConnCache, id: EndpointId) -> Option<Connection> {
    lock_conns(conns)
        .get(&id)
        .filter(|c| c.close_reason().is_none())
        .cloned()
}

/// Dial a peer and cache the connection.
async fn dial(
    endpoint: &Endpoint,
    conns: &ConnCache,
    addr: EndpointAddr,
) -> Result<Connection, String> {
    let id = addr.id;
    let conn = endpoint
        .connect(addr, SYNC_ALPN)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    lock_conns(conns).insert(id, conn.clone());
    Ok(conn)
}

/// Run `f` on the cached connection to the peer when it is still live, else
/// on a fresh one, which is dialed and cached.
///
/// A failure on a cached connection invalidates it and retries once fresh,
/// since the peer may have restarted. A failure on a fresh connection is
/// final.
async fn with_conn<T, Fut: Future<Output = Result<T, String>>>(
    endpoint: &Endpoint,
    conns: &ConnCache,
    addr: EndpointAddr,
    f: impl Fn(Connection) -> Fut,
) -> Result<T, String> {
    let id = addr.id;
    if let Some(conn) = cached(conns, id) {
        match f(conn).await {
            Ok(t) => return Ok(t),
            Err(_) => {
                lock_conns(conns).remove(&id);
            }
        }
    }
    let conn = dial(endpoint, conns, addr).await?;
    f(conn).await.inspect_err(|_| {
        lock_conns(conns).remove(&id);
    })
}

/// One request and response, over a connection from [`with_conn`].
async fn request(
    endpoint: &Endpoint,
    conns: &ConnCache,
    addr: EndpointAddr,
    req: &SyncRequest,
) -> Result<SyncResponse, String> {
    with_conn(endpoint, conns, addr, |conn| async move {
        exchange(&conn, req).await
    })
    .await
}

/// Write one frame to a long-lived stream. A little-endian `u32` length,
/// then the postcard body.
async fn write_frame<T: Serialize>(send: &mut SendStream, value: &T) -> Result<(), String> {
    let body = proto::encode(value);
    let len = u32::try_from(body.len()).map_err(|_| "frame too large".to_string())?;
    send.write_all(&len.to_le_bytes())
        .await
        .map_err(|e| format!("send failed: {e}"))?;
    send.write_all(&body)
        .await
        .map_err(|e| format!("send failed: {e}"))
}

/// Read one frame written by [`write_frame`].
async fn read_frame<T: DeserializeOwned>(recv: &mut RecvStream) -> Result<T, String> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len)
        .await
        .map_err(|e| format!("stream ended: {e}"))?;
    let len = u32::from_le_bytes(len) as usize;
    if len > RESPONSE_LIMIT {
        return Err(format!("a {len} byte frame exceeds the limit"));
    }
    let mut body = vec![0; len];
    recv.read_exact(&mut body)
        .await
        .map_err(|e| format!("stream ended: {e}"))?;
    proto::decode(&body).map_err(|e| format!("undecodable frame: {e}"))
}

/// Hold a device's link to its vault until the task is dropped. Each
/// attempt greets or pairs with `Hello`, then follows the watch stream. A
/// drop is reported and retried with backoff, which resets after an attempt
/// that reached the vault's heads.
async fn link(
    endpoint: Endpoint,
    conns: ConnCache,
    evt_tx: async_channel::Sender<Event>,
    app: Arc<str>,
    ticket: VaultTicket,
) {
    let vault = ticket.vault;
    let mut retry = LINK_RETRY_MIN;
    loop {
        let (reached, error) = follow(&endpoint, &conns, &evt_tx, &app, &ticket).await;
        if evt_tx.send(Event::LinkDown { vault, error }).await.is_err() {
            return;
        }
        if reached {
            retry = LINK_RETRY_MIN;
        }
        n0_future::time::sleep(retry).await;
        retry = (retry * 2).min(LINK_RETRY_MAX);
    }
}

/// One link attempt. Returns whether it reached the vault's heads and why
/// it ended.
async fn follow(
    endpoint: &Endpoint,
    conns: &ConnCache,
    evt_tx: &async_channel::Sender<Event>,
    app: &str,
    ticket: &VaultTicket,
) -> (bool, String) {
    let vault = ticket.vault;
    let hello = SyncRequest::Hello {
        session: vault,
        proto: PROTO_VERSION,
        pairing: Some(ticket.pairing),
        app: app.to_string(),
    };
    match request(endpoint, conns, ticket.host.clone(), &hello).await {
        Ok(SyncResponse::Hello { accepted: true, .. }) => (),
        Ok(SyncResponse::Hello { proto, .. }) => {
            let error =
                format!("protocol mismatch: vault speaks v{proto}, this build v{PROTO_VERSION}");
            return (false, error);
        }
        Ok(SyncResponse::Denied { reason }) => return (false, format!("link denied: {reason}")),
        Ok(_) => return (false, "unexpected hello response".to_string()),
        Err(e) => return (false, e),
    }
    let watch = &SyncRequest::Watch { session: vault };
    let open = |conn| async move { send_request(&conn, watch).await };
    let mut recv = match with_conn(endpoint, conns, ticket.host.clone(), open).await {
        Ok(recv) => recv,
        Err(e) => return (false, e),
    };
    let mut reached = false;
    loop {
        let evt = match read_frame::<WatchMsg>(&mut recv).await {
            Ok(WatchMsg::Heads(heads)) => {
                reached = true;
                Event::LinkUp { vault, heads }
            }
            Ok(WatchMsg::Changed(changes)) => Event::LinkChanged { vault, changes },
            Err(e) => return (reached, e),
        };
        if evt_tx.send(evt).await.is_err() {
            return (reached, "the application is gone".to_string());
        }
    }
}

/// Hello and snapshot against each ticket host in turn.
async fn join_snapshot(
    endpoint: &Endpoint,
    conns: &ConnCache,
    app: &str,
    ticket: &SessionTicket,
) -> Event {
    let session = ticket.session;
    let mut last_error = "ticket carries no host addresses".to_string();
    for host in &ticket.hosts {
        let hello = SyncRequest::Hello {
            session,
            proto: PROTO_VERSION,
            pairing: None,
            app: app.to_string(),
        };
        match request(endpoint, conns, host.clone(), &hello).await {
            Ok(SyncResponse::Hello { accepted: true, .. }) => {}
            Ok(SyncResponse::Hello { proto, .. }) => {
                last_error =
                    format!("protocol mismatch: host speaks v{proto}, this build v{PROTO_VERSION}");
                continue;
            }
            Ok(SyncResponse::Denied { reason }) => {
                last_error = format!("join denied: {reason}");
                continue;
            }
            Ok(_) => {
                last_error = "unexpected hello response".to_string();
                continue;
            }
            Err(e) => {
                last_error = e;
                continue;
            }
        }
        match request(
            endpoint,
            conns,
            host.clone(),
            &SyncRequest::Snapshot { session },
        )
        .await
        {
            Ok(SyncResponse::Snapshot { heads, objects }) => {
                return Event::Joined {
                    session,
                    heads,
                    objects,
                };
            }
            Ok(SyncResponse::Denied { reason }) => {
                last_error = format!("snapshot denied: {reason}");
            }
            Ok(_) => last_error = "unexpected snapshot response".to_string(),
            Err(e) => last_error = e,
        }
    }
    Event::Error {
        session: Some(session),
        message: format!("join failed: {last_error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::PairingSecret;
    use std::collections::BTreeSet;

    /// A server hosting one vault, plus the events it raises.
    fn server(pairing: PairingSecret) -> (SyncServer, ServedVault, async_channel::Receiver<Event>) {
        let (events, rx) = async_channel::unbounded();
        let server = SyncServer {
            shared: Shared::default(),
            events,
        };
        let vault = ServedVault {
            entry: VaultEntry {
                id: SessionId([1; 32]),
                access: BTreeSet::new(),
                pairing,
                store: Default::default(),
            },
            watchers: vec![],
        };
        (server, vault, rx)
    }

    fn hello_with(proto: u32, pairing: PairingSecret) -> SyncRequest {
        SyncRequest::Hello {
            session: SessionId([1; 32]),
            proto,
            pairing: Some(pairing),
            app: "gantz 0.0.1".to_string(),
        }
    }

    // A device on another protocol version never pairs, even with the secret.
    #[test]
    fn a_hello_for_another_protocol_never_pairs() {
        let pairing = PairingSecret::generate();
        let (server, mut vault, events) = server(pairing);
        let peer = PeerId([2; 32]);
        let resp = server.respond_vault(&mut vault, peer, hello_with(99, pairing));
        assert!(matches!(
            resp,
            SyncResponse::Hello {
                accepted: false,
                ..
            }
        ));
        assert!(vault.entry.access.is_empty());
        assert!(events.try_recv().is_err());

        let resp = server.respond_vault(&mut vault, peer, hello_with(PROTO_VERSION, pairing));
        let SyncResponse::Hello { accepted, .. } = resp else {
            panic!("expected a hello");
        };
        assert!(accepted);
        assert!(vault.entry.access.contains(&peer));
        let Ok(Event::DeviceSeen { paired, app, .. }) = events.try_recv() else {
            panic!("expected a device seen event");
        };
        assert!(paired);
        assert_eq!(app, "gantz 0.0.1");
    }
}
