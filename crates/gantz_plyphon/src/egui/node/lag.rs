//! `~lag`'s egui implementation.

use crate::egui::param::{param_state_row, rate_row, value_row};
use crate::node::Lag;
use crate::param::{param_state, param_value, with_value};
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};

impl NodeUi for Lag {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "~lag".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("One-pole lag: smooth a signal over a duration")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~lag").selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn show_state(&self) -> bool {
        // A summarised "N queued" state row (in `inspector_rows`) replaces the raw
        // `{value, pending}` dump.
        false
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        // The lag duration lives in VM state (a value edit must NOT change the
        // content address), shown as a single duration row.
        let state = ctx.extract_value().ok().flatten();
        param_state_row(body, state.as_ref());
        let mut value = state
            .as_ref()
            .and_then(param_value)
            .unwrap_or(Self::DEFAULT_DUR as f64) as f32;
        let dv = egui::DragValue::new(&mut value)
            .range(0.0..=10.0)
            .speed(0.001)
            .fixed_decimals(3)
            .suffix(" s");
        if value_row(body, "lag", dv) {
            let prev = state.unwrap_or_else(|| param_state(Self::DEFAULT_DUR as f64));
            let _ = ctx.update_value(with_value(prev, value as f64));
        }
        let mut resp = InspectorRowsResponse::default();
        let mut rate = self.rate();
        if rate_row(body, &mut rate) {
            self.set_rate(rate);
            resp.mark_changed();
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => Some(SocketDoc::ty("signal").with_description("signal to smooth")),
            SocketKind::Output => Some(SocketDoc::ty("signal").with_description("smoothed signal")),
        }
    }
}
