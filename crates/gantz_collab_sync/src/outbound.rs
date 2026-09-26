//! The serve and announce plane. Local tips are mirrored into the
//! runtime-owned served store and broadcast to peers.

use crate::{SessionState, Sessions};
use gantz_ca as ca;
use gantz_collab::{Command, GossipMsg, Handle, PeerId, SessionId};
use std::collections::BTreeSet;

/// The maximum tip entries per gossip `Tips` message, keeping each message
/// well under iroh-gossip's size limit.
const TIPS_PER_MSG: usize = 16;

/// Mirror each session's scoped closure into its served store and broadcast
/// the changed tips, when the sessions are dirty. Clears the flag. Returns
/// the announced tips.
pub fn announce(
    sessions: &mut Sessions,
    registry: &ca::Registry,
    handle: &Handle,
    origin: PeerId,
) -> Vec<(SessionId, ca::Name, ca::CommitAddr)> {
    let mut announced = Vec::new();
    if !sessions.dirty {
        return announced;
    }
    sessions.dirty = false;
    for (id, state) in sessions.sessions.iter_mut() {
        let scope = gantz_egui::sync::session_scope(registry, &state.branch_name());
        let mut changed = Vec::new();
        serve_scope(handle, state, registry, &scope);
        for name in &scope {
            let Some(tip) = registry.head(name) else {
                continue;
            };
            if state.last_announced.get(name) == Some(&tip) {
                continue;
            }
            let Some(commit) = registry.commits().get(&tip) else {
                continue;
            };
            state.last_announced.insert(name.clone(), tip);
            changed.push((name.clone(), tip, commit.graph));
        }
        for chunk in changed.chunks(TIPS_PER_MSG) {
            state.seq += 1;
            let msg = GossipMsg::Tips {
                origin,
                seq: state.seq,
                changed: chunk.to_vec(),
            };
            let _ = handle
                .cmds
                .try_send(Command::Broadcast { session: *id, msg });
        }
        announced.extend(changed.into_iter().map(|(n, tip, _)| (*id, n, tip)));
    }
    announced
}

/// Mirror the scoped closure of `scope` into the session's served store via
/// [`Command::Update`]. One reachability walk from the scoped tips via
/// [`ca::closure_from`] surfaces every required commit, graph and
/// content-referenced blob, nested references included. In-scope metadata
/// section entries ride along, so peers place synced nodes where their
/// author put them.
///
/// The runtime owns the store, so `state`'s served-content shadows keep the
/// update incremental without reading it back. Inserts are content-addressed
/// and idempotent, so shadow loss only costs a re-send.
pub fn serve_scope(
    handle: &Handle,
    state: &mut SessionState,
    registry: &ca::Registry,
    scope: &BTreeSet<ca::Name>,
) {
    let tips = scope.iter().filter_map(|n| registry.head(n));
    let live = ca::closure_from(registry, tips);
    let mut commits = Vec::new();
    let mut graphs = Vec::new();
    let mut blobs = Vec::new();
    for &addr in &live.commits {
        if state.served_commits.contains(&addr) {
            continue;
        }
        let Some(commit) = registry.commits().get(&addr) else {
            continue;
        };
        state.served_commits.insert(addr);
        commits.push((addr, commit.clone()));
    }
    for &ga in &live.graphs {
        if state.served_graphs.contains(&ga) {
            continue;
        }
        let Some(graph) = registry.graph(&ga) else {
            continue;
        };
        state.served_graphs.insert(ga);
        graphs.push((ga, graph.clone()));
    }
    for (section, addrs) in &live.blobs {
        let Some(store) = registry.blobs().get(section) else {
            continue;
        };
        for &addr in addrs {
            let key = (section.clone(), addr);
            if state.served_blobs.contains(&key) {
                continue;
            }
            let Some(bytes) = store.get(&addr) else {
                continue;
            };
            state.served_blobs.insert(key);
            blobs.push((section.clone(), store.liveness, addr, bytes.clone()));
        }
    }
    // In-scope metadata is the section entries keyed by scoped names or
    // content in the closure. The `heads` section travels as the head list.
    // Address-keyed entries have no scoping rule and stay local.
    let mut sections = Vec::new();
    for (id, section) in registry.sections() {
        if id.as_str() == ca::HEADS_ID {
            continue;
        }
        for (key, value) in &section.entries {
            let in_scope = match key {
                ca::Key::Commit(ca) => live.commits.contains(ca),
                ca::Key::Name(name) => scope.contains(name),
                ca::Key::Graph(ga) => live.graphs.contains(ga),
                ca::Key::Addr(_) => false,
            };
            if !in_scope {
                continue;
            }
            let entry = (id.clone(), key.clone());
            if state.served_sections.contains(&entry) {
                continue;
            }
            state.served_sections.insert(entry);
            sections.push((
                id.clone(),
                section.policy,
                section.liveness,
                key.clone(),
                value.clone(),
            ));
        }
    }
    let mut heads = Vec::new();
    for name in scope {
        let Some(tip) = registry.head(name) else {
            continue;
        };
        if state.served_heads.insert(name.clone(), tip) != Some(tip) {
            heads.push((name.clone(), tip));
        }
    }
    if heads.is_empty()
        && commits.is_empty()
        && graphs.is_empty()
        && blobs.is_empty()
        && sections.is_empty()
    {
        return;
    }
    let _ = handle.cmds.try_send(Command::Update {
        session: state.session.id,
        heads,
        commits,
        graphs,
        sections,
        blobs,
    });
}
