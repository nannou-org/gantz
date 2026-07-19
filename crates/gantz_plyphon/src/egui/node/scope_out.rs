//! `~scopeout`'s egui implementation.

use crate::egui::param::value_row;
use crate::node::ScopeOut;
use gantz_core::steel::SteelVal;
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};

impl NodeUi for ScopeOut {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "~scopeout".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Scope a signal into per-channel ring buffers; read them out on a trigger")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // The body shows just the node name; the buffered samples surface via the
        // outlet (into a `plot`), and the config is edited in the inspector.
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~scopeout").selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn show_state(&self) -> bool {
        // A summarised "frames × channels" state row (in `inspector_rows`) replaces the
        // raw dump of the per-channel sample rings.
        false
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();

        // State summary: how many rings the driver has written (the tapped
        // signal's width), and how many frames each holds.
        let (frames, channels) = ctx
            .extract_value()
            .ok()
            .flatten()
            .map_or((0, 0), |v| match v {
                SteelVal::ListV(rings) => {
                    let frames = rings.iter().next().map_or(0, |r| match r {
                        SteelVal::ListV(ring) => ring.len(),
                        _ => 0,
                    });
                    (frames, rings.len())
                }
                _ => (0, 0),
            });
        let row_h = gantz_egui::widget::node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("state");
            });
            row.col(|ui| {
                ui.label(format!("{frames} frames × {channels} channels"))
                    .on_hover_text("buffered dsp samples (one ring per channel)");
            });
        });

        // Ring length in frames (non-structural: not in the def; the driver caps
        // each per-channel ring at `size` frames).
        let mut size = self.size();
        let size_dv = egui::DragValue::new(&mut size)
            .range(1..=16_384)
            .speed(1.0)
            .suffix(" frames");
        if value_row(body, "size", size_dv) {
            self.set_size(size);
            resp.mark_changed();
        }

        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        match (kind, ix) {
            (SocketKind::Input, 0) => Some(SocketDoc::ty("signal").with_description(
                "signal to sample into per-channel ring buffers (any channel width)",
            )),
            (SocketKind::Input, 1) => {
                Some(SocketDoc::ty("bang").with_description("trigger: output the buffered samples"))
            }
            (SocketKind::Output, 0) => Some(
                SocketDoc::ty("list")
                    .with_description("one list of buffered samples per channel (oldest first)"),
            ),
            (SocketKind::Output, 1) => Some(
                SocketDoc::ty("number")
                    .with_description("the channel count (0 until the audio driver writes)"),
            ),
            _ => None,
        }
    }
}
