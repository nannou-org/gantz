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
use gantz_ca::CaHash;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::visit;
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
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.update!")]
pub struct UpdateBang;

impl gantz_format::NodeTag for UpdateBang {
    const TAG: &'static str = "UpdateBang";
}

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
    fn name(&self, _: &dyn gantz_egui::Registry) -> &str {
        "update!"
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
        _: &dyn gantz_egui::Registry,
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
// ToUpdateBang trait
// ---------------------------------------------------------------------------

/// Trait for types that may contain an [`UpdateBang`] node.
///
/// Implement this for your top-level node wrapper so that the
/// [`drive_update_bangs`] system can discover `update!` nodes.
pub trait ToUpdateBang {
    fn to_update_bang(&self) -> Option<&UpdateBang>;
}

impl ToUpdateBang for UpdateBang {
    fn to_update_bang(&self) -> Option<&UpdateBang> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// UpdateBangCollector
// ---------------------------------------------------------------------------

/// Collects paths to all [`UpdateBang`] nodes found during graph traversal.
struct UpdateBangCollector {
    pub paths: Vec<Vec<usize>>,
}

impl<N: ToUpdateBang> visit::TypedVisitor<N> for UpdateBangCollector {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &N) {
        if node.to_update_bang().is_some() {
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
pub fn entrypoints<N>(
    get_node: node::GetNode<'_>,
    graph: &gantz_core::node::graph::Graph<N>,
) -> Vec<gantz_core::compile::Entrypoint>
where
    N: gantz_core::Node + ToUpdateBang,
{
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
/// For each open head, traverses the working graph to find all `UpdateBang`
/// nodes, updates their state to the current update delta time, and triggers
/// a single push evaluation for all of them.
pub fn drive_update_bangs<N>(
    time: Res<Time>,
    registry: Res<crate::Registry<N>>,
    builtins: Res<bevy_gantz::BuiltinNodes<N>>,
    demos: Res<crate::Demos>,
    mut vms: NonSendMut<bevy_gantz::head::HeadVms>,
    heads: Query<(Entity, &bevy_gantz::head::WorkingGraph<N>), With<bevy_gantz::head::OpenHead>>,
    mut cmds: Commands,
) where
    N: gantz_core::Node + ToUpdateBang + Send + Sync,
{
    let dt = time.delta_secs_f64();

    for (entity, wg) in heads.iter() {
        let node_reg = crate::registry_ref(&registry, &builtins, &demos);
        let get_node = |ca: &gantz_ca::ContentAddr| node_reg.node(ca);

        // Collect all UpdateBang paths.
        let mut collector = UpdateBangCollector { paths: vec![] };
        gantz_core::graph::visit_typed(&get_node, &**wg, &[], &mut collector);

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
        });
    }
}
