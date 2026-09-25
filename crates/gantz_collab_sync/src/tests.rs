//! The plane against a fake runtime. A [`Handle`] built from two channels
//! captures every command and accepts scripted events.

use crate::*;
use gantz_ca::{self as ca, Commit, DataGraph, Datum, NodeData};
use gantz_collab::{
    Access, Command, Event, GossipMsg, Handle, Object, Objects, PeerId, Role, Session, SessionId,
    SessionTicket, proto,
};
use std::time::Duration;

/// A runtime stand-in: the commands it received and a sender for events.
struct Fake {
    handle: Handle,
    cmds: async_channel::Receiver<Command>,
    events: async_channel::Sender<Event>,
}

fn fake() -> Fake {
    let (cmd_tx, cmd_rx) = async_channel::unbounded();
    let (event_tx, event_rx) = async_channel::unbounded();
    Fake {
        handle: Handle {
            cmds: cmd_tx,
            events: event_rx,
        },
        cmds: cmd_rx,
        events: event_tx,
    }
}

impl Fake {
    fn drain(&self) -> Vec<Command> {
        std::iter::from_fn(|| self.cmds.try_recv().ok()).collect()
    }

    /// Deliver `event` and return the effects.
    fn deliver(
        &self,
        sessions: &mut Sessions,
        registry: &mut ca::Registry,
        open: &OpenHeads,
        event: Event,
    ) -> Vec<Effect> {
        self.events.send_blocking(event).unwrap();
        poll(sessions, registry, &self.handle, open)
    }
}

fn peer(n: u8) -> PeerId {
    PeerId([n; 32])
}

fn name(s: &str) -> ca::Name {
    s.parse().unwrap()
}

/// A graph whose nodes are `test` nodes numbered by `ids`.
fn graph(ids: &[u32]) -> DataGraph {
    let mut graph = DataGraph::default();
    for &id in ids {
        let data = Datum::Map(vec![("id".to_string(), Datum::F64(id as f64))]);
        graph.add_node(NodeData::new("test", data));
    }
    graph
}

/// Commit `graph` locally onto `parent`, returning the commit and its graph
/// address.
fn commit(
    registry: &mut ca::Registry,
    parent: Option<ca::CommitAddr>,
    graph: DataGraph,
    secs: u64,
) -> (ca::CommitAddr, ca::GraphAddr) {
    let ga = registry.add_graph(graph);
    let ca = registry.add_commit(Commit::new(Duration::from_secs(secs), parent, ga));
    (ca, ga)
}

/// The wire objects for a commit and its graph.
fn objects(commit: &Commit, graph: &DataGraph) -> Objects {
    Objects {
        objects: vec![
            Object::Commit(ca::commit_addr(commit), commit.clone().into()),
            Object::Graph(ca::graph_addr(graph), proto::encode_graph(graph)),
        ],
    }
}

fn ticket(session: SessionId, branch: &str) -> SessionTicket {
    SessionTicket::new(
        session,
        branch.to_string(),
        Access::Public,
        session_resolutions(),
        vec![],
    )
}

/// A guest session for `branch` with no placeholder, as after a join over
/// an existing local name.
fn guest(sessions: &mut Sessions, session: SessionId, branch: &str) {
    sessions.sessions.insert(
        session,
        SessionState::new(Session {
            id: session,
            branch: branch.to_string(),
            access: Access::Public,
            resolutions: session_resolutions(),
            role: Role::Guest,
        }),
    );
}

fn tips(session: SessionId, from: PeerId, name: ca::Name, tip: ca::CommitAddr) -> Event {
    Event::Gossip {
        session,
        from,
        msg: GossipMsg::Tips {
            origin: from,
            seq: 1,
            changed: vec![(
                name,
                tip,
                ca::GraphAddr::from(ca::ContentAddr::from([0; 32])),
            )],
        },
    }
}

#[test]
fn join_registers_mints_placeholder_and_joins() {
    let fake = fake();
    let mut sessions = Sessions::default();
    let mut registry = ca::Registry::default();
    let session = SessionId::generate();
    let ticket = ticket(session, "jam").to_string();
    let (id, branch) = join(
        &mut sessions,
        &mut registry,
        &fake.handle,
        &ticket,
        Duration::ZERO,
    )
    .unwrap();
    assert_eq!(id, session);
    assert_eq!(branch, name("jam"));
    let cmds = fake.drain();
    assert!(matches!(&cmds[0], Command::Register(e) if e.session.role == Role::Guest));
    assert!(matches!(&cmds[1], Command::Join(t) if t.session == session));
    assert_eq!(cmds.len(), 2);
    let placeholder = registry.head(&branch).expect("placeholder head");
    assert_eq!(sessions.sessions[&session].placeholder, Some(placeholder));
}

#[test]
fn join_rejects_proto_mismatch_and_bad_tickets() {
    let fake = fake();
    let mut sessions = Sessions::default();
    let mut registry = ca::Registry::default();
    let mut ticket = ticket(SessionId::generate(), "jam");
    ticket.proto += 1;
    let err = join(
        &mut sessions,
        &mut registry,
        &fake.handle,
        &ticket.to_string(),
        Duration::ZERO,
    )
    .unwrap_err();
    assert!(matches!(err, JoinError::Proto { .. }), "{err}");
    let err = join(
        &mut sessions,
        &mut registry,
        &fake.handle,
        "not a ticket",
        Duration::ZERO,
    )
    .unwrap_err();
    assert!(matches!(err, JoinError::Ticket(_)), "{err}");
    assert!(fake.drain().is_empty());
    assert!(sessions.sessions.is_empty());
}

#[test]
fn joined_snapshot_adopts_over_placeholder_serves_and_opens() {
    let fake = fake();
    let mut sessions = Sessions::default();
    let mut registry = ca::Registry::default();
    let session = SessionId::generate();
    let ticket = ticket(session, "jam").to_string();
    let (_, branch) = join(
        &mut sessions,
        &mut registry,
        &fake.handle,
        &ticket,
        Duration::ZERO,
    )
    .unwrap();
    let placeholder = registry.head(&branch).unwrap();
    fake.drain();

    // The host's snapshot: one root commit.
    let g = graph(&[1]);
    let host_commit = Commit::new(Duration::from_secs(1), None, ca::graph_addr(&g));
    let tip = ca::commit_addr(&host_commit);
    let effects = fake.deliver(
        &mut sessions,
        &mut registry,
        &OpenHeads::default(),
        Event::Joined {
            session,
            heads: vec![(branch.clone(), tip)],
            objects: objects(&host_commit, &g),
        },
    );

    assert_eq!(registry.head(&branch), Some(tip));
    let state = &sessions.sessions[&session];
    assert_eq!(state.placeholder, None);
    assert_eq!(state.last_announced[&branch], tip);
    assert!(matches!(
        effects[0],
        Effect::Moved { from: Some(from), to, .. } if from == placeholder && to == tip
    ));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Open(n) if *n == branch))
    );
    assert!(matches!(
        effects.last(),
        Some(Effect::Joined {
            commits: 1,
            graphs: 1,
            ..
        })
    ));
    // The adopted closure is served onward.
    let cmds = fake.drain();
    assert!(
        cmds.iter().any(
            |c| matches!(c, Command::Update { heads, .. } if heads == &[(branch.clone(), tip)])
        ),
        "{cmds:?}"
    );
    assert!(!sessions.dirty, "adoptions are not re-announced");
}

#[test]
fn known_uptodate_tips_are_dropped() {
    let fake = fake();
    let mut sessions = Sessions::default();
    let mut registry = ca::Registry::default();
    let session = SessionId::generate();
    guest(&mut sessions, session, "jam");
    let (a, _) = commit(&mut registry, None, graph(&[1]), 1);
    registry.set_head(name("jam"), a);

    let effects = fake.deliver(
        &mut sessions,
        &mut registry,
        &OpenHeads::default(),
        tips(session, peer(2), name("jam"), a),
    );
    assert!(effects.is_empty(), "{effects:?}");
    assert!(fake.drain().is_empty());
    assert!(sessions.sessions[&session].pending.is_empty());
}

#[test]
fn tips_start_a_fetch_and_objects_fast_forward_the_head() {
    let fake = fake();
    let mut sessions = Sessions::default();
    let mut registry = ca::Registry::default();
    let session = SessionId::generate();
    guest(&mut sessions, session, "jam");
    let (a, _) = commit(&mut registry, None, graph(&[1]), 1);
    registry.set_head(name("jam"), a);

    // A remote child of `a` the registry lacks.
    let g = graph(&[1, 2]);
    let b_commit = Commit::new(Duration::from_secs(2), Some(a), ca::graph_addr(&g));
    let b = ca::commit_addr(&b_commit);
    let effects = fake.deliver(
        &mut sessions,
        &mut registry,
        &OpenHeads::default(),
        tips(session, peer(2), name("jam"), b),
    );
    assert!(effects.is_empty(), "{effects:?}");
    let cmds = fake.drain();
    assert!(
        matches!(&cmds[..], [Command::Fetch { from, want, .. }] if *from == peer(2) && !want.is_empty()),
        "{cmds:?}"
    );
    assert!(
        sessions.sessions[&session]
            .pending
            .contains_key(&name("jam"))
    );

    let effects = fake.deliver(
        &mut sessions,
        &mut registry,
        &OpenHeads::default(),
        Event::Objects {
            session,
            from: peer(2),
            objects: objects(&b_commit, &g),
        },
    );
    assert_eq!(registry.head(&name("jam")), Some(b));
    assert!(matches!(
        effects[..],
        [Effect::Moved { from: Some(from), to, .. }, Effect::ResyncRefs] if from == a && to == b
    ));
    assert!(sessions.sessions[&session].pending.is_empty());
    assert!(!sessions.dirty, "fast-forwards are not re-announced");
    assert_eq!(sessions.sessions[&session].last_announced[&name("jam")], b);
}

#[test]
fn open_head_routes_to_remote_tip_without_moving_the_head() {
    let fake = fake();
    let mut sessions = Sessions::default();
    let mut registry = ca::Registry::default();
    let session = SessionId::generate();
    guest(&mut sessions, session, "jam");
    let (a, _) = commit(&mut registry, None, graph(&[1]), 1);
    registry.set_head(name("jam"), a);
    let g = graph(&[1, 2]);
    let b_commit = Commit::new(Duration::from_secs(2), Some(a), ca::graph_addr(&g));
    let b = ca::commit_addr(&b_commit);
    let open: OpenHeads = [(name("jam"), None)].into_iter().collect();

    fake.deliver(
        &mut sessions,
        &mut registry,
        &open,
        tips(session, peer(2), name("jam"), b),
    );
    let effects = fake.deliver(
        &mut sessions,
        &mut registry,
        &open,
        Event::Objects {
            session,
            from: peer(2),
            objects: objects(&b_commit, &g),
        },
    );
    assert!(
        matches!(
            &effects[..],
            [Effect::RemoteTip { name: n, remote, adopt_unrelated: false, .. }]
                if *n == name("jam") && *remote == b
        ),
        "{effects:?}"
    );
    assert_eq!(registry.head(&name("jam")), Some(a), "the host moves it");
    assert!(
        registry.commits().contains_key(&b),
        "the closure is applied"
    );
}

#[test]
fn background_merge_marks_dirty_and_is_announced() {
    let fake = fake();
    let mut sessions = Sessions::default();
    let mut registry = ca::Registry::default();
    let session = SessionId::generate();
    guest(&mut sessions, session, "jam");
    let (a, _) = commit(&mut registry, None, graph(&[1]), 1);
    let (b, _) = commit(&mut registry, Some(a), graph(&[1, 2]), 2);
    registry.set_head(name("jam"), b);

    // A remote sibling of `b`, diverged from `a` with a different node.
    let g = graph(&[1, 3]);
    let c_commit = Commit::new(Duration::from_secs(3), Some(a), ca::graph_addr(&g));
    let c = ca::commit_addr(&c_commit);
    fake.deliver(
        &mut sessions,
        &mut registry,
        &OpenHeads::default(),
        tips(session, peer(2), name("jam"), c),
    );
    fake.drain();
    let effects = fake.deliver(
        &mut sessions,
        &mut registry,
        &OpenHeads::default(),
        Event::Objects {
            session,
            from: peer(2),
            objects: objects(&c_commit, &g),
        },
    );
    let merged = registry.head(&name("jam")).unwrap();
    assert!(merged != b && merged != c, "a merge commit was minted");
    let merge = registry.commits().get(&merged).unwrap();
    assert_eq!(merge.parents().collect::<Vec<_>>(), vec![b, c]);
    assert!(matches!(
        effects[..],
        [Effect::Moved { from: Some(from), to, .. }, Effect::ResyncRefs] if from == b && to == merged
    ));
    assert!(
        sessions.dirty,
        "a minted merge is local content to announce"
    );

    let announced = announce(&mut sessions, &registry, &fake.handle, peer(1));
    assert_eq!(announced, vec![(session, name("jam"), merged)]);
    assert!(!sessions.dirty);
    let cmds = fake.drain();
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Broadcast { msg: GossipMsg::Tips { changed, .. }, .. }
                if changed.iter().any(|(n, t, _)| *n == name("jam") && *t == merged)
        )),
        "{cmds:?}"
    );
}

#[test]
fn fetch_without_progress_is_dropped() {
    let fake = fake();
    let mut sessions = Sessions::default();
    let mut registry = ca::Registry::default();
    let session = SessionId::generate();
    guest(&mut sessions, session, "jam");
    let (a, _) = commit(&mut registry, None, graph(&[1]), 1);
    registry.set_head(name("jam"), a);
    let g = graph(&[1, 2]);
    let b = ca::commit_addr(&Commit::new(
        Duration::from_secs(2),
        Some(a),
        ca::graph_addr(&g),
    ));

    fake.deliver(
        &mut sessions,
        &mut registry,
        &OpenHeads::default(),
        tips(session, peer(2), name("jam"), b),
    );
    assert_eq!(fake.drain().len(), 1, "one fetch");
    let effects = fake.deliver(
        &mut sessions,
        &mut registry,
        &OpenHeads::default(),
        Event::Objects {
            session,
            from: peer(2),
            objects: Objects { objects: vec![] },
        },
    );
    assert!(effects.is_empty());
    assert!(fake.drain().is_empty(), "no retry fetch");
    assert!(sessions.sessions[&session].pending.is_empty());
    assert_eq!(registry.head(&name("jam")), Some(a));
}

#[test]
fn announce_skips_clean_sessions_and_suppresses_echo() {
    let fake = fake();
    let mut sessions = Sessions::default();
    let mut registry = ca::Registry::default();
    let session = SessionId::generate();
    guest(&mut sessions, session, "jam");
    let (a, _) = commit(&mut registry, None, graph(&[1]), 1);
    registry.set_head(name("jam"), a);

    assert!(announce(&mut sessions, &registry, &fake.handle, peer(1)).is_empty());
    assert!(fake.drain().is_empty(), "clean sessions send nothing");

    // An adopted tip is served but not re-announced.
    sessions
        .sessions
        .get_mut(&session)
        .unwrap()
        .last_announced
        .insert(name("jam"), a);
    sessions.dirty = true;
    assert!(announce(&mut sessions, &registry, &fake.handle, peer(1)).is_empty());
    let cmds = fake.drain();
    assert!(
        cmds.iter().all(|c| matches!(c, Command::Update { .. })),
        "{cmds:?}"
    );

    // A local commit is announced once.
    let (b, _) = commit(&mut registry, Some(a), graph(&[1, 2]), 2);
    registry.set_head(name("jam"), b);
    sessions.dirty = true;
    let announced = announce(&mut sessions, &registry, &fake.handle, peer(1));
    assert_eq!(announced, vec![(session, name("jam"), b)]);
    let cmds = fake.drain();
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, Command::Broadcast { .. }))
            .count(),
        1,
        "{cmds:?}"
    );
    sessions.dirty = true;
    assert!(announce(&mut sessions, &registry, &fake.handle, peer(1)).is_empty());
}

#[test]
fn share_registers_serves_and_shares() {
    let fake = fake();
    let mut sessions = Sessions::default();
    let mut registry = ca::Registry::default();
    let (a, _) = commit(&mut registry, None, graph(&[1]), 1);
    registry.set_head(name("jam"), a);

    let id = share(
        &mut sessions,
        &registry,
        &fake.handle,
        &name("jam"),
        Access::Public,
    );
    let cmds = fake.drain();
    assert!(
        matches!(&cmds[0], Command::Register(e) if e.session.role == Role::Host && e.session.id == id)
    );
    assert!(
        matches!(&cmds[1], Command::Update { heads, commits, graphs, .. }
        if heads == &[(name("jam"), a)] && commits.len() == 1 && graphs.len() == 1)
    );
    assert!(matches!(&cmds[2], Command::Share(s) if *s == id));
    assert_eq!(cmds.len(), 3);
    assert_eq!(sessions.sessions[&id].last_announced[&name("jam")], a);
}

#[test]
fn presence_pointers_and_peers_are_tracked() {
    let fake = fake();
    let mut sessions = Sessions::default();
    let mut registry = ca::Registry::default();
    let session = SessionId::generate();
    guest(&mut sessions, session, "jam");
    let open = OpenHeads::default();

    let effects = fake.deliver(
        &mut sessions,
        &mut registry,
        &open,
        Event::PeerUp {
            session,
            peer: peer(2),
        },
    );
    assert!(matches!(effects[..], [Effect::PeerUp { peer: p, .. }] if p == peer(2)));
    assert_eq!(
        sessions.sessions[&session].conn,
        gantz_collab::ConnState::Live
    );

    fake.deliver(
        &mut sessions,
        &mut registry,
        &open,
        Event::Gossip {
            session,
            from: peer(2),
            msg: GossipMsg::Presence {
                origin: peer(2),
                name: Some("ada".to_string()),
            },
        },
    );
    assert_eq!(
        sessions.sessions[&session].peers[&peer(2)],
        Some("ada".to_string())
    );

    let pointer = |seq, pos| Event::Gossip {
        session,
        from: peer(2),
        msg: GossipMsg::Pointer {
            origin: peer(2),
            seq,
            name: name("jam"),
            pos,
        },
    };
    fake.deliver(
        &mut sessions,
        &mut registry,
        &open,
        pointer(2, Some((1.0, 2.0))),
    );
    fake.deliver(
        &mut sessions,
        &mut registry,
        &open,
        pointer(1, Some((9.0, 9.0))),
    );
    let p = &sessions.sessions[&session].pointers[&peer(2)];
    assert_eq!((p.seq, p.pos), (2, Some((1.0, 2.0))), "stale updates drop");

    let effects = fake.deliver(
        &mut sessions,
        &mut registry,
        &open,
        Event::PeerDown {
            session,
            peer: peer(2),
        },
    );
    assert!(matches!(effects[..], [Effect::PeerDown { .. }]));
    let state = &sessions.sessions[&session];
    assert!(state.peers.is_empty() && state.pointers.is_empty());
    assert_eq!(state.conn, gantz_collab::ConnState::Degraded);
}
