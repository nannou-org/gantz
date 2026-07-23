//! Simulated-peer convergence tests for `gantz_ca::sync`.
//!
//! Each test drives a small fleet of in-process peers exchanging tip
//! announcements over a network with a controllable (seeded, adversarial)
//! delivery order. No real networking: "fetching" reads the sender's
//! registry through the same `Staged` validate-then-apply path a live
//! protocol would use.
//!
//! The property under test: after the network drains, every peer holds the
//! *identical* tip commit address (not merely an equivalent graph), with no
//! further announcements pending - the definition of convergence for #286.

use gantz_ca::{
    BothModified, CommitAddr, DataGraph, Datum, EditOrDelete, Head, NodeData, Registry,
    Resolutions, SyncStep, commit_addr, graph_addr, history, merge::MergeResolution, merge_commits,
    monotonic_timestamp, plan_sync_step, sync::Staged,
};
use petgraph::visit::EdgeRef;
use std::time::Duration;

type Graph = DataGraph;
type Reg = Registry;

/// A canonical test node with the given tag.
fn node(tag: &str) -> NodeData {
    NodeData::new(tag, Datum::Map(vec![]))
}

/// A peer's view of the shared session: its registry, its tip on the shared
/// graph, and the session bookkeeping a live peer would hold.
struct Peer {
    reg: Reg,
    tip: CommitAddr,
    /// The newest commit timestamp observed (minted or received): feeds
    /// `monotonic_timestamp` when minting.
    newest_seen: Duration,
    /// Echo suppression: the tip most recently announced.
    last_announced: Option<CommitAddr>,
    /// Merge commits this peer minted itself.
    minted_merges: usize,
    /// Conflicts flagged by the most recent merge this peer performed.
    last_conflicts: usize,
    /// Announcements that planned as `Unrelated` (surfaced, never automatic).
    unrelated: usize,
}

/// A tip announcement in flight from one peer to another.
#[derive(Clone, Copy)]
struct Msg {
    from: usize,
    to: usize,
    tip: CommitAddr,
}

/// The in-flight message set. Delivery order is chosen by the test's `Rng`,
/// so any interleaving (including per-link reordering) is reachable.
#[derive(Default)]
struct Net {
    queue: Vec<Msg>,
}

/// A tiny deterministic LCG: adversarial delivery orders from a seed without
/// pulling in a rand dependency.
struct Rng(u64);

/// The fixed session policy: last edit wins, edits beat deletes.
const RESOLUTIONS: Resolutions = Resolutions {
    both_modified: BothModified::KeepNewest,
    delete_modify: EditOrDelete::KeepEdit,
};

impl Peer {
    fn graph(&self) -> &Graph {
        self.reg
            .commit_graph_ref(&self.tip)
            .expect("peer tip must resolve to a graph")
    }

    /// Make a local edit at wall-clock `now`, committing to the tip and
    /// returning the new tip for announcement.
    fn edit(&mut self, now: Duration, f: impl FnOnce(&mut Graph)) -> CommitAddr {
        let mut graph = self.graph().clone();
        f(&mut graph);
        let ts = monotonic_timestamp(now, self.newest_seen);
        let graph_ca = graph_addr(&graph);
        let mut head = Head::Commit(self.tip);
        let ca = self
            .reg
            .commit_graph_to_head(ts, graph_ca, || graph, &mut head);
        self.newest_seen = self.newest_seen.max(ts);
        self.tip = ca;
        ca
    }

    /// Session undo: mint a forward revert commit (parent = tip, graph =
    /// `target`'s) and return the new tip for announcement - the sim
    /// analogue of `gantz_egui`'s `revert_commit` op.
    fn revert(&mut self, now: Duration, target: CommitAddr) -> CommitAddr {
        let ts = monotonic_timestamp(now, self.newest_seen);
        let graph_ca = self.reg.commits()[&target].graph;
        let ca = self.reg.commit_graph(ts, Some(self.tip), graph_ca, || {
            unreachable!("revert target's graph exists")
        });
        self.newest_seen = self.newest_seen.max(ts);
        self.tip = ca;
        ca
    }

    /// Receive an announced tip: fetch its closure from the sender via the
    /// strict `Staged` path, then apply the planned sync step. Returns the
    /// new tip when this peer *minted* a commit (which must be announced);
    /// fast-forwards and adoptions of received tips are never re-announced.
    fn receive(&mut self, sender: &Reg, tip: CommitAddr) -> Option<CommitAddr> {
        let mut staged = Staged::new();
        loop {
            let missing = staged.missing(&self.reg, tip);
            if missing.is_empty() {
                break;
            }
            for ca in missing.commits {
                let commit = sender
                    .commits()
                    .get(&ca)
                    .expect("sender must hold the announced closure")
                    .clone();
                staged
                    .insert_commit(ca, commit)
                    .expect("honest sender content must verify");
            }
            for ga in missing.graphs {
                let graph = sender
                    .graphs()
                    .get(&ga)
                    .expect("sender must hold the announced closure")
                    .clone();
                staged
                    .insert_graph(ga, graph)
                    .expect("honest sender content must verify");
            }
        }
        let applied = staged.apply(&mut self.reg).expect("closure is complete");
        for ca in &applied.commits {
            self.newest_seen = self.newest_seen.max(self.reg.commits()[ca].timestamp);
        }
        match plan_sync_step(self.reg.commits(), self.tip, tip) {
            SyncStep::UpToDate => None,
            SyncStep::FastForward(t) | SyncStep::Adopt(t) => {
                self.tip = t;
                None
            }
            SyncStep::Merge { first, second } => {
                let resolution = merge_commits(&self.reg, first, second, RESOLUTIONS)
                    .expect("planned merge tips must be related");
                let MergeResolution::Diverged { outcome, .. } = resolution else {
                    panic!("planned merge must be diverged");
                };
                self.last_conflicts = outcome.conflicts.len();
                let graph_ca = graph_addr(&outcome.graph);
                let mut head = Head::Commit(self.tip);
                let graph = outcome.graph;
                let m =
                    self.reg
                        .commit_merge_canonical(first, second, graph_ca, || graph, &mut head);
                self.newest_seen = self.newest_seen.max(self.reg.commits()[&m].timestamp);
                self.tip = m;
                self.minted_merges += 1;
                Some(m)
            }
            SyncStep::Unrelated => {
                self.unrelated += 1;
                None
            }
        }
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

/// Peers sharing a common root commit whose graph holds the given nodes.
fn peers_with_base(n: usize, base_nodes: &[&str]) -> (Vec<Peer>, Net) {
    let mut graph = Graph::default();
    for node_tag in base_nodes {
        graph.add_node(node(node_tag));
    }
    let mut reg = Reg::default();
    let graph_ca = graph_addr(&graph);
    let root = reg.commit_graph(Duration::from_secs(1), None, graph_ca, || graph);
    let peers = (0..n)
        .map(|_| Peer {
            reg: reg.clone(),
            tip: root,
            newest_seen: Duration::from_secs(1),
            last_announced: None,
            minted_merges: 0,
            last_conflicts: 0,
            unrelated: 0,
        })
        .collect();
    (peers, Net::default())
}

/// Broadcast `from`'s tip to every other peer, suppressing re-announcement
/// of an unchanged tip.
fn announce(peers: &mut [Peer], net: &mut Net, from: usize) {
    let tip = peers[from].tip;
    if peers[from].last_announced == Some(tip) {
        return;
    }
    peers[from].last_announced = Some(tip);
    for to in 0..peers.len() {
        if to != from {
            net.queue.push(Msg { from, to, tip });
        }
    }
}

/// Deliver messages in the order chosen by `seed` until the network drains,
/// announcing every minted commit. Returns the number of deliveries.
fn run(peers: &mut [Peer], net: &mut Net, seed: u64) -> usize {
    let mut rng = Rng(seed);
    let mut steps = 0;
    while !net.queue.is_empty() {
        steps += 1;
        assert!(steps < 10_000, "sync failed to quiesce");
        let i = rng.pick(net.queue.len());
        let Msg { from, to, tip } = net.queue.swap_remove(i);
        let sender_reg = peers[from].reg.clone();
        if peers[to].receive(&sender_reg, tip).is_some() {
            announce(peers, net, to);
        }
    }
    steps
}

/// Assert all peers hold the identical tip, graph address, and graph value
/// (node order included), returning the converged tip.
fn assert_converged(peers: &[Peer]) -> CommitAddr {
    let tip = peers[0].tip;
    for (i, peer) in peers.iter().enumerate() {
        assert_eq!(peer.tip, tip, "peer {i} tip diverged");
        assert_eq!(peer.unrelated, 0, "peer {i} saw unrelated announcements");
    }
    let graph_ca = peers[0].reg.commits()[&tip].graph;
    let value = graph_value(peers[0].graph());
    for (i, peer) in peers.iter().enumerate().skip(1) {
        assert_eq!(
            peer.reg.commits()[&tip].graph,
            graph_ca,
            "peer {i} graph addr"
        );
        assert_eq!(graph_value(peer.graph()), value, "peer {i} graph value");
    }
    tip
}

/// A graph's node weights in index order plus its sorted edge triples:
/// equality here is stronger than address equality (it pins node indices,
/// which cross-peer layout coherence relies on).
fn graph_value(g: &Graph) -> (Vec<NodeData>, Vec<(usize, usize, gantz_ca::Edge)>) {
    let nodes = g.node_weights().cloned().collect();
    let mut edges: Vec<_> = g
        .edge_references()
        .map(|e| (e.source().index(), e.target().index(), *e.weight()))
        .collect();
    edges.sort();
    (nodes, edges)
}

/// The node weights of a peer's converged graph.
fn node_set(peer: &Peer) -> Vec<String> {
    let mut nodes: Vec<String> = peer.graph().node_weights().map(|n| n.tag.clone()).collect();
    nodes.sort();
    nodes
}

/// Distinct merge commits reachable from `tip`.
fn reachable_merge_commits(reg: &Reg, tip: CommitAddr) -> usize {
    history::ancestors(reg.commits(), tip)
        .filter(|ca| !reg.commits()[ca].merge_parents.is_empty())
        .count()
}

#[test]
fn two_peers_disjoint_adds_converge_with_one_merge() {
    let (mut peers, mut net) = peers_with_base(2, &["base"]);
    peers[0].edit(Duration::from_secs(10), |g| {
        g.add_node(node("a"));
    });
    announce(&mut peers, &mut net, 0);
    peers[1].edit(Duration::from_secs(20), |g| {
        g.add_node(node("b"));
    });
    announce(&mut peers, &mut net, 1);
    run(&mut peers, &mut net, 42);
    let tip = assert_converged(&peers);
    assert_eq!(node_set(&peers[0]), ["a", "b", "base"]);
    // Both peers minted the merge independently, yet exactly one distinct
    // merge commit exists: they minted the identical commit.
    assert_eq!(peers[0].minted_merges + peers[1].minted_merges, 2);
    assert_eq!(reachable_merge_commits(&peers[0].reg, tip), 1);
}

#[test]
fn convergence_is_delivery_order_independent() {
    // The same two-peer scenario must converge on the same tip regardless of
    // delivery order (here trivially, but the harness honours the seed).
    let converged_tip = |seed: u64| {
        let (mut peers, mut net) = peers_with_base(2, &["base"]);
        peers[0].edit(Duration::from_secs(10), |g| {
            g.add_node(node("a"));
        });
        announce(&mut peers, &mut net, 0);
        peers[1].edit(Duration::from_secs(20), |g| {
            g.add_node(node("b"));
        });
        announce(&mut peers, &mut net, 1);
        run(&mut peers, &mut net, seed);
        assert_converged(&peers)
    };
    let tips: Vec<_> = (0..8).map(converged_tip).collect();
    assert!(tips.windows(2).all(|w| w[0] == w[1]));
}

#[test]
fn both_modified_resolves_to_newest_edit() {
    let (mut peers, mut net) = peers_with_base(2, &["n"]);
    // Both peers modify the same base node; peer 1's edit is newer.
    peers[0].edit(Duration::from_secs(10), |g| {
        g[petgraph::graph::NodeIndex::new(0)] = node("n-old");
    });
    announce(&mut peers, &mut net, 0);
    peers[1].edit(Duration::from_secs(20), |g| {
        g[petgraph::graph::NodeIndex::new(0)] = node("n-new");
    });
    announce(&mut peers, &mut net, 1);
    run(&mut peers, &mut net, 7);
    assert_converged(&peers);
    // Last edit wins. At least one peer performed the merge and saw the
    // conflict; a peer may instead fast-forward onto the other's (identical)
    // merge commit without merging itself.
    assert_eq!(node_set(&peers[0]), ["n-new"]);
    assert!(peers[0].last_conflicts + peers[1].last_conflicts >= 1);
}

#[test]
fn delete_vs_modify_keeps_the_edit() {
    let (mut peers, mut net) = peers_with_base(2, &["base", "n"]);
    peers[0].edit(Duration::from_secs(10), |g| {
        g.remove_node(petgraph::graph::NodeIndex::new(1));
    });
    announce(&mut peers, &mut net, 0);
    peers[1].edit(Duration::from_secs(20), |g| {
        g[petgraph::graph::NodeIndex::new(1)] = node("n-edited");
    });
    announce(&mut peers, &mut net, 1);
    run(&mut peers, &mut net, 7);
    assert_converged(&peers);
    // KeepEdit: the modified node survives the delete, flagged as a conflict
    // on whichever peer(s) performed the merge.
    assert_eq!(node_set(&peers[0]), ["base", "n-edited"]);
    assert!(peers[0].last_conflicts + peers[1].last_conflicts >= 1);
}

#[test]
fn three_peers_converge_under_arbitrary_delivery_orders() {
    for seed in 0..40 {
        let (mut peers, mut net) = peers_with_base(3, &["base"]);
        for (i, (name, secs)) in [("a", 10), ("b", 20), ("c", 30)].iter().enumerate() {
            peers[i].edit(Duration::from_secs(*secs), |g| {
                g.add_node(node(name));
            });
            announce(&mut peers, &mut net, i);
        }
        run(&mut peers, &mut net, seed);
        assert_converged(&peers);
        assert_eq!(
            node_set(&peers[0]),
            ["a", "b", "base", "c"],
            "seed {seed}: all edits present"
        );
    }
}

#[test]
fn twin_commits_adopt_without_merging() {
    let (mut peers, mut net) = peers_with_base(2, &["base"]);
    // Both peers make the identical edit concurrently (e.g. two resync
    // passes): same graph, different timestamps.
    peers[0].edit(Duration::from_secs(10), |g| {
        g.add_node(node("x"));
    });
    announce(&mut peers, &mut net, 0);
    peers[1].edit(Duration::from_secs(20), |g| {
        g.add_node(node("x"));
    });
    announce(&mut peers, &mut net, 1);
    run(&mut peers, &mut net, 3);
    let tip = assert_converged(&peers);
    // Adoption, not merging: zero merge commits anywhere.
    assert_eq!(reachable_merge_commits(&peers[0].reg, tip), 0);
    assert_eq!(peers[0].minted_merges + peers[1].minted_merges, 0);
}

#[test]
fn fast_forwards_are_not_reannounced() {
    let (mut peers, mut net) = peers_with_base(2, &["base"]);
    peers[0].edit(Duration::from_secs(10), |g| {
        g.add_node(node("a"));
    });
    announce(&mut peers, &mut net, 0);
    // A single delivery: peer 1 fast-forwards and must stay silent.
    let steps = run(&mut peers, &mut net, 0);
    assert_eq!(steps, 1);
    assert_converged(&peers);
    assert_eq!(peers[1].minted_merges, 0);
}

#[test]
fn redelivery_is_idempotent() {
    let (mut peers, mut net) = peers_with_base(2, &["base"]);
    peers[0].edit(Duration::from_secs(10), |g| {
        g.add_node(node("a"));
    });
    announce(&mut peers, &mut net, 0);
    peers[1].edit(Duration::from_secs(20), |g| {
        g.add_node(node("b"));
    });
    announce(&mut peers, &mut net, 1);
    run(&mut peers, &mut net, 42);
    let tip = assert_converged(&peers);
    let commit_counts: Vec<_> = peers.iter().map(|p| p.reg.commits().len()).collect();
    // Re-deliver every peer's tip to everyone, bypassing suppression (a lost
    // ack / anti-entropy repeat).
    for from in 0..peers.len() {
        let t = peers[from].tip;
        for to in 0..peers.len() {
            if to != from {
                net.queue.push(Msg { from, to, tip: t });
            }
        }
    }
    run(&mut peers, &mut net, 43);
    assert_eq!(assert_converged(&peers), tip);
    let after: Vec<_> = peers.iter().map(|p| p.reg.commits().len()).collect();
    assert_eq!(commit_counts, after, "redelivery minted nothing");
}

#[test]
fn slow_clock_edits_stay_causally_ordered() {
    let (mut peers, mut net) = peers_with_base(2, &["n"]);
    // Peer 0 edits at t=100; peer 1's wall clock is far behind (t=51).
    peers[0].edit(Duration::from_secs(100), |g| {
        g[petgraph::graph::NodeIndex::new(0)] = node("n-first");
    });
    announce(&mut peers, &mut net, 0);
    run(&mut peers, &mut net, 0);
    assert_converged(&peers);
    // Peer 1 edits *after observing* the t=100 commit, with its slow clock.
    let tip = peers[1].edit(Duration::from_secs(51), |g| {
        g[petgraph::graph::NodeIndex::new(0)] = node("n-after");
    });
    // The monotonic guard orders the edit strictly after everything observed,
    // so "last edit wins" respects causality despite the skew.
    let ts = peers[1].reg.commits()[&tip].timestamp;
    assert!(ts > Duration::from_secs(100));
    announce(&mut peers, &mut net, 1);
    run(&mut peers, &mut net, 0);
    assert_converged(&peers);
    assert_eq!(node_set(&peers[0]), ["n-after"]);
}

#[test]
fn join_snapshot_tolerates_pruned_history() {
    // A host with pruned history: the surviving tip was detached in place, so
    // its content no longer hashes to its key.
    let (mut host_peers, _) = peers_with_base(1, &["base"]);
    let mut host = host_peers.remove(0);
    host.edit(Duration::from_secs(10), |g| {
        g.add_node(node("a"));
    });
    host.edit(Duration::from_secs(20), |g| {
        g.add_node(node("b"));
    });
    let tip = host.tip;
    let live = gantz_ca::LiveSet {
        commits: [tip].into_iter().collect(),
        graphs: [host.reg.commits()[&tip].graph].into_iter().collect(),
        blobs: Default::default(),
    };
    gantz_ca::prune(&mut host.reg, &live);
    assert_ne!(
        commit_addr(&host.reg.commits()[&tip]),
        tip,
        "tip is detached"
    );

    // The joiner applies the host's snapshot through the grandfathered path.
    let mut reg = Reg::default();
    let mut staged = Staged::new();
    for ca in history::ancestors(host.reg.commits(), tip) {
        staged.insert_commit_grandfathered(ca, host.reg.commits()[&ca].clone());
    }
    for (&ga, g) in host.reg.graphs() {
        staged.insert_graph(ga, g.clone()).unwrap();
    }
    assert_eq!(staged.grandfathered(), &[tip]);
    let applied = staged.apply(&mut reg).unwrap();
    assert_eq!(applied.truncated, 0, "the host already detached the parent");
    let joiner = Peer {
        reg,
        tip,
        newest_seen: Duration::from_secs(20),
        last_announced: None,
        minted_merges: 0,
        last_conflicts: 0,
        unrelated: 0,
    };

    // Live sync continues over the pruned base: concurrent edits still merge.
    let mut peers = vec![host, joiner];
    let mut net = Net::default();
    peers[0].edit(Duration::from_secs(30), |g| {
        g.add_node(node("host-edit"));
    });
    announce(&mut peers, &mut net, 0);
    peers[1].edit(Duration::from_secs(40), |g| {
        g.add_node(node("joiner-edit"));
    });
    announce(&mut peers, &mut net, 1);
    run(&mut peers, &mut net, 5);
    assert_converged(&peers);
    assert_eq!(
        node_set(&peers[0]),
        ["a", "b", "base", "host-edit", "joiner-edit"]
    );
}

#[test]
fn unrelated_announcements_are_surfaced_not_applied() {
    // Two peers with no shared history: the announcement is recorded as
    // unrelated and the local tip is untouched (the app decides what to do).
    let (mut a_peers, _) = peers_with_base(1, &["a"]);
    let (mut b_peers, _) = peers_with_base(1, &["b"]);
    let mut a = a_peers.remove(0);
    let b = b_peers.remove(0);
    let a_tip = a.tip;
    let minted = a.receive(&b.reg, b.tip);
    assert_eq!(minted, None);
    assert_eq!(a.tip, a_tip);
    assert_eq!(a.unrelated, 1);
}

#[test]
fn revert_commits_converge() {
    // A commits two edits and everyone converges; A then session-undoes the
    // second by minting a forward revert commit. Peers converge on the
    // revert WITHOUT any subsequent edit (backward navigation would present
    // an ancestor tip and be dropped as up-to-date), and its graph equals
    // the pre-edit state.
    let (mut peers, mut net) = peers_with_base(2, &["base"]);
    let e1 = peers[0].edit(Duration::from_secs(10), |g| {
        g.add_node(node("a"));
    });
    announce(&mut peers, &mut net, 0);
    peers[0].edit(Duration::from_secs(20), |g| {
        g.add_node(node("b"));
    });
    announce(&mut peers, &mut net, 0);
    run(&mut peers, &mut net, 7);
    assert_converged(&peers);

    peers[0].revert(Duration::from_secs(30), e1);
    announce(&mut peers, &mut net, 0);
    run(&mut peers, &mut net, 7);
    let tip = assert_converged(&peers);
    assert_eq!(node_set(&peers[0]), ["a", "base"]);
    // The revert is an ordinary forward commit: peers fast-forward, no merge.
    assert_eq!(peers[0].minted_merges + peers[1].minted_merges, 0);
    assert_eq!(
        peers[0].reg.commits()[&tip].graph,
        peers[0].reg.commits()[&e1].graph,
    );
}

#[test]
fn concurrent_revert_and_edit_converge() {
    // A session-undoes an edit while B concurrently adds a node: an ordinary
    // diverged pair. Every delivery order converges on one identical merge
    // commit; the revert acts as a removal of the undone addition, and B's
    // concurrent addition survives.
    for seed in [1, 42, 1234, 987654321] {
        let (mut peers, mut net) = peers_with_base(2, &["base"]);
        let e1 = peers[0].edit(Duration::from_secs(10), |g| {
            g.add_node(node("a"));
        });
        announce(&mut peers, &mut net, 0);
        peers[0].edit(Duration::from_secs(20), |g| {
            g.add_node(node("b"));
        });
        announce(&mut peers, &mut net, 0);
        run(&mut peers, &mut net, seed);
        assert_converged(&peers);

        peers[0].revert(Duration::from_secs(30), e1);
        announce(&mut peers, &mut net, 0);
        peers[1].edit(Duration::from_secs(31), |g| {
            g.add_node(node("c"));
        });
        announce(&mut peers, &mut net, 1);
        run(&mut peers, &mut net, seed);
        let tip = assert_converged(&peers);
        assert_eq!(node_set(&peers[0]), ["a", "base", "c"], "seed {seed}");
        assert_eq!(
            reachable_merge_commits(&peers[0].reg, tip),
            1,
            "seed {seed}"
        );
    }
}
