//! A node that triggers push evaluation every frame, outputting delta time.
//!
//! All `FrameBang` nodes in a graph are combined into a single multi-source
//! entrypoint via [`entrypoints()`]. Evaluation is driven by the
//! [`drive_frame_bangs`] Bevy system rather than from the node's `ui()` method,
//! so it continues even when the graph tab is not visible.

use bevy_ecs::prelude::*;
use bevy_egui::egui;
use bevy_time::prelude::*;
use gantz_ca::CaHash;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::visit;
use serde::{Deserialize, Serialize};
use steel::SteelVal;

// ---------------------------------------------------------------------------
// FrameBang node
// ---------------------------------------------------------------------------

/// A node that drives continuous evaluation every frame.
/// Outputs the frame's delta time in seconds as `f64`.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.frame!")]
pub struct FrameBang;

impl gantz_core::Node for FrameBang {
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

impl gantz_egui::NodeUi for FrameBang {
    fn name(&self, _: &dyn gantz_egui::Registry) -> &str {
        "frame!"
    }

    fn ui(
        &mut self,
        _ctx: gantz_egui::NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| ui.add(egui::Label::new("frame!").selectable(false)))
    }
}

// ---------------------------------------------------------------------------
// ToFrameBang trait
// ---------------------------------------------------------------------------

/// Trait for types that may contain a [`FrameBang`] node.
///
/// Implement this for your top-level node wrapper so that the
/// [`drive_frame_bangs`] system can discover `frame!` nodes.
pub trait ToFrameBang {
    fn to_frame_bang(&self) -> Option<&FrameBang>;
}

impl ToFrameBang for FrameBang {
    fn to_frame_bang(&self) -> Option<&FrameBang> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// FrameBangCollector
// ---------------------------------------------------------------------------

/// Collects paths to all [`FrameBang`] nodes found during graph traversal.
struct FrameBangCollector {
    pub paths: Vec<Vec<usize>>,
}

impl<N: ToFrameBang> visit::TypedVisitor<N> for FrameBangCollector {
    fn visit_pre(&mut self, ctx: visit::Ctx<'_, '_>, node: &N) {
        if node.to_frame_bang().is_some() {
            self.paths.push(ctx.path().to_vec());
        }
    }
}

// ---------------------------------------------------------------------------
// Entrypoints
// ---------------------------------------------------------------------------

/// Collect all `FrameBang` nodes in the graph and return a single multi-source
/// entrypoint covering all of them.
///
/// Returns an empty vec if no `FrameBang` nodes are found.
pub fn entrypoints<N>(
    get_node: node::GetNode<'_>,
    graph: &gantz_core::node::graph::Graph<N>,
) -> Vec<gantz_core::compile::Entrypoint>
where
    N: gantz_core::Node + ToFrameBang,
{
    let mut collector = FrameBangCollector { paths: vec![] };
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

/// Drives `frame!` nodes every frame, independent of GUI visibility.
///
/// For each open head, traverses the working graph to find all `FrameBang`
/// nodes, updates their state to the current frame delta time, and triggers
/// a single push evaluation for all of them.
pub fn drive_frame_bangs<N>(
    time: Res<Time>,
    registry: Res<crate::Registry<N>>,
    builtins: Res<bevy_gantz::BuiltinNodes<N>>,
    demos: Res<crate::Demos>,
    mut vms: NonSendMut<bevy_gantz::head::HeadVms>,
    heads: Query<(Entity, &bevy_gantz::head::WorkingGraph<N>), With<bevy_gantz::head::OpenHead>>,
    mut cmds: Commands,
) where
    N: gantz_core::Node + ToFrameBang + Send + Sync,
{
    let dt = time.delta_secs_f64();

    for (entity, wg) in heads.iter() {
        let node_reg = crate::registry_ref(&registry, &builtins, &demos);
        let get_node = |ca: &gantz_ca::ContentAddr| node_reg.node(ca);

        // Collect all FrameBang paths.
        let mut collector = FrameBangCollector { paths: vec![] };
        gantz_core::graph::visit_typed(&get_node, &**wg, &[], &mut collector);

        if collector.paths.is_empty() {
            continue;
        }

        // Update state for each FrameBang.
        let Some(vm) = vms.get_mut(&entity) else {
            continue;
        };
        for path in &collector.paths {
            if let Err(e) = node::state::update_value(vm, path, SteelVal::NumV(dt)) {
                bevy_log::error!("frame! state update failed: {e}");
            }
        }

        // Trigger a single eval for all FrameBangs combined.
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
