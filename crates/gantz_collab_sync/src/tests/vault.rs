//! Devices syncing through an in-process vault. The vault answers fetches
//! from its registry with `gantz_collab::store::objects`, applies pushes
//! with [`serve_push`] and tells every linked device what moved.

use super::{Fake, commit, fake, graph, name};
use crate::*;
use gantz_ca as ca;
use gantz_collab::{
    Command, Event, ObjectRef, Outdated, PROTO_MAX, PROTO_MIN, PairingSecret, PeerId, Push,
    SessionId, VaultId, VaultTicket, VersionInfo,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// One device: its runtime stand-in, sync state and registry.
struct Device {
    fake: Fake,
    sessions: Sessions,
    registry: ca::Registry,
    open: OpenHeads,
    /// Events for this device, delivered on its next step.
    inbox: VecDeque<Event>,
    /// Every command this device sent, for counting round trips.
    sent: Vec<Command>,
}

/// The vault and its devices.
struct Net {
    id: VaultId,
    peer: PeerId,
    registry: ca::Registry,
    devices: Vec<Device>,
}

impl Device {
    fn link(&self) -> &VaultLink {
        self.sessions.vault.as_ref().expect("linked")
    }

    /// Heads outside `local_only`.
    fn heads(&self) -> BTreeMap<ca::Name, ca::CommitAddr> {
        let local_only = &self.link().local_only;
        self.registry
            .heads()
            .filter(|(n, _)| !local_only.contains(*n))
            .map(|(n, ca)| (n.clone(), ca))
            .collect()
    }
}

impl Net {
    fn new(devices: usize) -> Self {
        let peer = gantz_collab::Identity::generate().peer_id();
        let devices = (0..devices)
            .map(|_| Device {
                fake: fake(),
                sessions: Sessions::default(),
                registry: ca::Registry::default(),
                open: OpenHeads::default(),
                inbox: VecDeque::new(),
                sent: Vec::new(),
            })
            .collect();
        Self {
            id: SessionId::generate(),
            peer,
            registry: ca::Registry::default(),
            devices,
        }
    }

    fn heads(&self) -> BTreeMap<ca::Name, ca::CommitAddr> {
        self.registry
            .heads()
            .map(|(n, ca)| (n.clone(), ca))
            .collect()
    }

    /// Link device `d`, never synced before, and bring its link up.
    fn link(&mut self, d: usize, local_only: &[&str]) {
        let id = iroh::EndpointId::from_bytes(&self.peer.0).unwrap();
        let ticket = VaultTicket {
            vault: self.id,
            pairing: PairingSecret::generate(),
            host: id.into(),
        };
        let local_only: BTreeSet<ca::Name> = local_only.iter().map(|n| name(n)).collect();
        let device = &mut self.devices[d];
        vault::link(
            &mut device.sessions,
            &device.fake.handle,
            ticket,
            BTreeMap::new(),
            local_only,
        )
        .unwrap();
        assert!(matches!(device.fake.drain()[..], [Command::Link(_)]));
        let heads = self.heads().into_iter().collect();
        let up = Event::LinkUp {
            vault: self.id,
            heads,
            info: Default::default(),
        };
        self.devices[d].inbox.push_back(up);
    }

    /// Deliver device `d`'s inbox, poll it, and answer what it sent.
    /// Returns the effects and whether anything happened.
    fn step(&mut self, d: usize) -> (Vec<Effect>, bool) {
        let device = &mut self.devices[d];
        let mut effects = Vec::new();
        let mut busy = !device.inbox.is_empty();
        while let Some(event) = device.inbox.pop_front() {
            device.fake.events.send_blocking(event).unwrap();
        }
        let handle = device.fake.handle.clone();
        effects.extend(poll(
            &mut device.sessions,
            &mut device.registry,
            &handle,
            &device.open,
        ));
        let cmds = device.fake.drain();
        busy |= !cmds.is_empty();
        for cmd in cmds {
            self.answer(d, &cmd);
            self.devices[d].sent.push(cmd);
        }
        (effects, busy)
    }

    /// Answer one command from device `d` as the vault would.
    fn answer(&mut self, d: usize, cmd: &Command) {
        match cmd {
            Command::Fetch { want, .. } => {
                let objects = gantz_collab::store::objects(&self.registry, want);
                let answer = Event::Objects {
                    session: self.id,
                    from: self.peer,
                    want: want.clone(),
                    objects,
                };
                self.devices[d].inbox.push_back(answer);
            }
            Command::Push { push, .. } => {
                let (name, tip) = (push.name.clone(), push.tip);
                let result = match serve_push(&mut self.registry, self.id, push.clone()) {
                    PushOutcome::Accepted(Command::UpdateVault { heads, .. }) => {
                        let changed = Event::LinkChanged {
                            vault: self.id,
                            changes: heads,
                        };
                        for device in &mut self.devices {
                            if device.sessions.vault.is_some() {
                                device.inbox.push_back(clone_changed(&changed));
                            }
                        }
                        Ok(tip)
                    }
                    PushOutcome::Accepted(_) => unreachable!("an update"),
                    PushOutcome::Stale(head) => Ok(head),
                    PushOutcome::Invalid(reason) => Err(reason),
                };
                let pushed = Event::Pushed {
                    vault: self.id,
                    name,
                    tip,
                    result,
                };
                self.devices[d].inbox.push_back(pushed);
            }
            _ => (),
        }
    }

    /// Step every linked device until nothing happens.
    fn settle(&mut self) {
        for round in 0.. {
            assert!(round < 100, "vault sync failed to settle");
            let mut busy = false;
            for d in 0..self.devices.len() {
                if self.devices[d].sessions.vault.is_some() {
                    busy |= self.step(d).1;
                }
            }
            if !busy {
                return;
            }
        }
    }

    fn assert_converged(&self) {
        let expected = self.heads();
        for (d, device) in self.devices.iter().enumerate() {
            assert_eq!(device.heads(), expected, "device {d} heads");
            assert_eq!(device.link().synced, expected, "device {d} synced");
        }
    }
}

fn clone_changed(event: &Event) -> Event {
    let Event::LinkChanged { vault, changes } = event else {
        unreachable!("a change notice");
    };
    Event::LinkChanged {
        vault: *vault,
        changes: changes.clone(),
    }
}

/// Add node `id` to `n`'s graph on device `d`, creating the name when absent.
fn edit(net: &mut Net, d: usize, n: &str, id: u32, secs: u64) -> ca::CommitAddr {
    let registry = &mut net.devices[d].registry;
    let parent = registry.head(&name(n));
    let mut g = parent
        .and_then(|ca| registry.commit_graph_ref(&ca))
        .cloned()
        .unwrap_or_default();
    g.add_node(graph(&[id]).node_weights().next().unwrap().clone());
    let (ca, _) = commit(registry, parent, g, secs);
    registry.set_head(name(n), ca);
    ca
}

/// The node ids in `n`'s graph on the vault.
fn vault_ids(net: &Net, n: &str) -> Vec<String> {
    let head = net.registry.head(&name(n)).unwrap();
    let mut ids: Vec<String> = net
        .registry
        .commit_graph_ref(&head)
        .unwrap()
        .node_weights()
        .map(|n| format!("{:?}", n.data))
        .collect();
    ids.sort();
    ids
}

#[test]
fn first_link_uploads_local_graphs_and_a_second_device_adopts_them() {
    let mut net = Net::new(2);
    let jam = edit(&mut net, 0, "jam", 1, 1);
    net.link(0, &[]);
    net.settle();
    assert_eq!(net.registry.head(&name("jam")), Some(jam));
    assert!(net.devices[0].link().synced_changed);
    net.link(1, &[]);
    net.settle();
    net.assert_converged();
    assert_eq!(net.devices[1].link().status, VaultStatus::Live);
}

#[test]
fn offline_edits_on_two_devices_merge() {
    let mut net = Net::new(2);
    edit(&mut net, 0, "jam", 1, 1);
    net.link(0, &[]);
    net.link(1, &[]);
    net.settle();
    edit(&mut net, 0, "jam", 2, 10);
    edit(&mut net, 1, "jam", 3, 20);
    net.settle();
    net.assert_converged();
    assert_eq!(vault_ids(&net, "jam").len(), 3);
}

#[test]
fn deletes_propagate_and_edits_beat_concurrent_deletes() {
    let mut net = Net::new(2);
    edit(&mut net, 0, "gone", 1, 1);
    edit(&mut net, 0, "kept", 2, 2);
    net.link(0, &[]);
    net.link(1, &[]);
    net.settle();
    net.devices[1].registry.remove_head(&name("gone"));
    net.devices[1].registry.remove_head(&name("kept"));
    let edited = edit(&mut net, 0, "kept", 3, 10);
    net.settle();
    net.assert_converged();
    assert_eq!(net.registry.head(&name("gone")), None);
    assert_eq!(net.registry.head(&name("kept")), Some(edited));
}

#[test]
fn unrelated_graphs_of_one_name_move_aside_with_their_nested_graphs() {
    let mut net = Net::new(2);
    edit(&mut net, 0, "jam", 1, 1);
    edit(&mut net, 0, "jam:child", 2, 1);
    let theirs = edit(&mut net, 1, "jam", 3, 2);
    edit(&mut net, 1, "jam:child", 4, 2);
    net.link(0, &[]);
    net.settle();
    let ours = net.registry.head(&name("jam")).unwrap();
    net.link(1, &[]);
    net.settle();
    net.assert_converged();
    let aside = name(&format!("jam-{}", theirs.display_short()));
    assert_eq!(net.registry.head(&name("jam")), Some(ours));
    assert_eq!(net.registry.head(&aside), Some(theirs));
    assert!(net.registry.head(&aside.child("child")).is_some());
    assert_eq!(net.heads().len(), 4);
}

#[test]
fn a_long_offline_history_arrives_in_one_fetch() {
    let mut net = Net::new(2);
    net.link(0, &[]);
    net.link(1, &[]);
    net.settle();
    for i in 0..200 {
        edit(&mut net, 0, "jam", i, u64::from(i) + 1);
    }
    net.settle();
    net.assert_converged();
    let fetches = net.devices[1]
        .sent
        .iter()
        .filter(|c| matches!(c, Command::Fetch { .. }))
        .count();
    assert_eq!(fetches, 1);
}

#[test]
fn local_only_names_never_sync() {
    let mut net = Net::new(2);
    edit(&mut net, 0, "base", 1, 1);
    edit(&mut net, 1, "base", 2, 2);
    edit(&mut net, 0, "jam", 3, 3);
    net.link(0, &["base"]);
    net.link(1, &["base"]);
    net.settle();
    net.assert_converged();
    assert_eq!(net.registry.head(&name("base")), None);
    assert_ne!(
        net.devices[0].registry.head(&name("base")),
        net.devices[1].registry.head(&name("base"))
    );
}

#[test]
fn a_failed_fetch_holds_its_name_until_the_vault_moves() {
    let mut net = Net::new(2);
    edit(&mut net, 0, "jam", 1, 1);
    net.link(0, &[]);
    net.settle();
    // Device 1 links but its fetch fails.
    let id = iroh::EndpointId::from_bytes(&net.peer.0).unwrap();
    let ticket = VaultTicket {
        vault: net.id,
        pairing: PairingSecret::generate(),
        host: id.into(),
    };
    let heads = net.heads().into_iter().collect();
    let device = &mut net.devices[1];
    vault::link(
        &mut device.sessions,
        &device.fake.handle,
        ticket,
        BTreeMap::new(),
        BTreeSet::new(),
    )
    .unwrap();
    device.fake.drain();
    let up = Event::LinkUp {
        vault: net.id,
        heads,
        info: Default::default(),
    };
    let effects = device
        .fake
        .deliver(&mut device.sessions, &mut device.registry, &device.open, up);
    assert!(effects.is_empty());
    let want = device.fake.fetch_want();
    let failed = Event::FetchFailed {
        session: net.id,
        from: net.peer,
        want,
        error: "gone".to_string(),
    };
    device.fake.deliver(
        &mut device.sessions,
        &mut device.registry,
        &device.open,
        failed,
    );
    assert!(device.fake.drain().is_empty(), "held names do not refetch");
    // The vault moves, so the name is fetched again.
    let tip = edit(&mut net, 0, "jam", 2, 5);
    net.step(0);
    assert_eq!(net.registry.head(&name("jam")), Some(tip));
    let device = &mut net.devices[1];
    let changed = Event::LinkChanged {
        vault: net.id,
        changes: vec![(name("jam"), Some(tip))],
    };
    device.fake.deliver(
        &mut device.sessions,
        &mut device.registry,
        &device.open,
        changed,
    );
    let want = device.fake.fetch_want();
    assert!(matches!(&want.refs[0], ObjectRef::Closure { tips, .. } if tips == &vec![tip]));
}

#[test]
fn open_names_move_through_the_host_once() {
    let mut net = Net::new(2);
    edit(&mut net, 0, "jam", 1, 1);
    net.link(0, &[]);
    net.link(1, &[]);
    net.settle();
    net.devices[1].open.insert(name("jam"), None);
    let tip = edit(&mut net, 0, "jam", 2, 5);
    net.step(0);
    net.step(0);
    let mut resets = Vec::new();
    for _ in 0..4 {
        let (effects, _) = net.step(1);
        resets.extend(effects.into_iter().filter_map(|e| match e {
            Effect::Reset { name, to } => Some((name, to)),
            _ => None,
        }));
    }
    assert_eq!(resets, vec![(name("jam"), Some(tip))]);
    // The host moves the open head, and the name is back in step.
    net.devices[1].registry.set_head(name("jam"), tip);
    net.settle();
    net.assert_converged();
}

#[test]
fn serve_push_refuses_stale_and_incomplete_pushes() {
    let mut vault = ca::Registry::default();
    let id = SessionId::generate();
    let (root, _) = commit(&mut vault, None, graph(&[1]), 1);
    vault.set_head(name("jam"), root);
    let mut device = vault.clone();
    let (tip, _) = commit(&mut device, Some(root), graph(&[1, 2]), 2);
    let stale = Push {
        name: name("jam"),
        tip: Some(tip),
        base: None,
        objects: Default::default(),
    };
    let outcome = serve_push(&mut vault, id, stale);
    assert!(matches!(outcome, PushOutcome::Stale(Some(h)) if h == root));
    let incomplete = Push {
        name: name("jam"),
        tip: Some(tip),
        base: Some(root),
        objects: Default::default(),
    };
    assert!(matches!(
        serve_push(&mut vault, id, incomplete),
        PushOutcome::Invalid(_)
    ));
    let complete = Push {
        name: name("jam"),
        tip: Some(tip),
        base: Some(root),
        objects: gantz_collab::store::closure(&device, &[tip], &[root]),
    };
    assert!(matches!(
        serve_push(&mut vault, id, complete),
        PushOutcome::Accepted(_)
    ));
    assert_eq!(vault.head(&name("jam")), Some(tip));
}

// A removal carries no content, so whatever objects ride with it are not
// mirrored into the served store.
#[test]
fn serve_push_mirrors_no_content_for_a_removal() {
    let mut vault = ca::Registry::default();
    let id = SessionId::generate();
    let (root, _) = commit(&mut vault, None, graph(&[1]), 1);
    vault.set_head(name("jam"), root);
    let mut device = vault.clone();
    let (other, _) = commit(&mut device, None, graph(&[2]), 2);
    let removal = Push {
        name: name("jam"),
        tip: None,
        base: Some(root),
        objects: gantz_collab::store::closure(&device, &[other], &[]),
    };
    let PushOutcome::Accepted(Command::UpdateVault {
        commits, graphs, ..
    }) = serve_push(&mut vault, id, removal)
    else {
        panic!("the removal was refused");
    };
    assert!(commits.is_empty() && graphs.is_empty());
    assert_eq!(vault.head(&name("jam")), None);
}

#[test]
fn an_incompatible_vault_pauses_sync_and_says_who_must_update() {
    let mut net = Net::new(1);
    net.link(0, &[]);
    net.settle();
    let info = |proto, app: &str| VersionInfo {
        proto_min: proto,
        proto_max: proto,
        app: app.to_string(),
    };
    let ours = info(PROTO_MAX, "gantz 0.4.0");
    let cases = [
        (
            info(PROTO_MAX + 1, "gantz 9.0.0"),
            Outdated::Us,
            VaultStatus::DeviceOutdated,
        ),
        (
            info(PROTO_MIN - 1, "gantz 0.1.0"),
            Outdated::Them,
            VaultStatus::VaultOutdated,
        ),
    ];
    for (theirs, outdated, status) in cases {
        let incompatible = Event::LinkIncompatible {
            vault: net.id,
            theirs: theirs.clone(),
            outdated,
        };
        net.devices[0].inbox.push_back(incompatible);
        net.step(0);
        let link = net.devices[0].link();
        assert_eq!(link.status, status);
        assert_eq!(link.vault_info.as_ref(), Some(&theirs));
    }
    // Local edits wait for the vault.
    edit(&mut net, 0, "jam", 1, 1);
    net.settle();
    assert!(net.devices[0].sent.is_empty());
    assert_eq!(net.registry.head(&name("jam")), None);
    // Once the vault updates, the edit syncs.
    let up = Event::LinkUp {
        vault: net.id,
        heads: vec![],
        info: ours,
    };
    net.devices[0].inbox.push_back(up);
    net.settle();
    net.assert_converged();
    assert_eq!(net.devices[0].link().status, VaultStatus::Live);
}

#[test]
fn a_refused_push_is_recorded_until_the_name_syncs() {
    let mut net = Net::new(1);
    edit(&mut net, 0, "jam", 1, 1);
    net.link(0, &[]);
    // The link comes up and the device pushes, but the vault refuses.
    let device = &mut net.devices[0];
    let up = device.inbox.pop_front().unwrap();
    device
        .fake
        .deliver(&mut device.sessions, &mut device.registry, &device.open, up);
    let cmds = device.fake.drain();
    let [Command::Push { push, .. }] = &cmds[..] else {
        panic!("expected one push, got {cmds:?}");
    };
    let refused = Event::Pushed {
        vault: net.id,
        name: push.name.clone(),
        tip: push.tip,
        result: Err("the vault is full".to_string()),
    };
    device.fake.deliver(
        &mut device.sessions,
        &mut device.registry,
        &device.open,
        refused,
    );
    let failure = device.link().failures.get(&name("jam")).cloned();
    assert_eq!(failure.as_deref(), Some("push failed: the vault is full"));
    assert!(device.fake.drain().is_empty(), "a failed name is held");
    // A later edit pushes again, and its acceptance clears the failure.
    edit(&mut net, 0, "jam", 2, 2);
    net.settle();
    net.assert_converged();
    assert!(net.devices[0].link().failures.is_empty());
}
