use crate::vault::{CONFIG_KEY, Dir, TicketServer, Vault, remove_old_ticket, request_ticket};
use bevy_pkv::PkvStore;
use gantz_ca as ca;
use gantz_collab::identity;
use gantz_collab::{Handle, Identity, Infra, RuntimeConfig};
use gantz_collab_sync::{OpenHeads, Sessions};
use gantz_store::{Load, STORE_FORMAT};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
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
            app: "gantz device".to_string(),
        };
        Self {
            handle: gantz_collab::spawn(Identity::generate(), config),
            sessions: Sessions::default(),
            registry: ca::Registry::default(),
        }
    }

    fn link(&mut self, ticket: &str) {
        let (synced, local_only) = (Default::default(), BTreeSet::new());
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

/// A new directory under the system temp dir.
fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("gantz-vault-{tag}-{}-{nanos}", std::process::id());
    std::env::temp_dir().join(name)
}

/// Write raw entries into the store in `dir`, as another build might.
fn write_store(dir: &Path, entries: &[(&str, &str)]) {
    std::fs::create_dir_all(dir).unwrap();
    let mut store = PkvStore::new_in_dir(dir);
    for (key, value) in entries {
        store.set_string(key, value).unwrap();
    }
}

fn read_store(dir: &Path, key: &str) -> Option<String> {
    PkvStore::new_in_dir(dir).get_string(key).unwrap()
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
    let dir = temp_dir("sync");
    let port = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut vault = Vault::open(&dir, "gantz vault", local(), Some(port)).unwrap();
    let (mut a, mut b) = (Device::new(), Device::new());
    step_until(&mut vault, &mut [], |_, _| request_ticket(&dir).is_ok());
    let ticket = request_ticket(&dir).unwrap();
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
    // A description reaches the other device alone, without a graph edit.
    let riff_name: ca::Name = "riff".parse().unwrap();
    let described = |reg: &ca::Registry| gantz_egui::section::description(reg, &riff_name);
    let text = "a riff".to_string();
    gantz_egui::section::set_description(&mut a.registry, riff_name.clone(), text);
    step_until(&mut vault, &mut [&mut a, &mut b], |v, d| {
        described(&v.registry).is_some() && described(&d[1].registry).is_some()
    });
    vault.stop();
    let mut vault = Vault::open(&dir, "gantz vault", local(), Some(port)).unwrap();
    assert_eq!(vault.head("riff"), Some(riff));
    assert_eq!(described(&vault.registry).as_deref(), Some("a riff"));
    assert_eq!(vault.config.devices.len(), 2);
    assert!(
        vault
            .config
            .devices
            .values()
            .all(|d| d.app == "gantz device")
    );
    let riff = a.edit("riff", "r2");
    step_until(&mut vault, &mut [&mut a, &mut b], |_, d| {
        d[1].head("riff") == Some(riff)
    });
    vault.stop();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_vault_written_by_a_newer_gantz_is_refused_untouched() {
    let dir = temp_dir("newer");
    let meta = format!("(format:{},written_by:\"gantz 9.0.0\")", STORE_FORMAT + 1);
    write_store(&dir, &[("store-meta", &meta)]);
    let Err(error) = Vault::open(&dir, "gantz vault", local(), None) else {
        panic!("a vault from a newer gantz opened");
    };
    assert!(error.contains("written by gantz 9.0.0"), "{error}");
    assert!(error.contains("Update gantz"), "{error}");
    assert_eq!(read_store(&dir, "store-meta"), Some(meta));
    assert_eq!(read_store(&dir, identity::KEY), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_unreadable_config_is_refused_untouched() {
    let dir = temp_dir("config");
    write_store(&dir, &[(CONFIG_KEY, "(id:")]);
    let Err(error) = Vault::open(&dir, "gantz vault", local(), None) else {
        panic!("a vault with an unreadable config opened");
    };
    assert!(error.contains("left untouched"), "{error}");
    assert_eq!(read_store(&dir, CONFIG_KEY).as_deref(), Some("(id:"));
    assert_eq!(read_store(&dir, identity::KEY), None);
    assert_eq!(read_store(&dir, "store-meta"), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

// Only the owner can connect, nothing is handed out before the first ticket,
// and a request gets the latest one. A stale socket is replaced, and a
// stopped server leaves no socket behind.
#[cfg(unix)]
#[test]
fn the_ticket_socket_hands_its_owner_the_latest_ticket() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("ticket");
    let socket = dir.join("ticket.sock");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&socket, "stale").unwrap();
    let tickets = TicketServer::start(&dir).unwrap();
    let mode = std::fs::metadata(&socket).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let error = request_ticket(&dir).unwrap_err();
    assert!(error.contains("no ticket yet"), "{error}");
    tickets.set("first".to_string());
    tickets.set("second".to_string());
    assert_eq!(request_ticket(&dir).unwrap(), "second");
    drop(tickets);
    assert!(!socket.exists());
    let error = request_ticket(&dir).unwrap_err();
    assert!(error.contains("not running"), "{error}");
    std::fs::remove_dir_all(&dir).unwrap();
}

// A ticket file that gantz 0.5 wrote is deleted, so no ticket stays on disk.
#[test]
fn an_old_ticket_file_is_deleted() {
    let dir = temp_dir("old-ticket");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ticket"), "gantzvault").unwrap();
    remove_old_ticket(&dir).unwrap();
    assert!(!dir.join("ticket").exists());
    remove_old_ticket(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

// `devices` and `revoke` open the directory, so a mistyped one is reported
// and left uncreated.
#[test]
fn a_missing_vault_is_refused_and_not_created() {
    let dir = temp_dir("missing");
    let Err(error) = Dir::open(&dir) else {
        panic!("a missing vault opened");
    };
    assert!(error.contains("no vault"), "{error}");
    assert!(!dir.exists());
}
