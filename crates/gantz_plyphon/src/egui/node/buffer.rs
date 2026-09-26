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
        Some("A zeroed scratch buffer for units that write and read a buffer")
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
        // The shape is structural. A change gives a new zeroed buffer.
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
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => None,
            SocketKind::Output => {
                Some(SocketDoc::ty("buffer").with_description("the buffer, for a buffer socket"))
            }
        }
    }
}
