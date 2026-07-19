//! `~sum`'s egui implementation.

use crate::egui::param::value_row;
use crate::node::Sum;
use gantz_egui::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
};

impl NodeUi for Sum {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "~sum".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Sum signals into their unity-gain mix (mixed, not packed)")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("~sum").selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn inspector_rows(
        &mut self,
        _ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        // Input count (structural: it changes the node's sockets -> respawn).
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
            SocketKind::Input => Some(SocketDoc::ty("signal").with_description(format!(
                "signal {ix} to sum (any channel width; silence when unconnected)"
            ))),
            SocketKind::Output => Some(SocketDoc::ty("signal").with_description(
                "the channel-wise sum (width = the widest input; mono inputs broadcast)",
            )),
        }
    }
}
