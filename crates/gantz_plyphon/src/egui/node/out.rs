//! `~out`'s egui implementation.

use crate::egui::param::{param_row, param_state_row};
use crate::node::Out;
use crate::param::{param_state, param_value, with_value};
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};

impl NodeUi for Out {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "~out".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Audio output: master gain to the speakers")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // The body shows just the node name; params are edited in the inspector.
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~out").selectable(false)));
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
        let mut resp = InspectorRowsResponse::default();
        let state = ctx.extract_value().ok().flatten();
        param_state_row(body, state.as_ref());
        let mut value = state
            .as_ref()
            .and_then(param_value)
            .unwrap_or(Self::DEFAULT_GAIN as f64) as f32;
        let mut lag = self.gain_lag();
        let dv = egui::DragValue::new(&mut value)
            .range(0.0..=1.0)
            .speed(0.005);
        let (value_changed, lag_changed) = param_row(body, "gain", dv, &mut lag);
        if value_changed {
            // Preserve any queued `pending` updates; only the value changes.
            let prev = state.unwrap_or_else(|| param_state(Self::DEFAULT_GAIN as f64));
            let _ = ctx.update_value(with_value(prev, value as f64));
        }
        if lag_changed {
            self.set_gain_lag(lag);
            resp.mark_changed();
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        match (kind, ix) {
            (SocketKind::Input, 0) => Some(SocketDoc::ty("signal").with_description(
                "signal to send to the audio output; mono fans across every \
                device channel, wider signals write channel i to bus i",
            )),
            (SocketKind::Input, 1) => Some(SocketDoc::ty("number").with_description(
                "master gain control; overrides the inspector value while connected",
            )),
            _ => None,
        }
    }
}
