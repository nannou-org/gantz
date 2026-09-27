use crate::join::{Peer, now};
use gantz_ca as ca;
use gantz_collab::{
    Access, Command, Event, GossipMsg, Handle, Identity, Infra, Object, ObjectRef, Role,
    RuntimeConfig, Session, SessionEntry, SessionId, SessionRegistry, Want, store,
};
use std::time::{Duration, Instant};

/// Wait for an event matching `pred` on a raw runtime.
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

/// Step the peer until `done`.
fn step_until(peer: &mut Peer, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out stepping the peer");
        peer.step(now(), true).unwrap();
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A raw host sharing `jam`, the peer joining it into a directory, an
/// edit to the mirrored file reaching the host as a tip, and the host
/// fetching the commit from the peer.
#[test]
#[ignore = "binds real sockets and may touch n0 discovery infrastructure"]
fn edits_to_a_mirrored_file_reach_the_host() {
    round_trip(Infra::N0);
}

/// The same over ticket addresses alone, with no relay and no address
/// lookup.
#[test]
#[ignore = "binds real sockets"]
fn edits_reach_the_host_without_relays() {
    round_trip(Infra::Custom {
        relays: vec![],
        pkarr: None,
    });
}

fn round_trip(infra: Infra) {
    let codec = crate::node::codec();
    let dir = std::env::temp_dir().join(format!(
        "gantz-join-{}-{}",
        std::process::id(),
        now().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    // The host's store: `jam` at one commit, from real nodes.
    let text = "(graph jam (b bang))";
    let parsed =
        gantz_egui::export::parse_export_at(text.as_bytes(), Duration::from_secs(1), &codec)
            .unwrap();
    let jam: ca::Name = "jam".parse().unwrap();
    let tip = parsed.head(&jam).unwrap();
    let commit = parsed.commits()[&tip].clone();
    let graph = parsed.graphs()[&commit.graph].clone();
    let session_id = SessionId::generate();
    let host = gantz_collab::spawn(
        Identity::generate(),
        RuntimeConfig {
            infra: infra.clone(),
        },
    );
    wait_for(&host, |e| matches!(e, Event::Ready { .. }).then_some(()));
    let mut served = SessionRegistry::default();
    store::merge(
        &mut served,
        [(jam.clone(), tip)],
        [(tip, commit.clone())],
        [(commit.graph, graph)],
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
                resolutions: gantz_collab_sync::session_resolutions(),
                role: Role::Host,
            },
            store: served,
        }))
        .unwrap();
    host.cmds.send_blocking(Command::Share(session_id)).unwrap();
    let ticket = wait_for(&host, |e| match e {
        Event::TicketReady { ticket, .. } => Some(ticket),
        _ => None,
    });

    // The peer joins and mirrors `jam` to disk.
    let mut peer = Peer::new(Identity::generate(), infra, dir.clone(), codec);
    peer.join(&ticket, now()).unwrap();
    let path = dir.join("jam.gantz");
    step_until(&mut peer, || path.exists());
    let mirrored = std::fs::read_to_string(&path).unwrap();
    assert!(mirrored.contains("(graph jam"), "{mirrored}");

    // An edit to the file becomes a commit the host hears about.
    let edited = mirrored.replacen("(bang0 bang)", "(bang0 bang)\n  (bang1 bang)", 1);
    assert_ne!(edited, mirrored);
    std::fs::write(&path, &edited).unwrap();
    let mut new_tip = None;
    step_until(&mut peer, || {
        new_tip = new_tip.or_else(|| {
            host.events.try_recv().ok().and_then(|e| match e {
                Event::Gossip {
                    msg: GossipMsg::Tips { changed, .. },
                    ..
                } => changed
                    .into_iter()
                    .find(|(n, t, _)| *n == jam && *t != tip)
                    .map(|(_, t, _)| t),
                _ => None,
            })
        });
        new_tip.is_some()
    });
    let new_tip = new_tip.unwrap();

    // The host fetches the new commit from the peer.
    host.cmds
        .send_blocking(Command::Fetch {
            session: session_id,
            from: peer.peer_id(),
            want: Want {
                refs: vec![ObjectRef::Commit(new_tip)],
            },
        })
        .unwrap();
    let mut fetched = false;
    step_until(&mut peer, || {
        fetched = fetched
            || host.events.try_recv().is_ok_and(|e| {
                match e {
            Event::Objects { objects, .. } => objects.objects.iter().any(
                |o| matches!(o, Object::Commit(ca, w) if *ca == new_tip && w.parent == Some(tip)),
            ),
            _ => false,
        }
            });
        fetched
    });

    std::fs::remove_dir_all(&dir).unwrap();
}
