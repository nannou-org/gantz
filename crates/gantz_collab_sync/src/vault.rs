//! Syncing a device with a vault, and the vault's side of a push.
//!
//! A device links to one vault. See [`gantz_collab::vault`]. For every name
//! outside `local_only`, the device tracks the vault head it last agreed
//! with, `synced`, and the vault's current head, `remote`. Each pass plans
//! one step per name that is out of step: adopt the vault's head, push the
//! local one, merge, or move an unrelated local graph aside. See
//! `vault::step`. The vault accepts a push only against its current head. A
//! rejection teaches the device the vault's head, and the name plans again
//! on the next pass.
//!
//! Runtime events only update state. The vault pass of [`poll`](crate::poll)
//! does all the planning, so a name moved by any means, locally or
//! remotely, is caught by the same pass.
//!
//! Names the host has open change through [`Effect`]s, so the host can
//! migrate live state. A name is then held until its local or remote head
//! moves, so an effect the host could not act on is not repeated every pass.
//! A failed step holds its name the same way, and records why in
//! [`VaultLink::failures`] until the name syncs. A fresh link clears every
//! hold and failure.
//!
//! The host persists [`VaultLink::synced`] whenever
//! [`VaultLink::synced_changed`] is set, after the registry. Persisting it
//! first could let a crash push an older head.

use crate::{Effect, JoinError, OpenHeads, Sessions, inbound, lifecycle::session_resolutions};
use gantz_ca as ca;
use gantz_collab::{
    Command, Event, Handle, ObjectRef, Objects, Outdated, PeerId, Push, VaultId, VaultTicket,
    VersionInfo, Want,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::mem;
use std::time::Duration;
use step::Step;

pub(crate) mod step;

/// This device's link to its vault and the sync state per name.
pub struct VaultLink {
    pub id: VaultId,
    /// The vault's identity.
    pub vault: PeerId,
    pub status: VaultStatus,
    /// What the vault speaks, as last heard. `None` until the vault first
    /// answers.
    pub vault_info: Option<VersionInfo>,
    /// Why each name that failed to sync did so. An entry stays until the
    /// name syncs or the link comes up again.
    pub failures: BTreeMap<ca::Name, String>,
    /// Names that never sync, such as the base graphs every build seeds.
    pub local_only: BTreeSet<ca::Name>,
    /// The vault head each name was last agreed at. The host persists it.
    pub synced: BTreeMap<ca::Name, ca::CommitAddr>,
    /// Set when `synced` changes. The host clears it once persisted.
    pub synced_changed: bool,
    /// The vault's heads as last heard. `None` until the link first comes
    /// up, since nothing can be planned against unknown heads.
    remote: Option<BTreeMap<ca::Name, ca::CommitAddr>>,
    /// Fetches of vault heads in flight, per name.
    fetches: HashMap<ca::Name, Fetch>,
    /// Pushes in flight, per name.
    pushes: HashMap<ca::Name, Option<ca::CommitAddr>>,
    /// Names left alone while their local and remote heads are these.
    held: HashMap<ca::Name, (Option<ca::CommitAddr>, Option<ca::CommitAddr>)>,
}

/// The state of a device's link to its vault. Names sync only while
/// [`VaultStatus::Live`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VaultStatus {
    /// The first link attempt is in flight.
    Connecting,
    /// The link is open.
    Live,
    /// The link dropped or failed to open, for this reason. It retries by
    /// itself.
    Offline(String),
    /// The vault refused this device, for this reason. That happens after
    /// the device is revoked, and only a new ticket fixes it.
    Denied(String),
    /// The vault only speaks older sync protocols. Only an update of the
    /// vault fixes it.
    VaultOutdated,
    /// The vault only speaks newer sync protocols. Only an update of this
    /// device fixes it.
    DeviceOutdated,
}

/// The outcome of a push on the vault's side.
#[derive(Debug)]
pub enum PushOutcome {
    /// The name moved. Send this [`Command::UpdateVault`] to mirror the
    /// change into the served store and notify linked devices.
    Accepted(Command),
    /// The vault's head was not the push's base. Holds the vault's head.
    Stale(Option<ca::CommitAddr>),
    /// The push's objects do not verify or do not complete its tip.
    Invalid(String),
}

/// A vault head's closure being fetched.
struct Fetch {
    tip: ca::CommitAddr,
    /// Commits this device already holds, which cut the vault's answer.
    have: Vec<ca::CommitAddr>,
    staged: ca::sync::Staged,
    /// The want in flight, which its answer echoes.
    want: Vec<ObjectRef>,
}

/// The state threaded through one pass.
struct Cx<'a> {
    registry: &'a mut ca::Registry,
    handle: &'a Handle,
    open: &'a OpenHeads,
    effects: &'a mut Vec<Effect>,
    /// Set when a name moved, so referrers resync once.
    resync: bool,
}

impl VaultLink {
    /// Record `head` as the agreed head for `name`, which clears its
    /// failure.
    fn set_synced(&mut self, name: &ca::Name, head: Option<ca::CommitAddr>) {
        if set(&mut self.synced, name, head) {
            self.synced_changed = true;
        }
        self.failures.remove(name);
    }

    /// Record why `name` failed to sync, and hold it until its local or
    /// remote head moves.
    fn fail(&mut self, registry: &ca::Registry, name: &ca::Name, reason: String) {
        log::warn!("vault: '{name}': {reason}");
        self.failures.insert(name.clone(), reason);
        self.hold(registry, name);
    }

    /// Hold `name` until its local or remote head moves.
    fn hold(&mut self, registry: &ca::Registry, name: &ca::Name) {
        let remote = self.remote_head(name);
        self.held
            .insert(name.clone(), (registry.head(name), remote));
    }

    fn remote_head(&self, name: &ca::Name) -> Option<ca::CommitAddr> {
        self.remote.as_ref().and_then(|r| r.get(name).copied())
    }
}

/// Link to the vault in `ticket`. The link opens in the background, and
/// names start syncing once the vault's heads arrive.
///
/// `synced` is the agreement persisted by an earlier run, empty on first
/// pairing. `local_only` names never sync.
pub fn link(
    sessions: &mut Sessions,
    handle: &Handle,
    ticket: VaultTicket,
    synced: BTreeMap<ca::Name, ca::CommitAddr>,
    local_only: BTreeSet<ca::Name>,
) -> Result<VaultId, JoinError> {
    let id = ticket.vault;
    let vault = ticket.host_id();
    handle
        .cmds
        .try_send(Command::Link(ticket))
        .map_err(|_| JoinError::RuntimeGone)?;
    sessions.vault = Some(VaultLink {
        id,
        vault,
        status: VaultStatus::Connecting,
        vault_info: None,
        failures: BTreeMap::new(),
        local_only,
        synced,
        synced_changed: false,
        remote: None,
        fetches: HashMap::new(),
        pushes: HashMap::new(),
        held: HashMap::new(),
    });
    Ok(id)
}

/// Drop the vault link. Local graphs stay as they are.
pub fn unlink(sessions: &mut Sessions, handle: &Handle) {
    if let Some(link) = sessions.vault.take() {
        let _ = handle.cmds.try_send(Command::Unlink(link.id));
    }
}

/// Whether `event` belongs to the vault link.
pub(crate) fn owns(link: &VaultLink, event: &Event) -> bool {
    match event {
        Event::LinkUp { vault, .. }
        | Event::LinkChanged { vault, .. }
        | Event::LinkDown { vault, .. }
        | Event::LinkIncompatible { vault, .. }
        | Event::LinkDenied { vault, .. }
        | Event::Pushed { vault, .. } => *vault == link.id,
        Event::Objects { session, .. } | Event::FetchFailed { session, .. } => *session == link.id,
        _ => false,
    }
}

/// Take the link down with `status`. Work in flight will not be answered.
fn down(link: &mut VaultLink, status: VaultStatus) {
    link.status = status;
    link.fetches.clear();
    link.pushes.clear();
}

/// Apply one vault event to the link's state. Fetched content is staged
/// and applied here. Planning waits for the next [`sync`].
pub(crate) fn handle_event(
    link: &mut VaultLink,
    registry: &mut ca::Registry,
    handle: &Handle,
    open: &OpenHeads,
    event: Event,
) {
    match event {
        Event::LinkUp { heads, info, .. } => {
            link.remote = Some(heads.into_iter().collect());
            link.status = VaultStatus::Live;
            link.vault_info = Some(info);
            link.fetches.clear();
            link.pushes.clear();
            link.held.clear();
            link.failures.clear();
        }
        Event::LinkChanged { changes, .. } => {
            if let Some(remote) = &mut link.remote {
                for (name, head) in changes {
                    set(remote, &name, head);
                }
            }
        }
        Event::LinkDown { error, .. } => {
            log::info!("vault link down: {error}");
            down(link, VaultStatus::Offline(error));
        }
        Event::LinkIncompatible {
            theirs, outdated, ..
        } => {
            log::warn!("vault on {} is incompatible with this gantz", theirs.app);
            let status = match outdated {
                Outdated::Us => VaultStatus::DeviceOutdated,
                Outdated::Them => VaultStatus::VaultOutdated,
            };
            link.vault_info = Some(theirs);
            down(link, status);
        }
        Event::LinkDenied { reason, .. } => {
            log::warn!("vault link denied: {reason}");
            down(link, VaultStatus::Denied(reason));
        }
        Event::Objects { want, objects, .. } => feed(link, registry, handle, open, want, objects),
        Event::FetchFailed { want, error, .. } => {
            let failed: Vec<ca::Name> = link
                .fetches
                .iter()
                .filter(|(_, f)| f.want == want.refs)
                .map(|(name, _)| name.clone())
                .collect();
            for name in failed {
                link.fetches.remove(&name);
                link.fail(registry, &name, format!("fetch failed: {error}"));
            }
        }
        Event::Pushed {
            name, tip, result, ..
        } => {
            link.pushes.remove(&name);
            match result {
                Ok(head) => {
                    if let Some(remote) = &mut link.remote {
                        set(remote, &name, head);
                    }
                    if head == tip {
                        link.set_synced(&name, tip);
                    }
                }
                Err(error) => link.fail(registry, &name, format!("push failed: {error}")),
            }
        }
        _ => (),
    }
}

/// Plan and act on every name out of step with the vault. Hosts reach it
/// through [`crate::poll`] each pass.
pub(crate) fn sync(
    link: &mut VaultLink,
    registry: &mut ca::Registry,
    handle: &Handle,
    open: &OpenHeads,
    effects: &mut Vec<Effect>,
) {
    if link.status != VaultStatus::Live {
        return;
    }
    let Some(remote) = &link.remote else {
        return;
    };
    let names: BTreeSet<ca::Name> = registry
        .heads()
        .map(|(n, _)| n)
        .chain(link.synced.keys())
        .chain(remote.keys())
        .filter(|n| {
            let synced = link.synced.get(*n).copied();
            registry.head(n) != synced || remote.get(*n).copied() != synced
        })
        .cloned()
        .collect();
    let mut cx = Cx {
        registry,
        handle,
        open,
        effects,
        resync: false,
    };
    for name in &names {
        reconcile(link, &mut cx, name);
    }
    if cx.resync {
        cx.effects.push(Effect::ResyncRefs);
    }
}

/// Bring one name one step closer to agreement with the vault.
fn reconcile(link: &mut VaultLink, cx: &mut Cx<'_>, name: &ca::Name) {
    if link.local_only.contains(name)
        || link.fetches.contains_key(name)
        || link.pushes.contains_key(name)
    {
        return;
    }
    let local = cx.registry.head(name);
    let remote = link.remote_head(name);
    if let Some(&held) = link.held.get(name) {
        if held == (local, remote) {
            return;
        }
        link.held.remove(name);
    }
    let base = link.synced.get(name).copied();
    if let Some(r) = remote.filter(|r| !cx.registry.commits().contains_key(r)) {
        fetch(link, cx, name, r, [local, base]);
        return;
    }
    match step::plan(cx.registry.commits(), local, base, remote) {
        Step::InSync => link.set_synced(name, local),
        // Planning again would find the same backwards move.
        Step::Keep => link.hold(cx.registry, name),
        Step::Adopt(target) => adopt(link, cx, name, target),
        Step::Push(tip) => push(link, cx, name, tip),
        Step::Merge {
            first,
            second,
            remote,
        } => merge(link, cx, name, first, second, remote),
        Step::Aside { local, remote } => aside(link, cx, name, local, remote),
    }
}

/// Set the local head to the vault's head exactly.
fn adopt(link: &mut VaultLink, cx: &mut Cx<'_>, name: &ca::Name, target: Option<ca::CommitAddr>) {
    if cx.open.contains_key(name) {
        cx.effects.push(Effect::Reset {
            name: name.clone(),
            to: target,
        });
        cx.resync = true;
        link.hold(cx.registry, name);
        return;
    }
    let from = cx.registry.head(name);
    match target {
        Some(to) => {
            cx.registry.set_head(name.clone(), to);
            cx.effects.push(Effect::Moved {
                session: link.id,
                name: name.clone(),
                from,
                to,
            });
            cx.resync = true;
        }
        None => {
            cx.registry.remove_head(name);
        }
    }
    link.set_synced(name, target);
}

/// Push the local head against the vault head this device last heard.
fn push(link: &mut VaultLink, cx: &mut Cx<'_>, name: &ca::Name, tip: Option<ca::CommitAddr>) {
    let base = link.remote_head(name);
    let objects = match tip {
        Some(tip) => gantz_collab::store::closure(cx.registry, &[tip], base.as_slice()),
        None => Objects::default(),
    };
    let push = Push {
        name: name.clone(),
        tip,
        base,
        objects,
    };
    let vault = link.id;
    if cx
        .handle
        .cmds
        .try_send(Command::Push { vault, push })
        .is_ok()
    {
        link.pushes.insert(name.clone(), tip);
    }
}

/// Merge the local head with the vault's, then push the merge.
fn merge(
    link: &mut VaultLink,
    cx: &mut Cx<'_>,
    name: &ca::Name,
    first: ca::CommitAddr,
    second: ca::CommitAddr,
    remote: ca::CommitAddr,
) {
    let resolutions = session_resolutions();
    if cx.open.contains_key(name) {
        // The host merges live, then the merge pushes on a later pass.
        cx.effects.push(Effect::RemoteTip {
            name: name.clone(),
            remote,
            resolutions,
            adopt_unrelated: false,
        });
        link.hold(cx.registry, name);
        return;
    }
    let from = cx.registry.head(name);
    match inbound::merge_headless(cx.registry, name, first, second, resolutions) {
        Ok(Some((to, _))) => {
            cx.effects.push(Effect::Moved {
                session: link.id,
                name: name.clone(),
                from,
                to,
            });
            cx.resync = true;
            push(link, cx, name, Some(to));
        }
        Ok(None) => link.hold(cx.registry, name),
        Err(e) => link.fail(cx.registry, name, format!("merge failed: {e}")),
    }
}

/// Move the local graph and its local-only nested graphs aside to a free
/// name, then adopt the vault's head. The aside names push as new names.
fn aside(
    link: &mut VaultLink,
    cx: &mut Cx<'_>,
    name: &ca::Name,
    local: ca::CommitAddr,
    remote: ca::CommitAddr,
) {
    let aside = free_name(cx.registry, name, local);
    // Synced nested graphs are shared with the vault, so they stay. The
    // fork still copies them, so the aside graph keeps its own.
    let moving: Vec<(ca::Name, ca::Name)> = cx
        .registry
        .heads()
        .map(|(n, _)| n)
        .filter(|n| *n != name && !link.synced.contains_key(*n))
        .filter_map(|n| Some((n.clone(), n.replace_prefix(name, &aside)?)))
        .collect();
    cx.registry.set_head(aside.clone(), local);
    gantz_egui::sync::fork_nested(cx.registry, now(), name, &aside);
    for (from, to) in moving {
        cx.registry.remove_head(&from);
        cx.effects.push(Effect::Renamed { from, to });
    }
    cx.registry.set_head(name.clone(), remote);
    cx.effects.push(Effect::Renamed {
        from: name.clone(),
        to: aside,
    });
    cx.effects.push(Effect::Moved {
        session: link.id,
        name: name.clone(),
        from: Some(local),
        to: remote,
    });
    cx.resync = true;
    link.set_synced(name, Some(remote));
}

/// Start fetching the closure of a vault head.
fn fetch(
    link: &mut VaultLink,
    cx: &mut Cx<'_>,
    name: &ca::Name,
    tip: ca::CommitAddr,
    have: [Option<ca::CommitAddr>; 2],
) {
    let have: Vec<ca::CommitAddr> = have
        .into_iter()
        .flatten()
        .filter(|ca| cx.registry.commits().contains_key(ca))
        .collect();
    let want = Want {
        refs: vec![ObjectRef::Closure {
            tips: vec![tip],
            have: have.clone(),
        }],
    };
    let fetch = Fetch {
        tip,
        have,
        staged: ca::sync::Staged::new(),
        want: want.refs.clone(),
    };
    let cmd = Command::Fetch {
        session: link.id,
        from: link.vault,
        want,
    };
    if cx.handle.cmds.try_send(cmd).is_ok() {
        link.fetches.insert(name.clone(), fetch);
    }
}

/// Stage the objects answering `want`. Apply each completed closure, or
/// fetch the frontier it still misses.
fn feed(
    link: &mut VaultLink,
    registry: &mut ca::Registry,
    handle: &Handle,
    open: &OpenHeads,
    want: Want,
    objects: gantz_collab::Objects,
) {
    let names: Vec<ca::Name> = link
        .fetches
        .iter()
        .filter(|(_, f)| f.want == want.refs)
        .map(|(name, _)| name.clone())
        .collect();
    let mut decoded = inbound::decode(objects);
    // An adopted view keeps the local camera of an open head, so syncing
    // never moves the viewport.
    let camera = names.iter().find_map(|n| open.get(n).copied().flatten());
    inbound::apply_sections(registry, mem::take(&mut decoded.sections), camera);
    for name in names {
        let Some(mut fetch) = link.fetches.remove(&name) else {
            continue;
        };
        let next = match inbound::advance(registry, &mut fetch.staged, fetch.tip, &decoded) {
            Ok(next) => next,
            Err(e) => {
                link.fail(registry, &name, format!("the vault's content {e}"));
                continue;
            }
        };
        if next.is_empty() {
            continue;
        }
        let next = frontier(next, &fetch.have);
        if next.refs == fetch.want {
            let reason = match decoded.errors.first() {
                Some(e) => format!("the vault sent content this gantz cannot read: {e}"),
                None => "the vault did not send the whole history".to_string(),
            };
            link.fail(registry, &name, reason);
            continue;
        }
        fetch.want = next.refs.clone();
        let cmd = Command::Fetch {
            session: link.id,
            from: link.vault,
            want: next,
        };
        if handle.cmds.try_send(cmd).is_ok() {
            link.fetches.insert(name, fetch);
        }
    }
}

/// Apply a device's push to the vault's registry, if it was made against
/// the vault's current head and its objects complete its tip.
pub fn serve_push(registry: &mut ca::Registry, vault: VaultId, push: Push) -> PushOutcome {
    let head = registry.head(&push.name);
    if head != push.base {
        return PushOutcome::Stale(head);
    }
    let Push {
        name, tip, objects, ..
    } = push;
    let Some(tip) = tip else {
        registry.remove_head(&name);
        let removed = update(vault, name, None, inbound::Decoded::default());
        return PushOutcome::Accepted(removed);
    };
    let decoded = inbound::decode(objects);
    let mut staged = ca::sync::Staged::new();
    match inbound::advance(registry, &mut staged, tip, &decoded) {
        Err(e) => return PushOutcome::Invalid(format!("push of '{name}' {e}")),
        Ok(next) if !next.is_empty() => {
            let reason = match decoded.errors.first() {
                Some(e) => format!("push of '{name}' is missing objects: {e}"),
                None => format!("push of '{name}' is missing objects"),
            };
            return PushOutcome::Invalid(reason);
        }
        Ok(_) => (),
    }
    inbound::apply_sections(registry, decoded.sections.clone(), None);
    registry.set_head(name.clone(), tip);
    PushOutcome::Accepted(update(vault, name, Some(tip), decoded))
}

/// The update mirroring an accepted push into the served store.
fn update(
    vault: VaultId,
    name: ca::Name,
    head: Option<ca::CommitAddr>,
    decoded: inbound::Decoded,
) -> Command {
    Command::UpdateVault {
        vault,
        heads: vec![(name, head)],
        commits: decoded.commits,
        graphs: decoded.graphs,
        sections: decoded.sections,
        blobs: decoded.blobs,
    }
}

/// Re-ask for the missing commits as one closure, so a long history takes
/// one round trip per budget-sized slice rather than one per commit.
fn frontier(want: Want, have: &[ca::CommitAddr]) -> Want {
    let (commits, mut rest): (Vec<ObjectRef>, Vec<ObjectRef>) = want
        .refs
        .into_iter()
        .partition(|r| matches!(r, ObjectRef::Commit(_)));
    let tips: Vec<ca::CommitAddr> = commits
        .into_iter()
        .filter_map(|r| match r {
            ObjectRef::Commit(ca) => Some(ca),
            _ => None,
        })
        .collect();
    if !tips.is_empty() {
        let closure = ObjectRef::Closure {
            tips,
            have: have.to_vec(),
        };
        rest.insert(0, closure);
    }
    Want { refs: rest }
}

/// `"{name}-{short}"` for the local head, with a counter if another graph
/// already holds it.
fn free_name(registry: &ca::Registry, name: &ca::Name, local: ca::CommitAddr) -> ca::Name {
    let base = format!("{name}-{}", local.display_short());
    let taken = |n: &ca::Name| registry.head(n).is_some_and(|h| h != local);
    let Ok(mut candidate) = base.parse::<ca::Name>();
    let mut i = 2;
    while taken(&candidate) {
        let Ok(next) = format!("{base}-{i}").parse();
        candidate = next;
        i += 1;
    }
    candidate
}

/// Insert or remove a map entry. Returns whether the map changed.
fn set(
    map: &mut BTreeMap<ca::Name, ca::CommitAddr>,
    name: &ca::Name,
    head: Option<ca::CommitAddr>,
) -> bool {
    let old = match head {
        Some(ca) => map.insert(name.clone(), ca),
        None => map.remove(name),
    };
    old != head
}

/// The wall clock, for the commits an aside mints.
fn now() -> Duration {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
}
