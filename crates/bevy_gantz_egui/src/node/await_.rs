//! The `await` node: receive a gantz task, swallow evaluation, and fire a
//! push evaluation with the task's result once it resolves.
//!
//! On receiving a [`TaskHandle`] the node stashes it in its state and selects
//! a dead branch arm, so evaluation stops at the node. The [`drive_awaits`]
//! Bevy system polls the stashed task in place each update, and on completion
//! writes the result pair into the node's state and triggers the node's push
//! entrypoint - the value output fires on success, the error output on
//! failure. A non-task input value passes straight through the value output
//! in the same evaluation.
//!
//! Pending tasks never leave the node's VM state, so the state-maintenance
//! machinery carries in-flight work everywhere the node goes: editor deletes
//! reindex it via `remove_value`/`move_value`, and head navigation, merges and
//! collab sync migrate it via `remap_root` (dropping - and thereby cancelling
//! - the tasks of deleted nodes). The one corner where a pending task is
//! deliberately dropped is nesting, which removes rather than relocates the
//! nested nodes' state.

use bevy_ecs::prelude::*;
use bevy_egui::egui;
use bevy_gantz::task::{TASK_CANCEL_FN, TASK_PREDICATE, TaskHandle};
use gantz_core::node::{self, Conns, EvalConf, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::visit;
use gantz_egui::node::DynNode;
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use steel::SteelVal;
use steel::rvals::FromSteelVal;

/// The branch arm firing the value output on task success.
const VALUE_ARM: isize = 0;

/// The branch arm firing the error output on task failure.
const ERROR_ARM: isize = 1;

/// The dead branch arm selected while a task is pending: no output fires.
const PENDING_ARM: isize = 2;

// ---------------------------------------------------------------------------
// Await node
// ---------------------------------------------------------------------------

/// A node that awaits a gantz task received on its input.
///
/// Receiving a task swallows the evaluation (nothing fires downstream) until
/// the task resolves, at which point [`drive_awaits`] fires the node's push
/// entrypoint: output 0 carries the resolved value, output 1 the error string
/// if the task failed. Any non-task input value passes straight through
/// output 0 in the same evaluation.
///
/// The node's state is always a `(list arm payload)` branch pair: the pending
/// arm holding the stashed task (or nothing), or the value/error arm written
/// by the driver just before it fires the entrypoint.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, NodeTag)]
pub struct Await;

impl gantz_core::Node for Await {
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        // Output 0 = the resolved value; output 1 = the error string.
        2
    }

    fn branches(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        // Arm 0 fires the value output, arm 1 the error output, arm 2 neither
        // (a task was received or is still pending - evaluation stops here).
        vec![
            EvalConf::Set(Conns::try_from([true, false]).unwrap()),
            EvalConf::Set(Conns::try_from([false, true]).unwrap()),
            EvalConf::Set(Conns::try_from([false, false]).unwrap()),
        ]
    }

    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        // The entry fn through which `drive_awaits` delivers task results.
        vec![EvalConf::All]
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn expr(&self, ctx: ExprCtx<'_, '_>) -> ExprResult {
        let expr = match ctx.inputs().first() {
            // A value arrived: stash tasks in state for the driver and select
            // the dead arm; pass anything else straight through the value
            // output. Any pending predecessor is cancelled explicitly (latest
            // wins) - relying on the replaced pair being dropped would be
            // best-effort timing, since steel heap-boxes `set!` state.
            Some(Some(input)) => format!(
                "(if ({TASK_PREDICATE} {input}) \
                     (begin \
                         ({TASK_CANCEL_FN} state) \
                         (set! state (list {PENDING_ARM} {input})) \
                         (list {PENDING_ARM} '())) \
                     (list {VALUE_ARM} {input}))"
            ),
            // Entered via the push entry fn: the driver wrote the result pair
            // into state, which is exactly the branch pair to return.
            _ => "(begin state)".to_string(),
        };
        node::parse_expr(&expr)
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        bevy_gantz::task::register_task_type(ctx.vm());
        node::state::init_value_if_absent(ctx.vm(), path, dead_pair).unwrap()
    }
}

impl gantz_egui::NodeUi for Await {
    fn name(&self, _: &gantz_egui::Env<'_>) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("await")
    }

    fn description(&self) -> Option<&'static str> {
        Some(
            "Awaits a gantz task (e.g. produced by `sleep`), swallowing \
             evaluation until the task resolves, then fires its result through \
             the value output - or the error message through the error output \
             if the task failed. A non-task input value passes straight \
             through the value output. A task arriving while another is \
             pending replaces it, cancelling the earlier task.",
        )
    }

    fn ui(
        &mut self,
        _ctx: gantz_egui::NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> gantz_egui::NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("await").selectable(false)));
        gantz_egui::NodeUiResponse::new(framed)
    }

    fn socket_doc(
        &self,
        _: &gantz_egui::Env<'_>,
        kind: gantz_egui::SocketKind,
        ix: usize,
    ) -> Option<gantz_egui::SocketDoc> {
        match (kind, ix) {
            (gantz_egui::SocketKind::Input, 0) => Some(
                gantz_egui::SocketDoc::ty("task | any")
                    .with_description("a gantz task to await, or any value to pass through"),
            ),
            (gantz_egui::SocketKind::Output, 0) => Some(
                gantz_egui::SocketDoc::ty("any")
                    .with_description("the resolved value, once the task completes"),
            ),
            (gantz_egui::SocketKind::Output, 1) => Some(
                gantz_egui::SocketDoc::ty("string")
                    .with_description("the error message, if the task failed"),
            ),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// State pairs
// ---------------------------------------------------------------------------

/// The state pair while no result is pending delivery: the dead arm with an
/// empty payload.
pub fn dead_pair() -> SteelVal {
    pair(PENDING_ARM, SteelVal::ListV(Default::default()))
}

/// The state pair delivering a resolved value through the value output.
pub fn value_pair(val: SteelVal) -> SteelVal {
    pair(VALUE_ARM, val)
}

/// The state pair delivering an error message through the error output.
pub fn error_pair(err: String) -> SteelVal {
    pair(ERROR_ARM, SteelVal::StringV(err.into()))
}

/// A `(list arm payload)` branch pair.
fn pair(arm: isize, payload: SteelVal) -> SteelVal {
    SteelVal::ListV([SteelVal::IntV(arm), payload].into_iter().collect())
}

/// The stashed task's handle, if the given state value is a pending pair
/// holding one.
///
/// The returned handle shares the cell of the one in state, so checking it
/// polls the task in place. A dead pair's `'()` payload simply fails the
/// handle conversion.
pub fn pending_handle(state: &SteelVal) -> Option<TaskHandle> {
    let SteelVal::ListV(list) = state else {
        return None;
    };
    let mut items = list.iter();
    let (Some(arm), Some(payload)) = (items.next(), items.next()) else {
        return None;
    };
    if *arm != SteelVal::IntV(PENDING_ARM) {
        return None;
    }
    TaskHandle::from_steelval(payload).ok()
}

// ---------------------------------------------------------------------------
// AwaitCollector
// ---------------------------------------------------------------------------

/// Collects the path of every [`Await`] node found during graph traversal,
/// discovered by [`Any`](std::any::Any) downcast within the erased UI node.
struct AwaitCollector {
    pub paths: Vec<Vec<usize>>,
}

impl visit::TypedVisitor<DynNode> for AwaitCollector {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &DynNode) {
        let n: &dyn gantz_core::Node = &**node;
        if (n as &dyn std::any::Any).downcast_ref::<Await>().is_some() {
            self.paths.push(ctx.path().to_vec());
        }
    }
}

// ---------------------------------------------------------------------------
// Bevy system
// ---------------------------------------------------------------------------

/// Drives `await` nodes every update, independent of GUI visibility.
///
/// For each open head and each `await` node whose state is a pending pair,
/// checks the stashed task in place. On completion it writes the result pair
/// into the node's state - overwriting the pair releases the handle - and
/// triggers the node's push entrypoint.
///
/// Pending tasks live entirely in node state, so nothing here needs pruning
/// or remapping: deleting a node, closing a head, or navigating to a graph
/// without the node drops the state (cancelling the task), while reindexing
/// edits and head navigation migrate it with the node.
pub fn drive_awaits(
    registry: Res<crate::Registry>,
    cache: Res<crate::GraphCache>,
    builtins: Res<crate::BuiltinNodes>,
    mut vms: NonSendMut<bevy_gantz::head::HeadVms>,
    heads: Query<(Entity, &bevy_gantz::head::HeadRef), With<bevy_gantz::head::OpenHead>>,
    mut cmds: Commands,
) {
    for (entity, head_ref) in heads.iter() {
        // The head's committed graph, read from the reified cache (the
        // working graph equals it by the `WorkingGraph` invariant).
        let Some(graph_ca) = registry.head_commit(&head_ref.0).map(|c| c.graph) else {
            continue;
        };
        let Some(graph) = cache.get(&graph_ca) else {
            continue;
        };
        let get_node =
            |ca: &gantz_ca::ContentAddr| crate::lookup_node(&cache, &builtins.instances, ca);

        let mut collector = AwaitCollector { paths: vec![] };
        gantz_core::graph::visit_typed(&get_node, graph, &[], &mut collector);

        if collector.paths.is_empty() {
            continue;
        }

        let Some(vm) = vms.get_mut(&entity) else {
            continue;
        };

        for path in collector.paths {
            let state = match node::state::extract_value(vm, &path) {
                Ok(Some(state)) => state,
                Ok(None) => continue,
                Err(e) => {
                    bevy_log::error!("await state read failed: {e}");
                    continue;
                }
            };
            let Some(handle) = pending_handle(&state) else {
                continue;
            };
            let Some(result) = handle.check() else {
                continue;
            };
            let pair = match result {
                Ok(val) => value_pair(val),
                Err(err) => error_pair(err),
            };
            if let Err(e) = node::state::update_value(vm, &path, pair) {
                bevy_log::error!("await result write failed: {e}");
                continue;
            }
            let n_outputs = 2;
            let entrypoint = gantz_core::compile::entrypoint::push(path, n_outputs);
            // Guard against delivering through an entry fn the current module
            // no longer compiles (e.g. the task resolved in the same update
            // as a graph edit).
            let fn_name = gantz_core::compile::entry_fn_name(&entrypoint.id());
            if vm.extract_value(&fn_name).is_err() {
                bevy_log::debug!("await eval skipped: entry fn {fn_name} not compiled");
                continue;
            }
            cmds.trigger(bevy_gantz::vm::EvalEntryEvent {
                head: entity,
                entrypoint,
                time: None,
            });
        }
    }
}
