//! The network runtime: an iroh endpoint driven on its own executor,
//! bridged to the application through [`Command`]/[`Event`] channels.
//!
//! Natively the driver runs a current-thread tokio runtime on a dedicated
//! thread (iroh requires a tokio reactor; the rest of gantz has none). On
//! wasm it runs on the browser's event loop via `wasm-bindgen-futures`. The
//! channels are `async-channel`, so the application side polls with plain
//! `try_send`/`try_recv` from its update loop on both targets.
//!
//! The runtime is deliberately dumb plumbing: it subscribes gossip topics,
//! forwards messages both ways, fetches objects on request, and serves the
//! [`SessionRegistry`](crate::SessionRegistry) to peers. All convergence
//! decisions (what to announce, what to fetch, how to merge) live with the
//! application.
//!
//! # Infrastructure
//!
//! The endpoint's relay and address-lookup infrastructure is chosen by
//! [`RuntimeConfig::infra`]. The default, [`Infra::N0`], uses n0's public
//! services: free but rate-limited with no SLA - suitable for development
//! and jamming. [`Infra::Custom`] runs entirely on self-hosted or
//! third-party infrastructure (relays via `iroh-relay`, address lookup via
//! a pkarr relay such as `iroh-dns-server`) - nothing n0 is baked in.
//! Native peers usually upgrade to direct (hole-punched) paths; browser
//! peers are relay-only by design.

use crate::{
    identity::Identity,
    proto::{self, GossipMsg, Objects, SyncRequest, SyncResponse, Want},
    session::{PeerId, SessionId},
    store::{self, SessionEntry, Shared},
    ticket::SessionTicket,
};
use gantz_ca::{
    BlobLiveness, Bytes, Commit, CommitAddr, ContentAddr, DataGraph, GraphAddr, Key, Liveness,
    MergePolicy, Name, SectionId, Value,
};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode,
    address_lookup::{PkarrPublisher, PkarrResolver},
    endpoint::{Connection, presets},
    protocol::{AcceptError, Router},
};
use iroh_gossip::{
    api::{Event as TopicEvent, GossipSender},
    net::Gossip,
    proto::TopicId,
};
use n0_future::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The ALPN identifying gantz's session-sync request protocol. The version
/// is part of the string: incompatible revisions are distinct protocols.
pub const SYNC_ALPN: &[u8] = b"gantz/sync/1";

/// The application-level protocol version negotiated in
/// [`SyncRequest::Hello`].
pub const PROTO_VERSION: u32 = 1;

/// The domain-separation tag hashed with a session id to derive its gossip
/// topic id: the raw session id never appears on the gossip wire. Versioned
/// alongside the protocol - changing it partitions old and new peers onto
/// disjoint topics.
pub const TOPIC_DOMAIN: &[u8] = b"gantz/session/v1";

/// Endpoint configuration for [`spawn`].
#[derive(Clone, Debug, Default)]
pub struct RuntimeConfig {
    /// The relay and address-lookup infrastructure the endpoint binds with.
    pub infra: Infra,
}

/// The relay and address-lookup (peer discovery) infrastructure.
///
/// Nothing n0-specific is baked into the protocol: [`Infra::Custom`] runs
/// entirely on self-hosted or third-party services, and an invalid custom
/// URL fails the runtime rather than silently falling back to n0.
#[derive(Clone, Debug, Default)]
pub enum Infra {
    /// n0's public defaults: their relay servers and the `iroh.link`
    /// address-lookup (pkarr/DNS) service. Free but rate-limited with no
    /// SLA - right for development and jamming; heavier use should bring
    /// its own infrastructure via [`Infra::Custom`].
    #[default]
    N0,
    /// Explicit infrastructure; nothing contacts n0.
    Custom {
        /// Relay server URLs (e.g. a self-hosted [`iroh-relay`]). Empty
        /// disables relaying entirely: peers must then be reachable
        /// directly (e.g. on a LAN) via ticket bootstrap addresses or
        /// address lookup. Browser peers are relay-routed by design, so a
        /// session with web participants needs at least one relay.
        ///
        /// [`iroh-relay`]: https://github.com/n0-computer/iroh/tree/main/iroh-relay
        relays: Vec<String>,
        /// A pkarr relay URL for publishing and resolving peer addresses
        /// (e.g. a self-hosted [`iroh-dns-server`]'s `/pkarr` endpoint).
        /// `None` skips address lookup entirely: peers are then dialable
        /// only via ticket bootstrap addresses, paths learnt over gossip,
        /// and the relays above.
        ///
        /// [`iroh-dns-server`]: https://github.com/n0-computer/iroh/tree/main/iroh-dns-server
        pkarr: Option<String>,
    },
}

/// The read limit for a request (want lists scale with missing objects).
const REQUEST_LIMIT: usize = 1024 * 1024;

/// The read limit for a response (a snapshot carries whole graph histories).
const RESPONSE_LIMIT: usize = 64 * 1024 * 1024;

/// An instruction from the application to the runtime.
///
/// Commands apply in send order (one channel), so a [`Register`] reliably
/// precedes the [`Share`]/[`Join`] that needs it. The channel is unbounded:
/// sending never blocks, which keeps the application's frame loop free of
/// runtime locks entirely - the served stores are owned by the runtime and
/// mutated only here.
///
/// [`Register`]: Command::Register
/// [`Share`]: Command::Share
/// [`Join`]: Command::Join
#[derive(Debug)]
pub enum Command {
    /// Register (or replace) a session: its configuration plus the initially
    /// served content (a filled store for a host, an empty one for a guest).
    Register(SessionEntry),
    /// Merge served content into a registered session's store:
    /// content-addressed commit/graph/blob inserts (idempotent), per-name
    /// head upserts and section entries applied per the section's merge
    /// policy. Graphs and blobs are verified against their claimed addresses
    /// (see [`store::merge`]); a failed verification drops the whole update
    /// with a warning. Unknown sessions are ignored with a warning.
    Update {
        session: SessionId,
        heads: Vec<(Name, CommitAddr)>,
        commits: Vec<(CommitAddr, Commit)>,
        graphs: Vec<(GraphAddr, DataGraph)>,
        sections: Vec<(SectionId, MergePolicy, Liveness, Key, Value)>,
        blobs: Vec<(SectionId, BlobLiveness, ContentAddr, Bytes)>,
    },
    /// Start serving and gossiping a session. The session must already be
    /// [`Register`](Command::Register)ed. Emits [`Event::TicketReady`].
    Share(SessionId),
    /// Join a session from a ticket: the application
    /// [`Register`](Command::Register)s the guest entry first; this fetches
    /// the snapshot from the ticket's hosts and subscribes the gossip topic.
    /// Emits [`Event::Joined`] or [`Event::Error`].
    Join(SessionTicket),
    /// Stop gossiping a session (its content stays served until
    /// [`Forget`](Command::Forget)).
    Leave(SessionId),
    /// Drop a session entirely: stop serving its content.
    Forget(SessionId),
    /// Broadcast a message on a session's gossip topic.
    Broadcast { session: SessionId, msg: GossipMsg },
    /// Fetch objects from a peer over the request plane. Emits
    /// [`Event::Objects`] or [`Event::Error`].
    Fetch {
        session: SessionId,
        from: PeerId,
        want: Want,
    },
}

/// A notification from the runtime to the application.
#[derive(Debug)]
pub enum Event {
    /// The endpoint is bound and dialable.
    Ready { peer: PeerId },
    /// The invite ticket for a shared session.
    TicketReady { session: SessionId, ticket: String },
    /// A join completed: the host's scoped heads and snapshot objects,
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
    /// Objects fetched from a peer.
    Objects {
        session: SessionId,
        from: PeerId,
        objects: Objects,
    },
    /// A peer became a direct gossip neighbour for a session.
    PeerUp { session: SessionId, peer: PeerId },
    /// A gossip neighbour was dropped.
    PeerDown { session: SessionId, peer: PeerId },
    /// A recoverable failure the application may surface.
    Error {
        session: Option<SessionId>,
        message: String,
    },
}

/// The application's handle to the runtime.
///
/// Both channels are unbounded: `cmds.try_send` never blocks and never
/// drops, and `events.try_recv` polls without waiting, so a per-frame
/// application loop touches no locks and never parks.
#[derive(Clone, Debug)]
pub struct Handle {
    /// Instructions into the runtime.
    pub cmds: async_channel::Sender<Command>,
    /// Notifications out of the runtime.
    pub events: async_channel::Receiver<Event>,
}

/// The request-plane server: answers [`SyncRequest`]s from the shared
/// session stores, gating restricted sessions by peer identity.
#[derive(Clone, Debug)]
struct SyncServer {
    shared: Shared,
}

/// Cached peer connections for the request plane, keyed by peer.
///
/// iroh does not pool connections, and a fresh QUIC handshake per request -
/// typically relay-routed until holepunching completes - dominated sync
/// latency. `Connection` is a cheap clonable handle, and holding one here
/// also keeps the connection alive between requests (the server side already
/// serves any number of streams per connection). Shared because request
/// tasks are spawned off the driver.
type ConnCache = Arc<Mutex<HashMap<EndpointId, Connection>>>;

impl SyncServer {
    /// Answer one request. Runs under the shared lock: lookups only.
    fn respond(&self, remote: PeerId, req: SyncRequest) -> SyncResponse {
        let (SyncRequest::Hello { session, .. }
        | SyncRequest::Snapshot { session }
        | SyncRequest::Heads { session }
        | SyncRequest::Want { session, .. }) = req;
        let state = self.shared.lock();
        let Some(entry) = state.sessions.get(&session) else {
            return SyncResponse::Denied {
                reason: "unknown session".to_string(),
            };
        };
        if !entry.allows(remote) {
            return SyncResponse::Denied {
                reason: "access denied".to_string(),
            };
        }
        match req {
            SyncRequest::Hello { proto, .. } => SyncResponse::Hello {
                proto: PROTO_VERSION,
                accepted: proto == PROTO_VERSION,
            },
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
        }
    }
}

impl iroh::protocol::ProtocolHandler for SyncServer {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = PeerId(*conn.remote_id().as_bytes());
        // One request per bi-stream; the connection serves until the peer
        // closes it (any stream error means exactly that).
        loop {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else {
                return Ok(());
            };
            let Ok(bytes) = recv.read_to_end(REQUEST_LIMIT).await else {
                return Ok(());
            };
            let Ok(req) = proto::decode::<SyncRequest>(&bytes) else {
                return Ok(());
            };
            let resp = self.respond(remote, req);
            if send.write_all(&proto::encode(&resp)).await.is_err() {
                return Ok(());
            }
            let _ = send.finish();
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

/// The driver: binds the endpoint, serves the sync protocol, and loops over
/// application commands until the command channel closes.
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
    let builder = match infra_builder(&config.infra) {
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
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let router = Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.clone())
        .accept(
            SYNC_ALPN,
            SyncServer {
                shared: shared.clone(),
            },
        )
        .spawn();
    send_evt(Event::Ready {
        peer: identity.peer_id(),
    })
    .await;

    // Per-subscribed-session gossip senders (receivers live in forwarders).
    let mut senders: HashMap<SessionId, GossipSender> = HashMap::new();
    // Bootstrap addresses learnt from tickets, as a dial fallback.
    let mut bootstrap: HashMap<SessionId, Vec<EndpointAddr>> = HashMap::new();
    // Cached request-plane connections, shared with the request tasks.
    let conns: ConnCache = ConnCache::default();

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
                n0_future::task::spawn(async move {
                    let evt = join_snapshot(&endpoint, &conns, &ticket).await;
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
                    let req = SyncRequest::Want { session, want };
                    let evt = match request(&endpoint, &conns, addr, &req).await {
                        Ok(SyncResponse::Objects(objects)) => Event::Objects {
                            session,
                            from,
                            objects,
                        },
                        Ok(SyncResponse::Denied { reason }) => Event::Error {
                            session: Some(session),
                            message: format!("fetch denied: {reason}"),
                        },
                        Ok(_) => Event::Error {
                            session: Some(session),
                            message: "unexpected fetch response".to_string(),
                        },
                        Err(message) => Event::Error {
                            session: Some(session),
                            message,
                        },
                    };
                    let _ = evt_tx.send(evt).await;
                });
            }
        }
    }
    // The application dropped its handle: shut the endpoint down.
    router.shutdown().await.ok();
    endpoint.close().await;
}

/// The endpoint builder for the configured [`Infra`].
///
/// Custom infrastructure starts from iroh's minimal preset, so nothing n0
/// remains; an unparsable URL is an error rather than a silent fallback (a
/// self-hosted deployment must not leak onto n0's services by accident).
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

/// The session's gossip topic id: a hash of the session id under
/// [`TOPIC_DOMAIN`], so the raw session id never appears on the gossip wire.
fn topic_id(session: SessionId) -> TopicId {
    let mut hasher = gantz_ca::Hasher::new();
    hasher.update(TOPIC_DOMAIN);
    hasher.update(&session.0);
    TopicId::from_bytes(hasher.finalize().into())
}

/// The best known dial target for a peer: its id (iroh's discovery and
/// learnt paths resolve it), enriched with any ticket bootstrap addresses
/// for the same peer.
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
    // An invalid key can only come from a corrupted allowlist entry; fall
    // back to a valueless dial target that simply fails to connect.
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
                    // Dropped tips re-heal on the next `Tips` announcement
                    // (anti-entropy `Digest`/`Heads` pulls are reserved wire
                    // slots, not yet implemented).
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

/// Lock the connection cache; a poisoned lock still yields the map (entries
/// are validated before use anyway).
fn lock_conns(conns: &ConnCache) -> std::sync::MutexGuard<'_, HashMap<EndpointId, Connection>> {
    conns
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One request/response over a bi-stream on the given connection.
async fn exchange(conn: &Connection, req: &SyncRequest) -> Result<SyncResponse, String> {
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| format!("stream failed: {e}"))?;
    send.write_all(&proto::encode(req))
        .await
        .map_err(|e| format!("send failed: {e}"))?;
    send.finish().map_err(|e| format!("finish failed: {e}"))?;
    let bytes = recv
        .read_to_end(RESPONSE_LIMIT)
        .await
        .map_err(|e| format!("receive failed: {e}"))?;
    proto::decode(&bytes).map_err(|e| format!("undecodable response ({} bytes): {e}", bytes.len()))
}

/// One request/response, reusing the cached connection to the peer when it
/// is still live, else dialing (and caching) a fresh one.
///
/// A failure on a cached connection invalidates it and retries once fresh
/// (the peer may have restarted); a failure on a fresh connection is final.
async fn request(
    endpoint: &Endpoint,
    conns: &ConnCache,
    addr: EndpointAddr,
    req: &SyncRequest,
) -> Result<SyncResponse, String> {
    let id = addr.id;
    let cached = lock_conns(conns)
        .get(&id)
        .filter(|c| c.close_reason().is_none())
        .cloned();
    if let Some(conn) = cached {
        match exchange(&conn, req).await {
            Ok(resp) => return Ok(resp),
            Err(_) => {
                lock_conns(conns).remove(&id);
            }
        }
    }
    let conn = endpoint
        .connect(addr, SYNC_ALPN)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    lock_conns(conns).insert(id, conn.clone());
    match exchange(&conn, req).await {
        Ok(resp) => Ok(resp),
        Err(e) => {
            lock_conns(conns).remove(&id);
            Err(e)
        }
    }
}

/// Hello + snapshot against each ticket host in turn.
async fn join_snapshot(endpoint: &Endpoint, conns: &ConnCache, ticket: &SessionTicket) -> Event {
    let session = ticket.session;
    let mut last_error = "ticket carries no host addresses".to_string();
    for host in &ticket.hosts {
        let hello = SyncRequest::Hello {
            session,
            proto: PROTO_VERSION,
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
