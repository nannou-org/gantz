//! A node that triggers push evaluation every update, outputting delta time.
//!
//! All `UpdateBang` nodes in a graph are combined into a single multi-source
//! entrypoint via [`entrypoints()`]. Evaluation is driven by the
//! [`drive_update_bangs`] Bevy system rather than from the node's `ui()` method,
//! so it continues even when the graph tab is not visible.
//!
//! Note this bangs once per *update*, not once per rendered frame. Under
//! presentation modes like Mailbox, updates can occur more frequently than
//! frames are presented.

use bevy_ecs::prelude::*;
use bevy_egui::egui;
use bevy_time::prelude::*;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::visit;
use gantz_egui::node::DynNode;
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};
use steel::SteelVal;

// ---------------------------------------------------------------------------
// UpdateBang node
// ---------------------------------------------------------------------------

/// A node that drives continuous evaluation every update.
///
/// Outputs the update's delta time in seconds as `f64`. This fires once per
/// *update*, which may be more frequent than rendered frames under presentation
/// modes like Mailbox.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
pub struct UpdateBang;

impl gantz_core::Node for UpdateBang {
    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        1
    }

    fn stateful(&self, _ctx: MetaCtx) -> bool {
        true
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        node::parse_expr("(begin state)")
    }

    fn register(&self, mut ctx: RegCtx<'_, '_>) {
        let path = ctx.path();
        node::state::init_value_if_absent(ctx.vm(), path, || SteelVal::NumV(0.0)).unwrap()
    }
}

impl gantz_egui::NodeUi for UpdateBang {
    fn name(&self, _: &gantz_egui::Env<'_>) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("update!")
    }

    fn description(&self) -> Option<&'static str> {
        Some(
            "Drives continuous evaluation, banging once per update with the update \
             delta time in seconds. Updates can fire more frequently than rendered \
             frames under presentation modes like Mailbox.",
        )
    }

    fn ui(
        &mut self,
        _ctx: gantz_egui::NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> gantz_egui::NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("update!").selectable(false)));
        gantz_egui::NodeUiResponse::new(framed)
    }

    fn socket_doc(
        &self,
        _: &gantz_egui::Env<'_>,
        kind: gantz_egui::SocketKind,
        _ix: usize,
    ) -> Option<gantz_egui::SocketDoc> {
        match kind {
            gantz_egui::SocketKind::Output => Some(
                gantz_egui::SocketDoc::ty("number")
                    .with_description("update delta time in seconds; emitted every update"),
            ),
            gantz_egui::SocketKind::Input => None,
        }
    }
}

// ---------------------------------------------------------------------------
// UpdateBangCollector
// ---------------------------------------------------------------------------

/// Collects paths to all [`UpdateBang`] nodes found during graph traversal,
/// discovered by [`Any`](std::any::Any) downcast within the erased UI node.
struct UpdateBangCollector {
    pub paths: Vec<Vec<usize>>,
}

impl visit::TypedVisitor<DynNode> for UpdateBangCollector {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &DynNode) {
        let n: &dyn gantz_core::Node = &**node;
        if (n as &dyn std::any::Any)
            .downcast_ref::<UpdateBang>()
            .is_some()
        {
            self.paths.push(ctx.path().to_vec());
        }
    }
}

// ---------------------------------------------------------------------------
// Entrypoints
// ---------------------------------------------------------------------------

/// Collect all `UpdateBang` nodes in the graph and return a single multi-source
/// entrypoint covering all of them.
///
/// Returns an empty vec if no `UpdateBang` nodes are found.
pub fn entrypoints(
    get_node: node::GetNode<'_>,
    graph: &gantz_core::node::graph::Graph<DynNode>,
) -> Vec<gantz_core::compile::Entrypoint> {
    let mut collector = UpdateBangCollector { paths: vec![] };
    gantz_core::graph::visit_typed(get_node, graph, &[], &mut collector);
    if collector.paths.is_empty() {
        return vec![];
    }
    let sources = collector
        .paths
        .into_iter()
        .map(|path| gantz_core::compile::entrypoint::push_source(path, 1));
    vec![gantz_core::compile::entrypoint::from_sources(sources)]
}

// ---------------------------------------------------------------------------
// Bevy system
// ---------------------------------------------------------------------------

/// Drives `update!` nodes every update, independent of GUI visibility.
///
/// For each open head, traverses the head's committed graph - read from the
/// reified cache, which the working graph equals by the `WorkingGraph`
/// invariant - to find all `UpdateBang` nodes, updates their state to the
/// current update delta time, and triggers a single push evaluation for all
/// of them.
pub fn drive_update_bangs(
    time: Res<Time>,
    registry: Res<crate::Registry>,
    cache: Res<crate::GraphCache>,
    builtins: Res<crate::BuiltinNodes>,
    mut vms: NonSendMut<bevy_gantz::head::HeadVms>,
    heads: Query<(Entity, &bevy_gantz::head::HeadRef), With<bevy_gantz::head::OpenHead>>,
    mut cmds: Commands,
) {
    let dt = time.delta_secs_f64();

    for (entity, head_ref) in heads.iter() {
        let Some(graph_ca) = registry.head_commit(&head_ref.0).map(|c| c.graph) else {
            continue;
        };
        let Some(graph) = cache.get(&graph_ca) else {
            continue;
        };
        let get_node =
            |ca: &gantz_ca::ContentAddr| crate::lookup_node(&cache, &builtins.instances, ca);

        // Collect all UpdateBang paths.
        let mut collector = UpdateBangCollector { paths: vec![] };
        gantz_core::graph::visit_typed(&get_node, graph, &[], &mut collector);

        if collector.paths.is_empty() {
            continue;
        }

        // Update state for each UpdateBang.
        let Some(vm) = vms.get_mut(&entity) else {
            continue;
        };
        for path in &collector.paths {
            if let Err(e) = node::state::update_value(vm, path, SteelVal::NumV(dt)) {
                bevy_log::error!("update! state update failed: {e}");
            }
        }

        // Trigger a single eval for all UpdateBangs combined.
        let sources = collector
            .paths
            .into_iter()
            .map(|path| gantz_core::compile::entrypoint::push_source(path, 1));
        let entrypoint = gantz_core::compile::entrypoint::from_sources(sources);
        cmds.trigger(bevy_gantz::vm::EvalEntryEvent {
            head: entity,
            entrypoint,
            // An update represents "now".
            time: None,
        });
    }
}
