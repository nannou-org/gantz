//! A node that triggers push evaluation every frame, outputting delta time.
//!
//! Evaluation is driven by the [`drive_frame_bangs`] Bevy system rather than
//! from the node's `ui()` method, so it continues even when the graph tab is
//! not visible.

use bevy_ecs::prelude::*;
use bevy_egui::egui;
use bevy_time::prelude::*;
use gantz_ca::CaHash;
use gantz_core::node::{self, EvalConf, ExprCtx, ExprResult, MetaCtx, RegCtx};
use gantz_core::visit;
use serde::{Deserialize, Serialize};
use steel::SteelVal;

// ---------------------------------------------------------------------------
// FrameBang node
// ---------------------------------------------------------------------------

/// A node that drives continuous evaluation by triggering `push_eval` every
/// frame. Outputs the frame's delta time in seconds as `f64`.
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

    fn push_eval(&self, _ctx: MetaCtx) -> Vec<EvalConf> {
        vec![EvalConf::All]
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
// Bevy system
// ---------------------------------------------------------------------------

/// Drives `frame!` nodes every frame, independent of GUI visibility.
///
/// For each open head, traverses the working graph to find all `FrameBang`
/// nodes, updates their state to the current frame delta time, and triggers
/// push evaluation.
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

        // Update state and trigger eval for each FrameBang path.
        let Some(vm) = vms.get_mut(&entity) else {
            continue;
        };

        for path in &collector.paths {
            if let Err(e) = node::state::update_value(vm, path, SteelVal::NumV(dt)) {
                bevy_log::error!("frame! state update failed: {e}");
            }
        }

        for path in collector.paths {
            let entrypoint = gantz_core::compile::entrypoint::push(path, 1);
            cmds.trigger(bevy_gantz::vm::EvalEvent {
                head: entity,
                entrypoint,
            });
        }
    }
}
