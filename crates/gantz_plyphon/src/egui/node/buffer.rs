//! `~buffer`'s egui implementation.

use crate::egui::param::value_row;
use crate::node::Buffer;
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};
use std::borrow::Cow;

impl NodeUi for Buffer {
    fn name(&self, _: &Env<'_>) -> Cow<'_, str> {
        Cow::Borrowed("~buffer")
    }

    fn description(&self) -> Option<&'static str> {
        Some("A scratch buffer for units that write and read a buffer, or a table from a list")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~buffer").selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn inspector_rows(
        &mut self,
        _ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        // The shape and the table format are structural. A change gives a new
        // buffer.
        let mut resp = InspectorRowsResponse::default();
        let mut frames = self.frames();
        let frames_dv = egui::DragValue::new(&mut frames)
            .range(1..=Buffer::MAX_FRAMES)
            .speed(64.0)
            .suffix(" frames");
        if value_row(body, "frames", frames_dv) {
            self.set_frames(frames);
            resp.mark_changed();
        }
        let mut channels = self.channels();
        let channels_dv = egui::DragValue::new(&mut channels)
            .range(1..=Buffer::MAX_CHANNELS)
            .speed(0.05);
        if value_row(body, "channels", channels_dv) {
            self.set_channels(channels);
            resp.mark_changed();
        }
        let mut wavetable = self.wavetable();
        let row_h = gantz_egui::widget::node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("wavetable");
            });
            row.col(|ui| {
                let hover = "convert the table to the wavetable format of `~osc` and `~cosc`";
                if ui
                    .checkbox(&mut wavetable, "")
                    .on_hover_text(hover)
                    .changed()
                {
                    self.set_wavetable(wavetable);
                    resp.mark_changed();
                }
            });
        });
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => Some(
                SocketDoc::ty("list")
                    .with_description("a table of numbers to write into the buffer"),
            ),
            SocketKind::Output => {
                Some(SocketDoc::ty("buffer").with_description("the buffer, for a buffer socket"))
            }
        }
    }
}
