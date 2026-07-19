//! `~sinosc`'s egui implementation.

use crate::egui::param::{param_row, param_state_row, rate_row};
use crate::node::SinOsc;
use crate::param::{param_state, param_value, with_value};
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};

impl NodeUi for SinOsc {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "~sinosc".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Sine oscillator (audio or control rate)")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // The body shows just the node name; params are edited in the inspector.
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~sinosc").selectable(false)));
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
        // The value lives in VM state (a value edit must NOT change the content
        // address); the lag lives in the weight (a lag edit is structural).
        let state = ctx.extract_value().ok().flatten();
        param_state_row(body, state.as_ref());
        let mut value = state
            .as_ref()
            .and_then(param_value)
            .unwrap_or(Self::DEFAULT_FREQ as f64) as f32;
        let mut lag = self.freq_lag();
        let dv = egui::DragValue::new(&mut value)
            .range(0.0..=20_000.0)
            .speed(1.0)
            .suffix(" Hz");
        let (value_changed, lag_changed) = param_row(body, "freq", dv, &mut lag);
        if value_changed {
            // Preserve any queued `pending` updates; only the value changes.
            let prev = state.unwrap_or_else(|| param_state(Self::DEFAULT_FREQ as f64));
            let _ = ctx.update_value(with_value(prev, value as f64));
        }
        if lag_changed {
            self.set_freq_lag(lag);
            resp.mark_changed();
        }
        let mut rate = self.rate();
        if rate_row(body, &mut rate) {
            self.set_rate(rate);
            resp.mark_changed();
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        match (kind, ix) {
            (SocketKind::Input, 0) => Some(SocketDoc::ty("signal | number").with_description(
                "frequency (Hz): a connected signal drives it directly (audio-rate FM), \
                 a connected number overrides the inspector value",
            )),
            (SocketKind::Output, _) => Some(SocketDoc::ty("signal").with_description(
                "sine signal at the configured frequency (and the configured ar/kr rate)",
            )),
            _ => None,
        }
    }
}
