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
//! A name's metadata, such as its description, syncs the same way once its
//! head is in step. See [`gantz_collab::vault`]. The device tracks the
//! metadata digest it last agreed with and the vault's current one. It
//! pushes a change made only here, and fetches one made only on the vault.
//! When both changed, the vault's wins, as a push would find it stale.
//!
//! Runtime events only update state. The vault pass of [`poll`](crate::poll)
//! does all the planning, so a name moved by any means, locally or
//! remotely, is caught by the same pass.
//!
//! Names the host has open change through [`Effect`]s, so the host can
//! migrate live state. A name is then held until its local or remote state
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
    Command, Event, Handle, MetaChange, NameState, ObjectRef, Objects, Outdated, PeerId, Push,
    SectionEntry, VaultId, VaultTicket, VersionInfo, Want, store,
};
use serde::{Deserialize, Serialize};
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
    /// What each name was last agreed at. The host persists it.
    pub synced: Synced,
    /// Set when `synced` changes. The host clears it once persisted.
    pub synced_changed: bool,
    /// The vault's heads and metadata digests as last heard. Names sync only
    /// while the link is live, so only once the vault's heads arrived.
    remote: Synced,
    /// The work on each name between passes. A name has one piece at most.
    work: HashMap<ca::Name, Work>,
}

/// Heads and metadata digests per name.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Synced {
    pub heads: BTreeMap<ca::Name, ca::CommitAddr>,
    pub metas: BTreeMap<ca::Name, ca::ContentAddr>,
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
    /// The vault's head or metadata was not the push's base. Holds the
    /// name's state on the vault.
    Stale(NameState),
    /// The push's objects do not verify or do not complete its tip.
    Invalid(String),
}

/// The work on one name between passes. See [`VaultLink`].
enum Work {
    /// Its vault head's closure is being fetched.
    Fetch(Fetch),
    /// Its vault metadata is being fetched.
    FetchMeta,
    /// A new head is being pushed.
    PushHead,
    /// A metadata change is being pushed, with the digest of the metadata
    /// it sets.
    PushMeta(Option<ca::ContentAddr>),
    /// The name is left alone while its local and vault states are these.
    Held(NameState, NameState),
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

impl Synced {
    /// `name`'s head and metadata digest.
    fn state(&self, name: &ca::Name) -> NameState {
        NameState {
            head: self.heads.get(name).copied(),
            meta: self.metas.get(name).copied(),
        }
    }
}

impl VaultLink {
    /// Record `state` as the vault's state of `name`.
    fn set_remote(&mut self, name: &ca::Name, state: NameState) {
        set(&mut self.remote.heads, name, state.head);
        set(&mut self.remote.metas, name, state.meta);
    }

    /// Record `head` as the agreed head for `name`, which clears its
    /// failure. A removed name forgets its agreed metadata too.
    fn set_synced(&mut self, name: &ca::Name, head: Option<ca::CommitAddr>) {
        let mut changed = set(&mut self.synced.heads, name, head);
        if head.is_none() {
            changed |= set(&mut self.synced.metas, name, None);
        }
        self.synced_changed |= changed;
        self.failures.remove(name);
    }

    /// Record `meta` as the agreed metadata digest for `name`, which clears
    /// its failure.
    fn set_synced_meta(&mut self, name: &ca::Name, meta: Option<ca::ContentAddr>) {
        if set(&mut self.synced.metas, name, meta) {
            self.synced_changed = true;
        }
        self.failures.remove(name);
    }

    /// Record why `name` failed to sync, and hold it until its local or
    /// remote state moves.
    fn fail(&mut self, registry: &ca::Registry, name: &ca::Name, reason: String) {
        log::warn!("vault: '{name}': {reason}");
        self.failures.insert(name.clone(), reason);
        self.hold(registry, name);
    }

    /// Hold `name` until its local or remote state moves.
    fn hold(&mut self, registry: &ca::Registry, name: &ca::Name) {
        let (local, remote) = self.states(registry, name);
        self.work.insert(name.clone(), Work::Held(local, remote));
    }

    /// Send `cmd` for `name`, and record `work` once it is sent.
    fn start(&mut self, handle: &Handle, name: &ca::Name, cmd: Command, work: Work) {
        if handle.cmds.try_send(cmd).is_ok() {
            self.work.insert(name.clone(), work);
        }
    }

    /// `name`'s local and vault states.
    fn states(&self, registry: &ca::Registry, name: &ca::Name) -> (NameState, NameState) {
        (store::name_state(registry, name), self.remote.state(name))
    }

    fn remote_head(&self, name: &ca::Name) -> Option<ca::CommitAddr> {
        self.remote.heads.get(name).copied()
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
    synced: Synced,
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
        remote: Synced::default(),
        work: HashMap::new(),
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

/// Set the link's status, and drop all work. Work in flight will not be
/// answered, and a new link plans every name afresh.
fn reset(link: &mut VaultLink, status: VaultStatus) {
    link.status = status;
    link.work.clear();
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
        Event::LinkUp {
            heads, metas, info, ..
        } => {
            link.remote = Synced {
                heads: heads.into_iter().collect(),
                metas: metas.into_iter().collect(),
            };
            link.vault_info = Some(info);
            link.failures.clear();
            reset(link, VaultStatus::Live);
        }
        Event::LinkChanged { name, state, .. } => link.set_remote(&name, state),
        Event::LinkDown { error, .. } => {
            log::info!("vault link down: {error}");
            reset(link, VaultStatus::Offline(error));
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
            reset(link, status);
        }
        Event::LinkDenied { reason, .. } => {
            log::warn!("vault link denied: {reason}");
            reset(link, VaultStatus::Denied(reason));
        }
        Event::Objects { want, objects, .. } => match meta_fetch(link, &want) {
            Some(name) => feed_meta(link, registry, &name, objects),
            None => feed(link, registry, handle, open, want, objects),
        },
        // A failure holds its name, which ends the fetch.
        Event::FetchFailed { want, error, .. } => match meta_fetch(link, &want) {
            Some(name) => link.fail(registry, &name, format!("metadata fetch failed: {error}")),
            None => {
                for name in fetches(link, &want) {
                    link.fail(registry, &name, format!("fetch failed: {error}"));
                }
            }
        },
        Event::Pushed {
            name, tip, result, ..
        } => {
            // An answer from before the link last came up finds other work.
            let pushing = match link.work.get(&name) {
                Some(Work::PushHead | Work::PushMeta(_)) => link.work.remove(&name),
                _ => None,
            };
            match result {
                Ok(state) => {
                    link.set_remote(&name, state);
                    match pushing {
                        Some(Work::PushHead) if state.head == tip => link.set_synced(&name, tip),
                        Some(Work::PushMeta(meta)) if state.meta == meta => {
                            link.set_synced_meta(&name, meta)
                        }
                        _ => (),
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
    let remote = &link.remote;
    let names: BTreeSet<ca::Name> = registry
        .heads()
        .map(|(n, _)| n)
        .chain(link.synced.heads.keys())
        .chain(remote.heads.keys())
        .filter(|n| !link.local_only.contains(*n))
        .filter(|n| {
            let synced = link.synced.state(n);
            store::name_state(registry, n) != synced || remote.state(n) != synced
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
    // Work in flight waits for its answer. A hold lasts until the name's
    // local or vault state moves.
    if let Some(work) = link.work.get(name) {
        let Work::Held(local, remote) = work else {
            return;
        };
        if (*local, *remote) == link.states(cx.registry, name) {
            return;
        }
        link.work.remove(name);
    }
    let local = cx.registry.head(name);
    let remote = link.remote_head(name);
    let base = link.synced.heads.get(name).copied();
    if let Some(r) = remote.filter(|r| !cx.registry.commits().contains_key(r)) {
        fetch(link, cx, name, r, [local, base]);
        return;
    }
    match step::plan(cx.registry.commits(), local, base, remote) {
        Step::InSync => {
            link.set_synced(name, local);
            sync_meta(link, cx, name);
        }
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
        meta: None,
    };
    let cmd = Command::Push {
        vault: link.id,
        push,
    };
    link.start(cx.handle, name, cmd, Work::PushHead);
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
        .filter(|n| *n != name && !link.synced.heads.contains_key(*n))
        .filter_map(|n| Some((n.clone(), n.replace_prefix(name, &aside)?)))
        .collect();
    // Metadata moves with the local graph it describes, so the vault's
    // metadata for the name wins.
    cx.registry.set_head(aside.clone(), local);
    move_meta(cx.registry, name, &aside);
    gantz_egui::sync::fork_nested(cx.registry, now(), name, &aside);
    for (from, to) in moving {
        move_meta(cx.registry, &from, &to);
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
    link.set_synced_meta(name, None);
}

/// Bring a name's metadata into agreement with the vault, once its head is.
fn sync_meta(link: &mut VaultLink, cx: &mut Cx<'_>, name: &ca::Name) {
    let Some(head) = cx.registry.head(name) else {
        return;
    };
    let local = store::meta_addr(cx.registry, name);
    let remote = link.remote.state(name).meta;
    let synced = link.synced.metas.get(name).copied();
    if local == remote {
        link.set_synced_meta(name, local);
    } else if remote == synced {
        push_meta(link, cx, name, head, remote);
    } else {
        fetch_meta(link, cx, name);
    }
}

/// Push the local metadata against the vault's, with the head unchanged.
fn push_meta(
    link: &mut VaultLink,
    cx: &mut Cx<'_>,
    name: &ca::Name,
    head: ca::CommitAddr,
    base: Option<ca::ContentAddr>,
) {
    let entries = store::name_meta(cx.registry, name);
    let meta = store::meta_digest(&entries);
    let push = Push {
        name: name.clone(),
        tip: Some(head),
        base: Some(head),
        objects: Objects::default(),
        meta: Some(MetaChange { base, entries }),
    };
    let cmd = Command::Push {
        vault: link.id,
        push,
    };
    link.start(cx.handle, name, cmd, Work::PushMeta(meta));
}

/// Start fetching the vault's metadata for `name`.
fn fetch_meta(link: &mut VaultLink, cx: &mut Cx<'_>, name: &ca::Name) {
    let want = Want {
        refs: vec![ObjectRef::Meta(name.clone())],
    };
    let cmd = Command::Fetch {
        session: link.id,
        from: link.vault,
        want,
    };
    link.start(cx.handle, name, cmd, Work::FetchMeta);
}

/// The name whose metadata fetch `want` answers, if one is in flight.
fn meta_fetch(link: &VaultLink, want: &Want) -> Option<ca::Name> {
    match &want.refs[..] {
        [ObjectRef::Meta(name)] if matches!(link.work.get(name), Some(Work::FetchMeta)) => {
            Some(name.clone())
        }
        _ => None,
    }
}

/// The names whose head fetch `want` answers.
fn fetches(link: &VaultLink, want: &Want) -> Vec<ca::Name> {
    link.work
        .iter()
        .filter(|(_, w)| matches!(w, Work::Fetch(f) if f.want == want.refs))
        .map(|(name, _)| name.clone())
        .collect()
}

/// Replace a name's metadata with the vault's. Metadata that this build
/// cannot store exactly, such as entries it cannot read, is not agreed. The
/// name is held instead of fetched again each pass.
fn feed_meta(link: &mut VaultLink, registry: &mut ca::Registry, name: &ca::Name, objects: Objects) {
    link.work.remove(name);
    let fetched = store::meta_digest(&objects);
    store::set_name_meta(registry, name, inbound::decode(objects).sections);
    if store::meta_addr(registry, name) == fetched {
        link.set_synced_meta(name, fetched);
    } else {
        let reason = "this gantz cannot store the vault's metadata exactly";
        link.fail(registry, name, reason.to_string());
    }
}

/// Move `from`'s metadata to `to`.
fn move_meta(registry: &mut ca::Registry, from: &ca::Name, to: &ca::Name) {
    let (key, to_key) = (ca::Key::Name(from.clone()), ca::Key::Name(to.clone()));
    let entries: Vec<_> = registry
        .sections()
        .iter()
        .filter(|(id, _)| id.as_str() != ca::HEADS_ID)
        .filter_map(|(id, section)| {
            let value = section.entries.get(&key)?.clone();
            Some((
                id.clone(),
                section.policy,
                section.liveness,
                to_key.clone(),
                value,
            ))
        })
        .collect();
    store::set_name_meta(registry, to, entries);
    store::set_name_meta(registry, from, Vec::new());
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
    link.start(cx.handle, name, cmd, Work::Fetch(fetch));
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
    let names = fetches(link, &want);
    let mut decoded = inbound::decode(objects);
    // An adopted view keeps the local camera of an open head, so syncing
    // never moves the viewport.
    let camera = names.iter().find_map(|n| open.get(n).copied().flatten());
    inbound::apply_sections(registry, mem::take(&mut decoded.sections), camera);
    for name in names {
        let Some(Work::Fetch(mut fetch)) = link.work.remove(&name) else {
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
        link.start(handle, &name, cmd, Work::Fetch(fetch));
    }
}

/// Apply a device's push to the vault's registry, if it was made against
/// the vault's current head and metadata, and its objects complete its tip.
/// A removal drops the name's metadata too.
pub fn serve_push(registry: &mut ca::Registry, vault: VaultId, push: Push) -> PushOutcome {
    let state = store::name_state(registry, &push.name);
    let meta_stale = push.meta.as_ref().is_some_and(|m| m.base != state.meta);
    if state.head != push.base || meta_stale {
        return PushOutcome::Stale(state);
    }
    let Push {
        name,
        tip,
        objects,
        meta,
        ..
    } = push;
    let Some(tip) = tip else {
        registry.remove_head(&name);
        store::set_name_meta(registry, &name, Vec::new());
        let removed = update(vault, name, None, Some(Vec::new()), Default::default());
        return PushOutcome::Accepted(removed);
    };
    // The metadata replaces the vault's whole, so it may hold this name's
    // entries alone.
    let meta = meta.map(|m| inbound::decode(m.entries).sections);
    let key = ca::Key::Name(name.clone());
    let foreign = |(id, _, _, k, _): &SectionEntry| *k != key || id == ca::HEADS_ID;
    if meta.iter().flatten().any(foreign) {
        return PushOutcome::Invalid(format!("metadata for '{name}' holds other entries"));
    }
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
    if let Some(entries) = &meta {
        store::set_name_meta(registry, &name, entries.clone());
    }
    registry.set_head(name.clone(), tip);
    PushOutcome::Accepted(update(vault, name, Some(tip), meta, decoded))
}

/// The update mirroring an accepted push of `name` into the served store.
fn update(
    vault: VaultId,
    name: ca::Name,
    head: Option<ca::CommitAddr>,
    meta: Option<Vec<SectionEntry>>,
    decoded: inbound::Decoded,
) -> Command {
    Command::UpdateVault {
        vault,
        name,
        head,
        meta,
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
fn set<V: Copy + PartialEq>(
    map: &mut BTreeMap<ca::Name, V>,
    name: &ca::Name,
    value: Option<V>,
) -> bool {
    let old = match value {
        Some(v) => map.insert(name.clone(), v),
        None => map.remove(name),
    };
    old != value
}

/// The wall clock, for the commits an aside mints.
fn now() -> Duration {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default()
}
