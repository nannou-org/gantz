//! Converging a head with another tip: local branch merges ([`merge_head`])
//! and session remote-tip syncs ([`sync_remote_tip`]), plus the shared
//! VM-state/layout/selection migration through a merge outcome.

use super::{cascade_pos, node_id};
use crate::widget::graph_scene::NodeIndex;
use gantz_ca::CommitAddr;
use gantz_ca::DataGraph;
use gantz_core::node;
use steel::steel_vm::engine::Engine;

/// Migrate a head's index-keyed VM state, layout and selection through a
/// merge outcome's node provenance, seeding layout for merged-in nodes from
/// the other side's persisted view (typically read from the registry's view
/// section; falling back to placement near the view centre - positions are
/// compatible because both sides share the base's coordinates).
///
/// `local_side` is the side the head's working graph played in the merge:
/// [`gantz_ca::Side::Ours`] for a branch merge into the head
/// ([`merge_head`]); sessions pass whichever side the local tip landed on
/// after canonical orientation ([`sync_remote_tip`]). `other_view` is the
/// opposite side's commit's stored view, if any.
///
/// Returns the mapping from pre-merge working-graph indices to merged
/// indices (identity whenever the other side removed no nodes), for any
/// remaining index-keyed data of the caller's.
pub(crate) fn apply_merge_migration(
    node_srcs: &[gantz_ca::merge::NodeSrc],
    local_side: gantz_ca::merge::Side,
    other_view: Option<&crate::SceneView>,
    vm: &mut Engine,
    head_view: &mut crate::SceneView,
    selection: &mut crate::widget::graph_scene::Selection,
) -> gantz_ca::Matching {
    // Where each pre-merge (local) node ended up, and where each node that
    // exists only on the other side ended up.
    let sides = |src: &gantz_ca::merge::NodeSrc| match local_side {
        gantz_ca::merge::Side::Ours => (src.ours, src.theirs),
        gantz_ca::merge::Side::Theirs => (src.theirs, src.ours),
    };
    let mut local_map = gantz_ca::Matching::new();
    let mut other_only = Vec::new();
    for (m, src) in node_srcs.iter().enumerate() {
        match sides(src) {
            (Some(l), _) => {
                local_map.insert(l, m);
            }
            (None, Some(o)) => other_only.push((m, o)),
            (None, None) => unreachable!("a merged node comes from somewhere"),
        }
    }

    // Migrate the index-keyed VM state, layout and selection. When the other
    // side removed no nodes the mapping is identity and this is a no-op.
    if let Err(e) = node::state::remap_root(vm, &local_map) {
        log::error!("merge migration: failed to remap node state: {e}");
    }
    let old_layout = std::mem::take(&mut head_view.layout);
    for (&l, &m) in &local_map {
        if let Some(pos) = old_layout.get(&node_id(l)) {
            head_view.layout.insert(node_id(m), *pos);
        }
    }
    selection.nodes = selection
        .nodes
        .iter()
        .filter_map(|n| local_map.get(&n.index()).map(|&m| NodeIndex::new(m)))
        .collect();
    selection.edges.clear();

    // Seed layout for merged-in nodes from the other side's persisted view.
    for (i, &(m, o)) in other_only.iter().enumerate() {
        let pos = other_view
            .and_then(|v| v.layout.get(&node_id(o)).copied())
            .unwrap_or_else(|| cascade_pos(head_view.camera.center, i));
        head_view.layout.insert(node_id(m), pos);
    }
    local_map
}

/// The result of a [`merge_head`] call.
#[derive(Debug)]
pub enum MergeHeadOutcome {
    /// Ours had no changes since the merge base: nothing was mutated and no
    /// commit was made. The caller navigates the head to this commit, which
    /// reloads the working graph and views.
    FastForward(CommitAddr),
    /// The merge was applied to the working graph and committed with two
    /// parents; `head` has been advanced. `mapping` records where each of the
    /// pre-merge graph's nodes ended up (old index to new index; absent =
    /// removed), for any remaining index-keyed data of the caller's. The
    /// caller re-registers the graph with the VM (merged-in nodes need their
    /// state initialized) and fires its committed/resync machinery.
    Merged {
        new_commit: CommitAddr,
        mapping: gantz_ca::Matching,
    },
    /// Conflicts (without `auto_resolve`) or hard blockers refused the merge;
    /// nothing was mutated. Carries the rendered reasons.
    Refused(Vec<String>),
    /// Nothing to do: unknown source, unrelated histories, or already up to
    /// date.
    Noop,
}

/// Merge the branch named `source` into `head`, applying the result to the
/// head's working `graph` in place (see [`gantz_ca::merge_commits`]).
///
/// On a true merge this migrates the index-keyed VM state, layout and
/// selection through the merged graph's node mapping (an identity mapping
/// whenever the source branch removed no nodes), seeds layout for merged-in
/// nodes from the source branch's persisted view in the registry's view
/// section (falling back to placement near the view centre), and commits the
/// result with two parents via
/// [`gantz_ca::Registry::commit_merge_to_head`] - upholding the
/// committed-working-graph invariant, so callers must not commit again.
///
/// Conflicts refuse the merge unless `auto_resolve` accepts the given
/// `resolutions`; hard blockers (a merged-in reference cycle) always refuse.
/// Fast-forwards mutate nothing - the caller navigates the head instead.
#[allow(clippy::too_many_arguments)]
pub fn merge_head(
    registry: &mut gantz_ca::Registry,
    timestamp: gantz_ca::Timestamp,
    head: &mut gantz_ca::Head,
    graph: &mut DataGraph,
    vm: &mut Engine,
    head_view: &mut crate::SceneView,
    selection: &mut crate::widget::graph_scene::Selection,
    source: &str,
    resolutions: gantz_ca::Resolutions,
    auto_resolve: bool,
) -> MergeHeadOutcome {
    let Some(ours_tip) = registry.head_commit_ca(head) else {
        log::error!("MergeHead: no commit for head {head}");
        return MergeHeadOutcome::Noop;
    };
    let Some(theirs_tip) = registry.head(&source.parse().expect("infallible")) else {
        log::error!("MergeHead: unknown source branch '{source}'");
        return MergeHeadOutcome::Noop;
    };
    let outcome = match gantz_ca::merge_commits(registry, ours_tip, theirs_tip, resolutions) {
        Err(e) => {
            log::warn!("MergeHead: cannot merge '{source}': {e}");
            return MergeHeadOutcome::Noop;
        }
        Ok(gantz_ca::MergeResolution::AlreadyUpToDate) => return MergeHeadOutcome::Noop,
        Ok(gantz_ca::MergeResolution::FastForward) => {
            return MergeHeadOutcome::FastForward(theirs_tip);
        }
        Ok(gantz_ca::MergeResolution::Diverged { outcome, .. }) => outcome,
    };

    // Refuse on hard blockers, and on conflicts unless the caller opted into
    // the selected resolutions.
    let blockers = crate::merge::merge_blockers(registry, head, &outcome.graph);
    if !blockers.is_empty() {
        return MergeHeadOutcome::Refused(blockers);
    }
    if !outcome.conflicts.is_empty() && !auto_resolve {
        return MergeHeadOutcome::Refused(crate::merge::conflict_strings(&outcome.conflicts));
    }

    // Migrate the index-keyed VM state, layout and selection through the
    // merged indices; the head's working graph plays the ours side (by the
    // committed-working-graph invariant it *is* ours' tip graph). Layout for
    // merged-in nodes seeds from the source branch's persisted view.
    let theirs_view = crate::section::view(registry, &theirs_tip);
    let ours_map = apply_merge_migration(
        &outcome.node_srcs,
        gantz_ca::merge::Side::Ours,
        theirs_view.as_ref(),
        vm,
        head_view,
        selection,
    );

    // Swap the merged data graph straight in as the working graph and commit
    // it with both parents, so the registry address matches the working
    // content by construction.
    let merged_data = outcome.graph;
    *graph = merged_data.clone();
    let new_commit = registry.commit_merge_to_head(
        timestamp,
        gantz_ca::graph_addr(&merged_data),
        || merged_data,
        theirs_tip,
        head,
    );
    MergeHeadOutcome::Merged {
        new_commit,
        mapping: ours_map,
    }
}

/// The result of a [`sync_remote_tip`] call.
#[derive(Debug)]
pub enum SyncTipOutcome {
    /// The local tip already contains the remote tip; nothing was mutated.
    UpToDate,
    /// No commit was minted: the caller navigates the head to this commit
    /// (a fast-forward, or the deterministic winner of a same-graph "twin"
    /// adoption - see [`gantz_ca::SyncStep::Adopt`]).
    Moved(CommitAddr),
    /// A canonical merge commit was minted and `head` advanced; the merged
    /// graph was swapped into the working graph. As with
    /// [`MergeHeadOutcome::Merged`], the caller re-registers the graph with
    /// the VM and fires its committed/resync machinery. Session conflicts
    /// are auto-resolved by the session's resolutions; `conflicts` carries
    /// the count for surfacing.
    Merged {
        new_commit: CommitAddr,
        mapping: gantz_ca::Matching,
        conflicts: usize,
    },
    /// Hard blockers (a merged-in reference cycle) or missing registry
    /// content refused the merge; nothing was mutated.
    Blocked(Vec<String>),
    /// The tips share no common ancestor: surfaced to the app (e.g. rename
    /// the local graph aside), never resolved automatically.
    Unrelated,
}

/// Bring `head` up to date with a `remote` tip received from a session peer,
/// applying [`gantz_ca::plan_sync_step`]'s decision to the head's working
/// `graph` in place.
///
/// The session analogue of [`merge_head`], driven by a commit address rather
/// than a branch name. Diverged graphs merge in *canonical orientation* via
/// [`gantz_ca::Registry::commit_merge_canonical`] (no timestamp parameter:
/// it is derived from the tips), so every peer merging the same pair mints
/// the identical commit. VM state, layout and selection migrate through the
/// merged indices for whichever side the local tip played; conflicts are
/// auto-resolved per `resolutions` (the fixed session policy) and surfaced
/// as a count.
///
/// The remote tip's closure must already be in the registry (fetched and
/// applied via [`gantz_ca::sync::Staged`]). On [`SyncTipOutcome::Merged`]
/// the committed-working-graph invariant is upheld - callers must not commit
/// again.
///
/// `adopt_unrelated` adopts a remote tip that shares no local history
/// instead of surfacing [`SyncTipOutcome::Unrelated`]: the join flow's
/// placeholder head (an empty graph minted so the session's tab opens
/// immediately) is deliberately unrelated to the session content it awaits.
#[allow(clippy::too_many_arguments)]
pub fn sync_remote_tip(
    registry: &mut gantz_ca::Registry,
    head: &mut gantz_ca::Head,
    graph: &mut DataGraph,
    vm: &mut Engine,
    head_view: &mut crate::SceneView,
    selection: &mut crate::widget::graph_scene::Selection,
    remote: CommitAddr,
    resolutions: gantz_ca::Resolutions,
    adopt_unrelated: bool,
) -> SyncTipOutcome {
    let Some(local) = registry.head_commit_ca(head) else {
        log::error!("sync_remote_tip: no commit for head {head}");
        return SyncTipOutcome::UpToDate;
    };
    let (first, second) = match gantz_ca::plan_sync_step(registry.commits(), local, remote) {
        gantz_ca::SyncStep::UpToDate => return SyncTipOutcome::UpToDate,
        gantz_ca::SyncStep::FastForward(t) => return SyncTipOutcome::Moved(t),
        gantz_ca::SyncStep::Adopt(t) if t == local => return SyncTipOutcome::UpToDate,
        gantz_ca::SyncStep::Adopt(t) => return SyncTipOutcome::Moved(t),
        gantz_ca::SyncStep::Unrelated if adopt_unrelated => {
            return SyncTipOutcome::Moved(remote);
        }
        gantz_ca::SyncStep::Unrelated => return SyncTipOutcome::Unrelated,
        gantz_ca::SyncStep::Merge { first, second } => (first, second),
    };
    let outcome = match gantz_ca::merge_commits(registry, first, second, resolutions) {
        // The plan and the merge read the same commits, so these arms are
        // unreachable in practice; hold the plan's meaning if they change.
        Ok(gantz_ca::MergeResolution::AlreadyUpToDate) => return SyncTipOutcome::UpToDate,
        Ok(gantz_ca::MergeResolution::FastForward) => return SyncTipOutcome::Moved(remote),
        Err(e) => {
            log::warn!("sync_remote_tip: cannot merge remote tip: {e}");
            return SyncTipOutcome::Blocked(vec![e.to_string()]);
        }
        Ok(gantz_ca::MergeResolution::Diverged { outcome, .. }) => outcome,
    };

    let blockers = crate::merge::merge_blockers(registry, head, &outcome.graph);
    if !blockers.is_empty() {
        return SyncTipOutcome::Blocked(blockers);
    }

    // Which side the local tip played after canonical orientation.
    let (local_side, other_tip) = if first == local {
        (gantz_ca::merge::Side::Ours, second)
    } else {
        (gantz_ca::merge::Side::Theirs, first)
    };
    let other_view = crate::section::view(registry, &other_tip);
    let mapping = apply_merge_migration(
        &outcome.node_srcs,
        local_side,
        other_view.as_ref(),
        vm,
        head_view,
        selection,
    );
    let conflicts = outcome.conflicts.len();

    // Swap in the merged graph and mint the canonical merge commit.
    *graph = outcome.graph;
    let new_commit = registry.commit_merge_canonical(
        first,
        second,
        gantz_ca::graph_addr(&*graph),
        || graph.clone(),
        head,
    );
    SyncTipOutcome::Merged {
        new_commit,
        mapping,
        conflicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::node_id;
    use crate::ops::test_util::*;
    use crate::widget::graph_scene::Selection;
    use gantz_core::ROOT_STATE;
    use gantz_core::node::graph::NodeIx;
    use steel::SteelVal;

    #[allow(clippy::type_complexity)]
    fn run_merge(
        reg: &mut gantz_ca::Registry,
        head: &mut gantz_ca::Head,
        graph: &mut DataGraph,
        vm: &mut Engine,
        view: &mut crate::SceneView,
        selection: &mut Selection,
        auto_resolve: bool,
    ) -> MergeHeadOutcome {
        merge_head(
            reg,
            std::time::Duration::from_secs(9),
            head,
            graph,
            vm,
            view,
            selection,
            "beta",
            gantz_ca::Resolutions::default(),
            auto_resolve,
        )
    }

    // Ours edited a node while theirs added one: the merge keeps ours' indices
    // (identity mapping), applies theirs' addition, and commits two parents.
    #[test]
    fn merge_head_applies_theirs_and_commits_two_parents() {
        let (mut reg, mut head) = diverged_registry(&[1, 2], &[1, 20], &[1, 2, 3]);
        let ours_tip = reg.head_commit_ca(&head).unwrap();
        let theirs_tip = reg.head(&"beta".parse().unwrap()).unwrap();
        let mut graph = test_graph(&[1, 20]);
        let mut vm = Engine::new_base();
        let mut view = crate::SceneView::default();
        view.layout.insert(node_id(0), egui::pos2(0.0, 0.0));
        view.layout.insert(node_id(1), egui::pos2(1.0, 0.0));
        let mut selection = Selection::default();
        selection.nodes.insert(NodeIx::new(1));

        let outcome = run_merge(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            false,
        );
        let MergeHeadOutcome::Merged { new_commit, .. } = outcome else {
            panic!("expected Merged, got {outcome:?}");
        };

        // The merged graph keeps ours' nodes in place and appends theirs' add.
        let weights: Vec<u32> = graph.node_weights().map(value).collect();
        assert_eq!(weights, vec![1, 20, 3]);
        // Ours' layout and selection are untouched; the merged-in node has a
        // (fallback) layout entry.
        assert_eq!(view.layout.get(&node_id(1)), Some(&egui::pos2(1.0, 0.0)));
        assert!(view.layout.contains_key(&node_id(2)));
        assert!(selection.nodes.contains(&NodeIx::new(1)));
        // The commit joins both parents and the head advanced to it.
        let commit = &reg.commits()[&new_commit];
        assert_eq!(commit.parent, Some(ours_tip));
        assert_eq!(commit.merge_parents, vec![theirs_tip]);
        assert_eq!(reg.head_commit_ca(&head), Some(new_commit));
    }

    // Theirs removed a node: ours' surviving state/layout/selection migrate
    // through the returned mapping.
    #[test]
    fn merge_head_migrates_state_layout_selection_on_removal() {
        let (mut reg, mut head) = diverged_registry(&[1, 2], &[1, 2], &[2]);
        let mut graph = test_graph(&[1, 2]);
        let mut vm = Engine::new_base();
        vm.register_value(ROOT_STATE, SteelVal::empty_hashmap());
        node::state::update_value(&mut vm, &[1], SteelVal::IntV(42)).unwrap();
        let mut view = crate::SceneView::default();
        view.layout.insert(node_id(0), egui::pos2(0.0, 0.0));
        view.layout.insert(node_id(1), egui::pos2(1.0, 0.0));
        let mut selection = Selection::default();
        selection.nodes.insert(NodeIx::new(1));

        let outcome = run_merge(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            false,
        );
        let MergeHeadOutcome::Merged { mapping, .. } = outcome else {
            panic!("expected Merged, got {outcome:?}");
        };

        // Node 2 (ours ix 1) survives at merged ix 0.
        assert_eq!(mapping, gantz_ca::Matching::from([(1, 0)]));
        let weights: Vec<u32> = graph.node_weights().map(value).collect();
        assert_eq!(weights, vec![2]);
        // Its state, layout and selection followed.
        let state = node::state::extract_value(&vm, &[0]).unwrap();
        assert_eq!(state, Some(SteelVal::IntV(42)));
        assert_eq!(view.layout.len(), 1);
        assert_eq!(view.layout.get(&node_id(0)), Some(&egui::pos2(1.0, 0.0)));
        assert_eq!(
            selection.nodes.iter().copied().collect::<Vec<_>>(),
            vec![NodeIx::new(0)],
        );
    }

    // Conflicting edits refuse the merge (mutating nothing) unless the caller
    // opts into the default resolutions.
    #[test]
    fn merge_head_refuses_conflicts_unless_auto_resolve() {
        let (mut reg, mut head) = diverged_registry(&[1, 2], &[1, 20], &[1, 30]);
        let ours_tip = reg.head_commit_ca(&head).unwrap();
        let mut graph = test_graph(&[1, 20]);
        let mut vm = Engine::new_base();
        let mut view = crate::SceneView::default();
        let mut selection = Selection::default();

        let outcome = run_merge(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            false,
        );
        let MergeHeadOutcome::Refused(reasons) = outcome else {
            panic!("expected Refused, got {outcome:?}");
        };
        assert!(!reasons.is_empty());
        // Nothing moved.
        assert_eq!(reg.head_commit_ca(&head), Some(ours_tip));
        assert_eq!(graph.node_weights().map(value).collect::<Vec<_>>(), [1, 20]);

        // Opting in applies the default resolution (ours wins).
        let outcome = run_merge(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            true,
        );
        assert!(matches!(outcome, MergeHeadOutcome::Merged { .. }));
        assert_eq!(graph.node_weights().map(value).collect::<Vec<_>>(), [1, 20]);
        assert_ne!(reg.head_commit_ca(&head), Some(ours_tip));
    }

    // A source branch that is strictly ahead fast-forwards without a commit.
    #[test]
    fn merge_head_fast_forwards() {
        let mut reg = gantz_ca::Registry::default();
        let base_ca = commit_test_graph(&mut reg, 1, None, &test_graph(&[1]));
        let theirs_ca = commit_test_graph(&mut reg, 2, Some(base_ca), &test_graph(&[1, 2]));
        reg.set_head("alpha".parse().unwrap(), base_ca);
        reg.set_head("beta".parse().unwrap(), theirs_ca);
        let mut head = gantz_ca::Head::Branch("alpha".parse().unwrap());
        let mut graph = test_graph(&[1]);
        let mut vm = Engine::new_base();
        let mut view = crate::SceneView::default();
        let mut selection = Selection::default();

        let outcome = run_merge(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            false,
        );
        let MergeHeadOutcome::FastForward(target) = outcome else {
            panic!("expected FastForward, got {outcome:?}");
        };
        assert_eq!(target, theirs_ca);
        // Nothing mutated: navigation is the caller's job.
        assert_eq!(reg.head(&"alpha".parse().unwrap()), Some(base_ca));
        assert_eq!(graph.node_count(), 1);
    }

    /// The fixed session policy used by the `sync_remote_tip` tests.
    fn session_resolutions() -> gantz_ca::Resolutions {
        gantz_ca::Resolutions {
            both_modified: gantz_ca::BothModified::KeepNewest,
            delete_modify: Default::default(),
        }
    }

    #[allow(clippy::type_complexity)]
    fn run_sync(
        reg: &mut gantz_ca::Registry,
        head: &mut gantz_ca::Head,
        graph: &mut DataGraph,
        vm: &mut Engine,
        view: &mut crate::SceneView,
        selection: &mut Selection,
        remote: CommitAddr,
    ) -> SyncTipOutcome {
        sync_remote_tip(
            reg,
            head,
            graph,
            vm,
            view,
            selection,
            remote,
            session_resolutions(),
            false,
        )
    }

    // The join flow's placeholder head adopts an unrelated remote tip
    // instead of surfacing it.
    #[test]
    fn sync_remote_tip_adopts_unrelated_when_asked() {
        let secs = |s| std::time::Duration::from_secs(s);
        let mut reg = gantz_ca::Registry::default();
        let g = test_graph(&[]);
        let placeholder = reg.commit_graph(secs(1), None, gantz_ca::graph_addr(&g), || g);
        let g = test_graph(&[9]);
        let foreign = reg.commit_graph(secs(2), None, gantz_ca::graph_addr(&g), || g);
        reg.set_head("alpha".parse().unwrap(), placeholder);
        let mut head = gantz_ca::Head::Branch("alpha".parse().unwrap());
        let mut graph = test_graph(&[]);
        let mut vm = Engine::new_base();
        let mut view = crate::SceneView::default();
        let mut selection = Selection::default();
        let outcome = sync_remote_tip(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            foreign,
            session_resolutions(),
            true,
        );
        // Navigation is the caller's job: the outcome names the target.
        assert!(matches!(outcome, SyncTipOutcome::Moved(t) if t == foreign));
    }

    // Two peers of the same session merge the same diverged pair from
    // opposite sides: each migrates its own side's indices, and both mint
    // the *identical* canonical merge commit.
    #[test]
    fn sync_remote_tip_merges_canonically_from_either_side() {
        // Peer 1: head on alpha (ours-canonical, older), remote = beta tip.
        let (mut reg_1, mut head_1) = diverged_registry(&[1, 2], &[1, 20], &[1, 2, 3]);
        let alpha_tip = reg_1.head_commit_ca(&head_1).unwrap();
        let beta_tip = reg_1.head(&"beta".parse().unwrap()).unwrap();
        let mut graph_1 = test_graph(&[1, 20]);
        let mut vm_1 = Engine::new_base();
        let mut view_1 = crate::SceneView::default();
        let mut selection_1 = Selection::default();
        let outcome_1 = run_sync(
            &mut reg_1,
            &mut head_1,
            &mut graph_1,
            &mut vm_1,
            &mut view_1,
            &mut selection_1,
            beta_tip,
        );
        let SyncTipOutcome::Merged {
            new_commit: commit_1,
            conflicts: 0,
            ..
        } = outcome_1
        else {
            panic!("expected clean Merged, got {outcome_1:?}");
        };
        let weights: Vec<u32> = graph_1.node_weights().map(value).collect();
        assert_eq!(weights, vec![1, 20, 3]);
        // Canonical orientation: alpha (older) is the first parent even
        // though it is also the local tip here.
        let commit = &reg_1.commits()[&commit_1];
        assert_eq!(commit.parent, Some(alpha_tip));
        assert_eq!(commit.merge_parents, vec![beta_tip]);

        // Peer 2: identical registry, but head on beta with alpha remote -
        // the local tip plays the theirs side after canonicalization.
        let (mut reg_2, _) = diverged_registry(&[1, 2], &[1, 20], &[1, 2, 3]);
        let mut head_2 = gantz_ca::Head::Branch("beta".parse().unwrap());
        let mut graph_2 = test_graph(&[1, 2, 3]);
        let mut vm_2 = Engine::new_base();
        vm_2.register_value(ROOT_STATE, SteelVal::empty_hashmap());
        node::state::update_value(&mut vm_2, &[2], SteelVal::IntV(7)).unwrap();
        let mut view_2 = crate::SceneView::default();
        view_2.layout.insert(node_id(2), egui::pos2(2.0, 0.0));
        let mut selection_2 = Selection::default();
        selection_2.nodes.insert(NodeIx::new(2));
        let outcome_2 = run_sync(
            &mut reg_2,
            &mut head_2,
            &mut graph_2,
            &mut vm_2,
            &mut view_2,
            &mut selection_2,
            alpha_tip,
        );
        let SyncTipOutcome::Merged {
            new_commit: commit_2,
            mapping,
            ..
        } = outcome_2
        else {
            panic!("expected Merged, got {outcome_2:?}");
        };
        // Identical merge commit and graph value on both peers.
        assert_eq!(commit_1, commit_2);
        let weights: Vec<u32> = graph_2.node_weights().map(value).collect();
        assert_eq!(weights, vec![1, 20, 3]);
        // Peer 2's local (theirs-side) indices happen to be preserved here;
        // its state/layout/selection followed the mapping.
        assert_eq!(mapping, gantz_ca::Matching::from([(0, 0), (1, 1), (2, 2)]));
        let state = node::state::extract_value(&vm_2, &[2]).unwrap();
        assert_eq!(state, Some(SteelVal::IntV(7)));
        assert_eq!(view_2.layout.get(&node_id(2)), Some(&egui::pos2(2.0, 0.0)));
        assert!(selection_2.nodes.contains(&NodeIx::new(2)));
    }

    // Twin commits (same graph, independent mints) adopt the deterministic
    // winner instead of merging; the loser side moves, the winner side is
    // already up to date.
    #[test]
    fn sync_remote_tip_adopts_newer_twin() {
        let secs = |s| std::time::Duration::from_secs(s);
        let mut reg = gantz_ca::Registry::default();
        let g = test_graph(&[1]);
        let base_ca = reg.commit_graph(secs(1), None, gantz_ca::graph_addr(&g), || g);
        let g = test_graph(&[1, 2]);
        let twin_a = reg.commit_graph(secs(2), Some(base_ca), gantz_ca::graph_addr(&g), || g);
        let g = test_graph(&[1, 2]);
        let twin_b = reg.commit_graph(secs(3), Some(base_ca), gantz_ca::graph_addr(&g), || g);
        reg.set_head("alpha".parse().unwrap(), twin_a);
        let mut head = gantz_ca::Head::Branch("alpha".parse().unwrap());
        let mut graph = test_graph(&[1, 2]);
        let mut vm = Engine::new_base();
        let mut view = crate::SceneView::default();
        let mut selection = Selection::default();

        let outcome = run_sync(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            twin_b,
        );
        let SyncTipOutcome::Moved(target) = outcome else {
            panic!("expected Moved, got {outcome:?}");
        };
        assert_eq!(target, twin_b, "the newer twin wins");
        // Navigation is the caller's job: nothing mutated yet.
        assert_eq!(reg.head(&"alpha".parse().unwrap()), Some(twin_a));

        // From the winner's side the same pair is already settled.
        reg.set_head("alpha".parse().unwrap(), twin_b);
        let outcome = run_sync(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            twin_a,
        );
        assert!(matches!(outcome, SyncTipOutcome::UpToDate));
    }

    #[test]
    fn sync_remote_tip_fast_forwards_and_reports_up_to_date() {
        let secs = |s| std::time::Duration::from_secs(s);
        let mut reg = gantz_ca::Registry::default();
        let g = test_graph(&[1]);
        let base_ca = reg.commit_graph(secs(1), None, gantz_ca::graph_addr(&g), || g);
        let g = test_graph(&[1, 2]);
        let child = reg.commit_graph(secs(2), Some(base_ca), gantz_ca::graph_addr(&g), || g);
        reg.set_head("alpha".parse().unwrap(), base_ca);
        let mut head = gantz_ca::Head::Branch("alpha".parse().unwrap());
        let mut graph = test_graph(&[1]);
        let mut vm = Engine::new_base();
        let mut view = crate::SceneView::default();
        let mut selection = Selection::default();

        let outcome = run_sync(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            child,
        );
        assert!(matches!(outcome, SyncTipOutcome::Moved(t) if t == child));

        reg.set_head("alpha".parse().unwrap(), child);
        let mut graph = test_graph(&[1, 2]);
        let outcome = run_sync(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            base_ca,
        );
        assert!(matches!(outcome, SyncTipOutcome::UpToDate));
    }

    #[test]
    fn sync_remote_tip_surfaces_unrelated() {
        let secs = |s| std::time::Duration::from_secs(s);
        let mut reg = gantz_ca::Registry::default();
        let g = test_graph(&[1]);
        let local = reg.commit_graph(secs(1), None, gantz_ca::graph_addr(&g), || g);
        let g = test_graph(&[9]);
        let foreign = reg.commit_graph(secs(2), None, gantz_ca::graph_addr(&g), || g);
        reg.set_head("alpha".parse().unwrap(), local);
        let mut head = gantz_ca::Head::Branch("alpha".parse().unwrap());
        let mut graph = test_graph(&[1]);
        let mut vm = Engine::new_base();
        let mut view = crate::SceneView::default();
        let mut selection = Selection::default();

        let outcome = run_sync(
            &mut reg,
            &mut head,
            &mut graph,
            &mut vm,
            &mut view,
            &mut selection,
            foreign,
        );
        assert!(matches!(outcome, SyncTipOutcome::Unrelated));
        assert_eq!(reg.head(&"alpha".parse().unwrap()), Some(local));
    }
}
