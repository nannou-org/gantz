//! A node that triggers push evaluation every frame, outputting delta time.

use crate::widget::node_inspector;
use crate::{NodeCtx, NodeUi};
use gantz_ca::CaHash;
use gantz_core::node::{self, EvalConf, ExprCtx, ExprResult, MetaCtx, RegCtx};
use serde::{Deserialize, Serialize};
use steel::SteelVal;

/// A node that drives continuous evaluation by triggering `push_eval` every
/// frame. Outputs the frame's delta time in seconds as `f64`.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.frame!")]
pub struct FrameBang {
    /// When true, requests continuous repainting so evaluation occurs every
    /// frame. When false, evaluation only happens on frames triggered by
    /// other causes (interaction, etc).
    #[serde(default = "default_continuous")]
    #[cahash(skip)]
    pub continuous: bool,
}

impl Default for FrameBang {
    fn default() -> Self {
        Self { continuous: true }
    }
}

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

impl NodeUi for FrameBang {
    fn name(&self, _: &dyn crate::Registry) -> &str {
        "frame!"
    }

    fn ui(
        &mut self,
        mut ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        let continuous = self.continuous;
        uictx.framed(|ui, _sockets| {
            let dt = ui.ctx().input(|i| i.stable_dt);
            ctx.update_value(SteelVal::NumV(dt as f64)).unwrap();
            ctx.push_eval();
            if continuous {
                ui.ctx().request_repaint();
            }
            ui.add(egui::Label::new("frame!").selectable(false))
        })
    }

    fn inspector_rows(&mut self, _ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("continuous");
            });
            row.col(|ui| {
                ui.checkbox(&mut self.continuous, "");
            });
        });
    }
}

fn default_continuous() -> bool {
    true
}
