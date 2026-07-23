//! End-to-end share/join/fetch between two runtimes in one process.
//!
//! Binds real sockets and may touch iroh's discovery/relay infrastructure,
//! so it is ignored by default; run manually with
//! `cargo test -p gantz_collab -- --ignored`.

use gantz_ca::{
    BlobLiveness, Commit, DataGraph, Datum, Key, Liveness, MergePolicy, Name, NodeData, Value,
    blob_addr, commit_addr,
};
use gantz_collab::{
    Access, Command, Event, Handle, Identity, Object, ObjectRef, PeerId, Role, Session,
    SessionEntry, SessionId, SessionRegistry, SessionTicket, Want, store,
};
use std::time::{Duration, Instant};

/// Wait for an event matching `pred`, panicking after a generous deadline.
fn wait_for<T>(handle: &Handle, mut pred: impl FnMut(Event) -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match handle.events.try_recv() {
            Ok(event) => {
                if let Some(t) = pred(event) {
                    return t;
                }
            }
            Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "timed out waiting for event");
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(async_channel::TryRecvError::Closed) => panic!("runtime closed its events"),
        }
    }
}

#[test]
#[ignore = "binds real sockets and may touch n0 discovery infrastructure"]
fn share_join_and_fetch_between_two_runtimes() {
    let session_id = SessionId::generate();
    let mut graph = DataGraph::default();
    graph.add_node(NodeData::new("test", Datum::Map(vec![])));
    let graph_ca = gantz_ca::graph_addr(&graph);
    let graph_blob = gantz_collab::proto::encode_graph(&graph);
    let commit = Commit::new(Duration::from_secs(1), None, graph_ca);
    let tip = commit_addr(&commit);
    let jam: Name = "jam".parse().unwrap();
    // A stored scene view for the tip and an audio-buffer blob, fed over the
    // announce path (`Command::Update`) rather than the initial store.
    let view = Value::Datum(Datum::Map(vec![("zoom".to_string(), Datum::F64(1.5))]));
    let view_object = Object::Section {
        id: "egui.view".to_string(),
        policy: MergePolicy::KeepExisting,
        liveness: Liveness::WithCommit,
        key: Key::Commit(tip),
        value: gantz_collab::proto::encode_value(&view),
    };
    let pcm = b"pcm".to_vec();
    let pcm_addr = blob_addr(&pcm);
    let pcm_object = Object::Blob {
        section: "dsp.buffer".to_string(),
        liveness: BlobLiveness::ContentReferenced,
        addr: pcm_addr,
        bytes: pcm.clone(),
    };

    // The host runtime serves one session with a single-commit store.
    let host = gantz_collab::spawn(Identity::generate(), Default::default());
    let host_peer = wait_for(&host, |e| match e {
        Event::Ready { peer } => Some(peer),
        _ => None,
    });
    let mut served = SessionRegistry::default();
    store::merge(
        &mut served,
        [(jam.clone(), tip)],
        [(tip, commit.clone())],
        [(graph_ca, graph.clone())],
        [],
        [],
    )
    .unwrap();
    host.cmds
        .send_blocking(Command::Register(SessionEntry {
            session: Session {
                id: session_id,
                branch: "jam".to_string(),
                access: Access::Public,
                resolutions: Default::default(),
                role: Role::Host,
            },
            store: served,
        }))
        .unwrap();
    // Mid-session content: the view and blob arrive as an update, not as
    // part of the registered store. Commands apply in send order, so both
    // are served before the ticket below can exist.
    host.cmds
        .send_blocking(Command::Update {
            session: session_id,
            heads: vec![],
            commits: vec![],
            graphs: vec![],
            sections: vec![(
                "egui.view".to_string(),
                MergePolicy::KeepExisting,
                Liveness::WithCommit,
                Key::Commit(tip),
                view.clone(),
            )],
            blobs: vec![(
                "dsp.buffer".to_string(),
                BlobLiveness::ContentReferenced,
                pcm_addr,
                pcm.clone().into(),
            )],
        })
        .unwrap();
    host.cmds.send_blocking(Command::Share(session_id)).unwrap();
    let ticket = wait_for(&host, |e| match e {
        Event::TicketReady { ticket, .. } => Some(ticket),
        _ => None,
    });
    let ticket: SessionTicket = ticket.parse().unwrap();
    assert_eq!(ticket.name, "jam");
    assert_eq!(PeerId(*ticket.hosts[0].id.as_bytes()), host_peer);

    // The guest joins from the ticket and receives the snapshot.
    let guest = gantz_collab::spawn(Identity::generate(), Default::default());
    wait_for(&guest, |e| match e {
        Event::Ready { .. } => Some(()),
        _ => None,
    });
    guest
        .cmds
        .send_blocking(Command::Register(SessionEntry {
            session: Session {
                id: session_id,
                branch: ticket.name.clone(),
                access: ticket.access.clone(),
                resolutions: ticket.resolutions,
                role: Role::Guest,
            },
            store: SessionRegistry::default(),
        }))
        .unwrap();
    guest.cmds.send_blocking(Command::Join(ticket)).unwrap();
    let (heads, objects) = wait_for(&guest, |e| match e {
        Event::Joined { heads, objects, .. } => Some((heads, objects)),
        Event::Error { message, .. } => panic!("join failed: {message}"),
        _ => None,
    });
    assert_eq!(heads, vec![(jam.clone(), tip)]);
    // Snapshot order: commits, graphs, blobs, then non-head section entries.
    assert_eq!(
        objects.objects,
        vec![
            Object::Commit(tip, commit.clone().into()),
            Object::Graph(graph_ca, graph_blob.clone()),
            pcm_object.clone(),
            view_object.clone(),
        ]
    );

    // A targeted fetch over the request plane returns the same objects; the
    // wanted commit's stored view piggybacks on the commit.
    guest
        .cmds
        .send_blocking(Command::Fetch {
            session: session_id,
            from: host_peer,
            want: Want {
                refs: vec![
                    ObjectRef::Commit(tip),
                    ObjectRef::Graph(graph_ca),
                    ObjectRef::Blob {
                        section: "dsp.buffer".to_string(),
                        addr: pcm_addr,
                    },
                ],
            },
        })
        .unwrap();
    let fetched = wait_for(&guest, |e| match e {
        Event::Objects { objects, from, .. } => {
            assert_eq!(from, host_peer);
            Some(objects)
        }
        Event::Error { message, .. } => panic!("fetch failed: {message}"),
        _ => None,
    });
    assert_eq!(
        fetched.objects,
        vec![
            Object::Commit(tip, commit.into()),
            view_object,
            Object::Graph(graph_ca, graph_blob),
            pcm_object,
        ]
    );
}

#[test]
#[ignore = "binds real sockets and may touch n0 discovery infrastructure"]
fn restricted_sessions_deny_unlisted_peers() {
    let session_id = SessionId::generate();
    let host = gantz_collab::spawn(Identity::generate(), Default::default());
    wait_for(&host, |e| match e {
        Event::Ready { .. } => Some(()),
        _ => None,
    });
    host.cmds
        .send_blocking(Command::Register(SessionEntry {
            session: Session {
                id: session_id,
                branch: "private".to_string(),
                // An empty allowlist: nobody may join.
                access: Access::Restricted(Default::default()),
                resolutions: Default::default(),
                role: Role::Host,
            },
            store: SessionRegistry::default(),
        }))
        .unwrap();
    host.cmds.send_blocking(Command::Share(session_id)).unwrap();
    let ticket = wait_for(&host, |e| match e {
        Event::TicketReady { ticket, .. } => Some(ticket),
        _ => None,
    });
    let ticket: SessionTicket = ticket.parse().unwrap();

    let guest = gantz_collab::spawn(Identity::generate(), Default::default());
    wait_for(&guest, |e| match e {
        Event::Ready { .. } => Some(()),
        _ => None,
    });
    guest.cmds.send_blocking(Command::Join(ticket)).unwrap();
    let message = wait_for(&guest, |e| match e {
        Event::Error { message, .. } => Some(message),
        Event::Joined { .. } => panic!("restricted join must be denied"),
        _ => None,
    });
    assert!(message.contains("denied"), "unexpected error: {message}");
}
