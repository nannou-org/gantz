//! VM utilities for evaluating and navigating gantz graphs.
//!
//! This module provides:
//! - Evaluation events and observer (`EvalEntryEvent`, `on_eval_entry`)
//! - The compile-input memo ([`CompiledInputs`]) driving the UI layer's
//!   input-addressed VM synchronisation system (`bevy_gantz_egui::vm::sync`)

use crate::head;
use crate::reg::Registry;
use bevy_ecs::prelude::*;
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::{compile as core_compile, diagnostic};
use std::time::Duration;

/// Resource holding the [`core_compile::Config`] used whenever a head's graph
/// is (re)compiled into its VM.
///
/// Defaults to the core defaults. Override (and trigger a recompile) to e.g.
/// enable `emit_all_node_fns` when debugging codegen in the module view.
#[derive(Default, Resource)]
pub struct CompileConfig(pub core_compile::Config);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The inputs that determine a head's compiled module.
#[derive(Clone, Copy, PartialEq)]
pub struct Inputs {
    /// The content address of the head's working graph.
    pub graph: ca::GraphAddr,
    /// The codegen configuration.
    pub config: core_compile::Config,
}

/// The inputs of a head's last compile *attempt* (success or failure).
///
/// `None` = never attempted. The UI layer's `vm::sync` compares this against
/// the current inputs (the head's committed graph CA + config) to decide when
/// to (re)compile - there is no dirty flag to set or forget.
#[derive(Component, Default)]
pub struct CompiledInputs(pub Option<Inputs>);

/// When `true`, [`validate_committed`] hashes every open head's working graph
/// each frame and warns if it differs from the head's committed graph CA - i.e.
/// a system mutated the working graph without committing it, violating the
/// [`WorkingGraph`](head::WorkingGraph) commit-before-return invariant.
///
/// Defaults to `false` (no extra hashing); enable at runtime to debug a
/// suspected missing commit.
#[derive(Default, Resource)]
pub struct ValidateCommitted(pub bool);

/// Event to trigger evaluation of an entrypoint.
#[derive(Event)]
pub struct EvalEntryEvent {
    /// The head entity to evaluate on.
    pub head: Entity,
    /// The entrypoint to evaluate.
    pub entrypoint: core_compile::Entrypoint,
    /// The monotonic time (seconds, on the [`EvalEpoch`](crate::EvalEpoch))
    /// this evaluation logically fires at, exposed to nodes as `%args`'s `time`.
    ///
    /// `None` means "now" - resolved to [`EvalEpoch::now_secs`](crate::EvalEpoch)
    /// in [`on_eval_entry`]. A `tick!` passes `Some(t)` with each tick's exact
    /// firing time so timed control updates schedule sample-accurately; one-shot
    /// firings (`update!`, GUI pushes) leave it `None`.
    pub time: Option<f64>,
}

/// Emitted after VM evaluation completes, for timing capture.
///
/// This event allows UI layers (like `bevy_gantz_egui`) to observe VM execution
/// timing without the core crate depending on UI-related types.
#[derive(Event)]
pub struct EvalEntryComplete {
    /// The head entity that was evaluated.
    pub entity: Entity,
    /// The duration of the VM execution.
    pub duration: Duration,
}

// ---------------------------------------------------------------------------
// Core VM utilities
// ---------------------------------------------------------------------------

/// The node-identity mapping for navigating a head from the `from` commit to
/// the `to` commit: old node index -> new node index.
///
/// Prefers chain-tracked identity (see [`gantz_ca::diff::matching`]) in
/// whichever direction has a first-parent chain - `to` descending from
/// `from` (redo) or `from` descending from `to` (undo, inverted) - falling
/// back to direct content matching for divergent navigation (e.g. across
/// history-pane jumps). `None` only when an endpoint commit or graph is
/// missing from the registry.
pub fn navigation_matching(
    registry: &ca::Registry,
    from: ca::CommitAddr,
    to: ca::CommitAddr,
) -> Option<ca::Matching> {
    let commits = registry.commits();
    if ca::history::first_parent_chain_to(commits, to, from).is_some() {
        ca::diff::matching(registry, from, to)
    } else if ca::history::first_parent_chain_to(commits, from, to).is_some() {
        // The chain runs the other way: track identity along it and invert
        // (matchings are injective).
        let matching = ca::diff::matching(registry, to, from)?;
        Some(matching.into_iter().map(|(t, f)| (f, t)).collect())
    } else {
        let from_g = registry.commit_graph_ref(&from)?;
        let to_g = registry.commit_graph_ref(&to)?;
        Some(ca::diff::match_nodes(from_g, to_g))
    }
}

/// Migrate a navigating head's VM node state from the `from` commit's graph
/// to the `to` commit's, keeping the VM so that every node present on both
/// sides retains its state (`vm::sync` re-registers the new graph over the
/// kept VM, initialising only the nodes without state).
///
/// The VM is dropped - falling back to a fresh init - only when no mapping
/// can be derived or the state fails to remap.
pub fn migrate_vm_state(
    registry: &ca::Registry,
    vms: &mut head::HeadVms,
    entity: Entity,
    from: Option<ca::CommitAddr>,
    to: Option<ca::CommitAddr>,
) {
    let (Some(from), Some(to)) = (from, to) else {
        vms.remove(&entity);
        return;
    };
    let Some(vm) = vms.get_mut(&entity) else {
        return;
    };
    match navigation_matching(registry, from, to) {
        Some(mapping) => {
            if let Err(e) = gantz_core::node::state::remap_root(vm, &mapping) {
                log::error!("navigation: failed to remap node state: {e}; reinitialising");
                vms.remove(&entity);
            }
        }
        None => {
            vms.remove(&entity);
        }
    }
}

// ---------------------------------------------------------------------------
// Observers
// ---------------------------------------------------------------------------

/// Observer that handles evaluation events by calling the appropriate VM function.
///
/// Emits an `EvalEntryComplete` event with timing information for UI layers to observe.
pub fn on_eval_entry(
    trigger: On<EvalEntryEvent>,
    epoch: Res<crate::EvalEpoch>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<(&head::Module, &mut head::Diagnostics)>,
) {
    let event = trigger.event();
    let fn_name = core_compile::entry_fn_name(&event.entrypoint.id());
    if let Some(vm) = vms.get_mut(&event.head) {
        // Expose this firing's time to nodes via `%args` (e.g. DSP control inputs
        // stamp queued values with it). `None` means "now".
        let time = event.time.unwrap_or_else(|| epoch.now_secs());
        vm.update_value(gantz_core::ARGS, gantz_core::args::time(time));
        let start = web_time::Instant::now();
        let result = vm.call_function_by_name_with_args(&fn_name, vec![]);
        // Runtime diagnostics reflect the latest evaluation only.
        if let Ok((module, mut diagnostics)) = heads.get_mut(event.head) {
            diagnostics
                .0
                .retain(|d| d.severity != diagnostic::Severity::Runtime);
            if let (Err(e), Some(compiled)) = (&result, &module.compiled) {
                diagnostics
                    .0
                    .push(diagnostic::from_eval_error(e, vm, compiled));
            }
        }
        if let Err(e) = result {
            log::error!("{e}");
        }
        cmds.trigger(EvalEntryComplete {
            entity: event.head,
            duration: start.elapsed(),
        });
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Commit a head's working graph to the registry when it has diverged from the
/// head's current commit, updating the head and emitting a
/// [`head::CommittedEvent`]. Returns `true` if a new commit was made.
///
/// **Call this from any system that mutates a head's
/// [`WorkingGraph`](head::WorkingGraph), before the system returns** - it is how
/// the commit-before-return invariant (see `WorkingGraph`) is upheld, which in
/// turn lets `vm::sync` recompile from the committed address without re-hashing.
/// This is the single place a working graph is content-addressed.
pub fn commit_working_graph(
    registry: &mut Registry,
    cmds: &mut Commands,
    entity: Entity,
    head: &mut ca::Head,
    graph: &ca::DataGraph,
) -> bool {
    // The working graph IS the stored form, so its registry address is
    // computed directly - no erase step.
    let graph_ca = ca::graph_addr(graph);
    let Some(head_commit) = registry.head_commit(head) else {
        return false;
    };
    if head_commit.graph == graph_ca {
        return false;
    }
    let old_head = head.clone();
    let dg = graph.clone();
    let new_commit_ca =
        registry.commit_graph_to_head(crate::reg::timestamp(), graph_ca, || dg, head);
    log::debug!("Graph changed -> {}", new_commit_ca.display_short());
    cmds.trigger(head::CommittedEvent {
        entity,
        old_head,
        new_head: head.clone(),
    });
    true
}

/// Debug check for the [`WorkingGraph`](head::WorkingGraph) commit-before-return
/// invariant.
///
/// When [`ValidateCommitted`] is enabled, hash every open head's working
/// graph and warn if it differs from the head's committed graph CA - i.e. a
/// system mutated the working graph without committing it. Every weight is
/// also checked for canonicality (address computation assumes it). A no-op
/// (no hashing) when disabled, which is the default.
pub fn validate_committed(
    validate: Res<ValidateCommitted>,
    registry: Res<Registry>,
    heads: Query<head::OpenHeadDataReadOnly, With<head::OpenHead>>,
) {
    if !validate.0 {
        return;
    }
    for data in heads.iter() {
        for (ix, weight) in data.working_graph.0.node_weights().enumerate() {
            if !weight.is_canonical() {
                log::warn!(
                    "WorkingGraph invariant check: head {:?} node {ix} ({}) is not \
                     canonical - its address (and thus the commit comparison) is \
                     unreliable",
                    data.entity,
                    weight.tag,
                );
            }
        }
        let working = ca::graph_addr(&data.working_graph.0);
        let committed = registry.head_commit(&data.head_ref.0).map(|c| c.graph);
        if committed != Some(working) {
            log::warn!(
                "WorkingGraph invariant violated: head {:?} working graph ({}) does \
                 not match its committed graph ({:?}) - a system mutated it without \
                 committing (see `commit_working_graph`)",
                data.entity,
                working.display_short(),
                committed.map(|c| c.display_short().to_string()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_ca::{DataGraph, Datum, NodeData};
    use std::time::Duration;

    /// A minimal erased node distinguished by its payload value.
    fn nd(v: i64) -> NodeData {
        let mut n = NodeData {
            tag: "Num".to_string(),
            data: Datum::Map(vec![("v".to_string(), Datum::I64(v))]),
            refs: vec![],
            blobs: vec![],
        };
        n.canonicalize();
        n
    }

    fn graph(vals: &[i64]) -> DataGraph {
        let mut g = DataGraph::default();
        for &v in vals {
            g.add_node(nd(v));
        }
        g
    }

    /// A minimal registry: base `[10, 20, 30]`, then a child commit deleting
    /// index 1 (swap-removal: `[10, 30]`).
    fn base_and_child() -> (ca::Registry, ca::CommitAddr, ca::CommitAddr) {
        let mut reg = ca::Registry::default();
        let g = graph(&[10, 20, 30]);
        let base_ca = ca::graph_addr(&g);
        let base = reg.commit_graph(Duration::from_secs(1), None, base_ca, || g);
        let g = graph(&[10, 30]);
        let child_ca = ca::graph_addr(&g);
        let child = reg.commit_graph(Duration::from_secs(2), Some(base), child_ca, || g);
        (reg, base, child)
    }

    #[test]
    fn navigation_matching_tracks_redo_along_the_chain() {
        let (reg, base, child) = base_and_child();
        // Redo direction: base -> child. Index 2 swap-moved to 1; 1 deleted.
        let m = navigation_matching(&reg, base, child).unwrap();
        assert_eq!(m, ca::Matching::from([(0, 0), (2, 1)]));
    }

    #[test]
    fn navigation_matching_inverts_for_undo() {
        let (reg, base, child) = base_and_child();
        // Undo direction: child -> base. The chain runs the other way, so
        // the tracked matching is inverted: child index 1 returns to 2.
        let m = navigation_matching(&reg, child, base).unwrap();
        assert_eq!(m, ca::Matching::from([(0, 0), (1, 2)]));
    }

    #[test]
    fn navigation_matching_falls_back_to_content_for_divergent_commits() {
        let mut reg = ca::Registry::default();
        let g = graph(&[7]);
        let a_ca = ca::graph_addr(&g);
        let a = reg.commit_graph(Duration::from_secs(1), None, a_ca, || g);
        let g = graph(&[9, 7]);
        let b_ca = ca::graph_addr(&g);
        let b = reg.commit_graph(Duration::from_secs(2), None, b_ca, || g);
        // Unrelated commits: direct content matching pairs the equal node.
        let m = navigation_matching(&reg, a, b).unwrap();
        assert_eq!(m, ca::Matching::from([(0, 1)]));
    }
}
