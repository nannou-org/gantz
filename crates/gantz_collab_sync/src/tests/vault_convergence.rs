//! Simulated vault sync tests for `crate::vault::step::plan`.
//!
//! Devices sync named graphs through a compare-and-swap vault model under
//! seeded, adversarial schedules of edits, creates, deletes, offline spans,
//! stale reads and push races. There is no real networking. Every transfer
//! goes through `closure_diff` and the strict `Staged` path, as the live
//! protocol does.
//!
//! The properties under test:
//!
//! - Convergence. Once every device syncs to quiescence, each holds exactly
//!   the vault's heads.
//! - No lost edits. The vault only moves a name to a descendant of its head
//!   or to an unrelated recreate, or removes it on an explicit delete. A
//!   device only drops an unsynced local head for one that contains it.

use crate::vault::step::{self, Step};
use gantz_ca::{
    BothModified, CommitAddr, Datum, EditOrDelete, Head, MergeAnalysis, MergeResolution, Name,
    NodeData, Registry, Resolutions, closure_diff, graph_addr, history, merge_commits,
    monotonic_timestamp, sync::Staged,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// The fixed merge policy. Last edit wins and edits beat deletes.
const RESOLUTIONS: Resolutions = Resolutions {
    both_modified: BothModified::KeepNewest,
    delete_modify: EditOrDelete::KeepEdit,
};

/// A small name pool, so devices collide on names often.
const NAMES: [&str; 3] = ["a", "b", "c"];

type Heads = BTreeMap<Name, CommitAddr>;

/// The referee. It accepts a push only against its current head.
#[derive(Default)]
struct Vault {
    reg: Registry,
}

/// One device's registry and its vault bookkeeping.
#[derive(Default)]
struct Device {
    reg: Registry,
    /// The vault head each name was last agreed at.
    synced: Heads,
    /// The vault heads as last seen. Stale until the next refresh.
    remote: Heads,
    /// The newest commit timestamp observed. Feeds `monotonic_timestamp`.
    newest_seen: Duration,
}

/// Counts of the interesting paths a schedule took, so a sweep can check it
/// is not vacuous.
#[derive(Debug, Default)]
struct Stats {
    rejected: usize,
    merged: usize,
    asides: usize,
    resurrected: usize,
}

/// A tiny deterministic LCG. It gives adversarial schedules from a seed
/// without a rand dependency.
struct Rng(u64);

impl Vault {
    fn heads(&self) -> Heads {
        heads(&self.reg)
    }

    /// Accept `tip` for `name` if the vault's head is still `base`. Returns
    /// the vault's head on rejection.
    fn push(
        &mut self,
        from: &Registry,
        name: &Name,
        tip: Option<CommitAddr>,
        base: Option<CommitAddr>,
    ) -> Result<(), Option<CommitAddr>> {
        let head = self.reg.head(name);
        if head != base {
            return Err(head);
        }
        match tip {
            None => {
                self.reg.remove_head(name);
            }
            Some(tip) => {
                transfer(from, &mut self.reg, tip, base);
                if let Some(base) = base {
                    let analysis = history::analyze(self.reg.commits(), tip, base);
                    assert!(
                        matches!(
                            analysis,
                            MergeAnalysis::AlreadyUpToDate | MergeAnalysis::Unrelated
                        ),
                        "the vault would drop '{name}' at {base} for {tip}: {analysis:?}",
                    );
                }
                self.reg.set_head(name.clone(), tip);
            }
        }
        Ok(())
    }
}

impl Device {
    /// Add a node to `name`'s graph, creating the name as a new root commit
    /// when it is absent.
    fn edit(&mut self, now: Duration, name: &Name, tag: &str) {
        let parent = self.reg.head(name);
        let mut graph = parent
            .and_then(|ca| self.reg.commit_graph_ref(&ca))
            .cloned()
            .unwrap_or_default();
        graph.add_node(node(tag));
        let ts = monotonic_timestamp(now, self.newest_seen);
        let graph_ca = graph_addr(&graph);
        let ca = self.reg.commit_graph(ts, parent, graph_ca, || graph);
        self.reg.set_head(name.clone(), ca);
        self.newest_seen = self.newest_seen.max(ts);
    }

    fn delete(&mut self, name: &Name) {
        self.reg.remove_head(name);
    }

    /// Learn the vault's current heads, as a link does on `LinkUp`.
    fn refresh(&mut self, vault: &Vault) {
        self.remote = vault.heads();
    }

    /// Every name this device has an opinion on.
    fn names(&self) -> BTreeSet<Name> {
        let local = self.reg.heads().map(|(n, _)| n.clone());
        let known = self.synced.keys().chain(self.remote.keys()).cloned();
        local.chain(known).collect()
    }

    /// Bring one name into agreement with the vault as far as one step
    /// allows. Returns whether any state changed.
    fn sync_name(&mut self, vault: &mut Vault, name: &Name, stats: &mut Stats) -> bool {
        let local = self.reg.head(name);
        let base = self.synced.get(name).copied();
        let remote = self.remote.get(name).copied();
        if let Some(r) = remote {
            if !self.reg.commits().contains_key(&r) {
                transfer(&vault.reg, &mut self.reg, r, local.into_iter().chain(base));
                let ts = self.reg.commits()[&r].timestamp;
                self.newest_seen = self.newest_seen.max(ts);
            }
        }
        match step::plan(self.reg.commits(), local, base, remote) {
            Step::InSync => set(&mut self.synced, name, local),
            Step::Keep => false,
            Step::Adopt(target) => {
                if let Some(l) = local.filter(|&l| Some(l) != base) {
                    let t = target.expect("an unsynced edit never loses to a delete");
                    assert!(
                        contains(&self.reg, t, l),
                        "'{name}' dropped the unsynced local head {l} for {t}",
                    );
                }
                match target {
                    Some(t) => {
                        self.reg.set_head(name.clone(), t);
                    }
                    None => {
                        self.reg.remove_head(name);
                    }
                }
                set(&mut self.synced, name, target);
                true
            }
            Step::Push(tip) => {
                if tip.is_some() && remote.is_none() && base.is_some() {
                    stats.resurrected += 1;
                }
                self.push(vault, name, tip, remote, stats)
            }
            Step::Merge { first, second, .. } => {
                let merged = self.merge(first, second);
                self.reg.set_head(name.clone(), merged);
                stats.merged += 1;
                self.push(vault, name, Some(merged), remote, stats)
            }
            Step::Aside { local, remote } => {
                let aside: Name = format!("{name}-{}", local.display_short()).parse().unwrap();
                self.reg.set_head(aside, local);
                self.reg.set_head(name.clone(), remote);
                set(&mut self.synced, name, Some(remote));
                stats.asides += 1;
                true
            }
        }
    }

    /// Push against the vault head this device last saw. A rejection
    /// teaches the device the vault's actual head.
    fn push(
        &mut self,
        vault: &mut Vault,
        name: &Name,
        tip: Option<CommitAddr>,
        base: Option<CommitAddr>,
        stats: &mut Stats,
    ) -> bool {
        match vault.push(&self.reg, name, tip, base) {
            Ok(()) => {
                set(&mut self.synced, name, tip);
                set(&mut self.remote, name, tip);
            }
            Err(head) => {
                set(&mut self.remote, name, head);
                stats.rejected += 1;
            }
        }
        true
    }

    /// Mint the canonical merge commit of two diverged tips.
    fn merge(&mut self, first: CommitAddr, second: CommitAddr) -> CommitAddr {
        let resolution = merge_commits(&self.reg, first, second, RESOLUTIONS)
            .expect("planned merge tips must be related");
        let MergeResolution::Diverged { outcome, .. } = resolution else {
            panic!("a planned merge must be diverged");
        };
        let graph_ca = graph_addr(&outcome.graph);
        let graph = outcome.graph;
        let mut head = Head::Commit(first);
        let merged = self
            .reg
            .commit_merge_canonical(first, second, graph_ca, || graph, &mut head);
        let ts = self.reg.commits()[&merged].timestamp;
        self.newest_seen = self.newest_seen.max(ts);
        merged
    }

    /// Sync every name once. Returns whether any state changed.
    fn sync_all(&mut self, vault: &mut Vault, stats: &mut Stats) -> bool {
        self.refresh(vault);
        let mut changed = false;
        for name in self.names() {
            changed |= self.sync_name(vault, &name, stats);
        }
        changed
    }
}

impl Rng {
    fn pick(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
}

fn name(s: &str) -> Name {
    s.parse().unwrap()
}

fn node(tag: &str) -> NodeData {
    NodeData::new(tag, Datum::Map(vec![]))
}

fn heads(reg: &Registry) -> Heads {
    reg.heads().map(|(n, ca)| (n.clone(), ca)).collect()
}

/// Insert or remove a map entry. Returns whether the map changed.
fn set(map: &mut Heads, name: &Name, value: Option<CommitAddr>) -> bool {
    let old = match value {
        Some(ca) => map.insert(name.clone(), ca),
        None => map.remove(name),
    };
    old != value
}

/// Whether `outer`'s history contains `inner`, or reaches the same graph.
fn contains(reg: &Registry, outer: CommitAddr, inner: CommitAddr) -> bool {
    let graph = |ca: CommitAddr| reg.commits()[&ca].graph;
    history::analyze(reg.commits(), outer, inner) == MergeAnalysis::AlreadyUpToDate
        || graph(outer) == graph(inner)
}

/// Copy what `src` holds for `tip` beyond `have` into `dst` through the strict
/// staging path. The receiver must hold the closure of every `have` commit
/// it passes.
fn transfer(
    src: &Registry,
    dst: &mut Registry,
    tip: CommitAddr,
    have: impl IntoIterator<Item = CommitAddr>,
) {
    let live = closure_diff(src, [tip], have);
    let mut staged = Staged::new();
    for ca in live.commits {
        staged
            .insert_commit(ca, src.commits()[&ca].clone())
            .unwrap();
    }
    for ga in live.graphs {
        staged.insert_graph(ga, src.graphs()[&ga].clone()).unwrap();
    }
    assert!(staged.is_complete(dst, tip), "closure_diff left a gap");
    staged.apply(dst).unwrap();
}

/// Sync every device until a full round changes nothing.
fn quiesce(vault: &mut Vault, devices: &mut [Device], stats: &mut Stats) {
    for round in 0.. {
        assert!(round < 50, "vault sync failed to quiesce");
        let mut changed = false;
        for device in devices.iter_mut() {
            changed |= device.sync_all(vault, stats);
        }
        if !changed {
            break;
        }
    }
}

fn assert_converged(vault: &Vault, devices: &[Device]) {
    let expected = vault.heads();
    for (i, device) in devices.iter().enumerate() {
        assert_eq!(heads(&device.reg), expected, "device {i} heads");
        assert_eq!(device.synced, expected, "device {i} synced");
    }
}

/// A random schedule over `n` devices, then quiescence.
fn run(seed: u64, n: usize, steps: usize, stats: &mut Stats) {
    let mut rng = Rng(seed);
    let mut vault = Vault::default();
    let mut devices: Vec<Device> = (0..n).map(|_| Device::default()).collect();
    let mut online = vec![true; n];
    for step in 0..steps {
        let now = Duration::from_secs(10 + step as u64);
        let d = rng.pick(n);
        let target = name(NAMES[rng.pick(NAMES.len())]);
        match rng.pick(10) {
            0..=3 => devices[d].edit(now, &target, &format!("d{d}s{step}")),
            4 => devices[d].delete(&target),
            5 => online[d] = !online[d],
            6 if online[d] => devices[d].refresh(&vault),
            7..=9 if online[d] => {
                devices[d].sync_name(&mut vault, &target, stats);
            }
            _ => (),
        }
    }
    quiesce(&mut vault, &mut devices, stats);
    assert_converged(&vault, &devices);
}

#[test]
fn devices_converge_under_random_schedules() {
    let mut stats = Stats::default();
    for seed in 0..300 {
        run(seed, 2, 60, &mut stats);
        run(seed, 3, 90, &mut stats);
    }
    // The sweep must exercise every contested path.
    assert!(stats.rejected > 0, "{stats:?}");
    assert!(stats.merged > 0, "{stats:?}");
    assert!(stats.asides > 0, "{stats:?}");
    assert!(stats.resurrected > 0, "{stats:?}");
}

#[test]
fn first_pairing_moves_unrelated_graphs_aside() {
    let mut stats = Stats::default();
    let mut vault = Vault::default();
    let mut devices = vec![Device::default(), Device::default()];
    let jam = name("jam");
    devices[0].edit(Duration::from_secs(1), &jam, "laptop");
    devices[1].edit(Duration::from_secs(2), &jam, "desktop");
    let desktop = devices[1].reg.head(&jam).unwrap();
    quiesce(&mut vault, &mut devices, &mut stats);
    assert_converged(&vault, &devices);
    assert_eq!(stats.asides, 1);
    let aside = name(&format!("jam-{}", desktop.display_short()));
    assert_eq!(vault.reg.head(&aside), Some(desktop));
    assert_eq!(vault.heads().len(), 2);
}

#[test]
fn deletes_propagate_to_devices_without_local_edits() {
    let mut stats = Stats::default();
    let mut vault = Vault::default();
    let mut devices = vec![Device::default(), Device::default()];
    let jam = name("jam");
    devices[0].edit(Duration::from_secs(1), &jam, "a");
    quiesce(&mut vault, &mut devices, &mut stats);
    devices[1].delete(&jam);
    quiesce(&mut vault, &mut devices, &mut stats);
    assert_converged(&vault, &devices);
    assert!(vault.heads().is_empty());
}

#[test]
fn a_concurrent_edit_beats_a_delete() {
    let mut stats = Stats::default();
    let mut vault = Vault::default();
    let mut devices = vec![Device::default(), Device::default()];
    let jam = name("jam");
    devices[0].edit(Duration::from_secs(1), &jam, "a");
    quiesce(&mut vault, &mut devices, &mut stats);
    devices[0].delete(&jam);
    devices[1].edit(Duration::from_secs(2), &jam, "b");
    let edited = devices[1].reg.head(&jam).unwrap();
    devices[0].sync_all(&mut vault, &mut stats);
    quiesce(&mut vault, &mut devices, &mut stats);
    assert_converged(&vault, &devices);
    assert_eq!(vault.reg.head(&jam), Some(edited));
    assert_eq!(stats.resurrected, 1);
}

#[test]
fn a_stale_push_is_rejected_then_merged() {
    let mut stats = Stats::default();
    let mut vault = Vault::default();
    let mut devices = vec![Device::default(), Device::default()];
    let jam = name("jam");
    devices[0].edit(Duration::from_secs(1), &jam, "base");
    quiesce(&mut vault, &mut devices, &mut stats);
    devices[0].edit(Duration::from_secs(2), &jam, "a");
    devices[1].edit(Duration::from_secs(3), &jam, "b");
    devices[0].sync_name(&mut vault, &jam, &mut stats);
    devices[1].sync_name(&mut vault, &jam, &mut stats);
    assert_eq!(stats.rejected, 1);
    quiesce(&mut vault, &mut devices, &mut stats);
    assert_converged(&vault, &devices);
    assert_eq!(stats.merged, 1);
    let tip = vault.reg.head(&jam).unwrap();
    let tags: BTreeSet<&str> = vault
        .reg
        .commit_graph_ref(&tip)
        .unwrap()
        .node_weights()
        .map(|n| n.tag.as_str())
        .collect();
    assert_eq!(tags, ["a", "b", "base"].into_iter().collect());
}
