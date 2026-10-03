use crate::vault::{Dir, Vault};
use gantz_ca as ca;
use gantz_collab::{Handle, Identity, Infra, RuntimeConfig};
use gantz_collab_sync::{OpenHeads, Sessions};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime};

/// A device: its runtime, sync state and registry.
struct Device {
    handle: Handle,
    sessions: Sessions,
    registry: ca::Registry,
}

impl Device {
    fn new() -> Self {
        let config = RuntimeConfig {
            infra: local(),
            port: None,
        };
        Self {
            handle: gantz_collab::spawn(Identity::generate(), config),
            sessions: Sessions::default(),
            registry: ca::Registry::default(),
        }
    }

    fn link(&mut self, ticket: &str) {
        let (synced, local_only) = (BTreeMap::new(), BTreeSet::new());
        gantz_collab_sync::vault::link(
            &mut self.sessions,
            &self.handle,
            ticket.parse().unwrap(),
            synced,
            local_only,
        )
        .unwrap();
    }

    fn step(&mut self) {
        let open = OpenHeads::default();
        gantz_collab_sync::poll(&mut self.sessions, &mut self.registry, &self.handle, &open);
    }

    /// Add a node tagged `tag` to `name`'s graph, creating the name.
    fn edit(&mut self, name: &str, tag: &str) -> ca::CommitAddr {
        let name: ca::Name = name.parse().unwrap();
        let parent = self.registry.head(&name);
        let mut graph = parent
            .and_then(|ca| self.registry.commit_graph_ref(&ca))
            .cloned()
            .unwrap_or_default();
        graph.add_node(ca::NodeData::new(tag, ca::Datum::Map(vec![])));
        let ga = ca::graph_addr(&graph);
        let ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        let commit = self.registry.commit_graph(ts, parent, ga, || graph);
        self.registry.set_head(name, commit);
        commit
    }

    fn head(&self, name: &str) -> Option<ca::CommitAddr> {
        self.registry.head(&name.parse().unwrap())
    }
}

impl Vault {
    fn head(&self, name: &str) -> Option<ca::CommitAddr> {
        self.registry.head(&name.parse().unwrap())
    }
}

/// Relay-free infrastructure, so the test needs no network services.
fn local() -> Infra {
    Infra::Custom {
        relays: vec![],
        pkarr: None,
    }
}

/// Step the vault and devices until `done`.
fn step_until(
    vault: &mut Vault,
    devices: &mut [&mut Device],
    done: impl Fn(&Vault, &[&mut Device]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done(vault, devices) {
        assert!(Instant::now() < deadline, "timed out stepping the vault");
        vault.step().unwrap();
        for device in devices.iter_mut() {
            device.step();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "binds real sockets"]
fn devices_sync_through_a_vault_that_survives_a_restart() {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gantz-vault-{}-{nanos}", std::process::id()));
    let port = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut vault = Vault::open(&dir, local(), Some(port)).unwrap();
    let (mut a, mut b) = (Device::new(), Device::new());
    step_until(&mut vault, &mut [], |v, _| v.ticket.is_some());
    let ticket = vault.ticket.clone().unwrap();
    assert!(Dir::open(&dir).is_err(), "a running vault holds its lock");

    // A graph made before pairing reaches the vault, then the other device.
    let jam = a.edit("jam", "a1");
    a.link(&ticket);
    b.link(&ticket);
    step_until(&mut vault, &mut [&mut a, &mut b], |v, d| {
        v.head("jam") == Some(jam) && d[1].head("jam") == Some(jam)
    });

    // A delete reaches both.
    a.registry.remove_head(&"jam".parse().unwrap());
    step_until(&mut vault, &mut [&mut a, &mut b], |v, d| {
        v.head("jam").is_none() && d[1].head("jam").is_none()
    });

    // The vault keeps its graphs and devices across a restart, and the
    // devices link again by themselves.
    let riff = a.edit("riff", "r1");
    step_until(&mut vault, &mut [&mut a, &mut b], |v, d| {
        v.head("riff") == Some(riff) && d[1].head("riff") == Some(riff)
    });
    vault.stop();
    let mut vault = Vault::open(&dir, local(), Some(port)).unwrap();
    assert_eq!(vault.head("riff"), Some(riff));
    assert_eq!(vault.config.devices.len(), 2);
    let riff = a.edit("riff", "r2");
    step_until(&mut vault, &mut [&mut a, &mut b], |_, d| {
        d[1].head("riff") == Some(riff)
    });
    vault.stop();
    std::fs::remove_dir_all(&dir).unwrap();
}
