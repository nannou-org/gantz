//! `~unpack`'s egui implementation.

use crate::egui::param::value_row;
use crate::node::Unpack;
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};

impl NodeUi for Unpack {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "~unpack".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Split a multichannel signal into mono outputs")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~unpack").selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn inspector_rows(
        &mut self,
        _ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        // Output count (structural: it changes the node's sockets -> respawn).
        let mut count = self.count();
        let dv = egui::DragValue::new(&mut count).range(1..=64).speed(1.0);
        if value_row(body, "count", dv) {
            self.set_count(count);
            resp.mark_changed();
        }
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => Some(
                SocketDoc::ty("signal")
                    .with_description("signal to split into mono channels (any channel width)"),
            ),
            SocketKind::Output => Some(SocketDoc::ty("signal").with_description(format!(
                "channel {ix} of the input (silence past its width)"
            ))),
        }
    }
}
