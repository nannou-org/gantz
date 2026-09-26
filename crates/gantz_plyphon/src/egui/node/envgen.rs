//! `~envgen`'s egui implementation.

use crate::egui::param::{params_state_row, rate_row, value_row};
use crate::node::Envgen;
use crate::node::envgen::SOCKETS;
use crate::param::{param_value_keyed, params_state, with_param_value};
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};
use std::borrow::Cow;

impl NodeUi for Envgen {
    fn name(&self, _: &Env<'_>) -> Cow<'_, str> {
        Cow::Borrowed("~envgen")
    }

    fn description(&self) -> Option<&'static str> {
        Some("Envelope generator, started and released by a gate")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~envgen").selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn show_state(&self) -> bool {
        false
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        let state = ctx.extract_value().ok().flatten();
        params_state_row(body, state.as_ref());
        // The socket values live in VM state, so an edit is not structural.
        for socket in SOCKETS {
            let state = ctx.extract_value().ok().flatten();
            let mut value = state
                .as_ref()
                .and_then(|s| param_value_keyed(s, socket.name))
                .unwrap_or(socket.default as f64) as f32;
            let dv = egui::DragValue::new(&mut value).speed(0.01);
            if value_row(body, socket.name, dv) {
                let prev = state.unwrap_or_else(|| params_state(&[]));
                let _ = ctx.update_value(with_param_value(prev, socket.name, value as f64));
            }
        }
        let mut rate = self.rate();
        if rate_row(body, &mut rate) {
            self.set_rate(rate);
            resp.mark_changed();
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => {
                let socket = SOCKETS.get(ix)?;
                Some(SocketDoc::ty("signal | number").with_description(socket.doc))
            }
            SocketKind::Output => Some(SocketDoc::ty("signal").with_description("the envelope")),
        }
    }
}
