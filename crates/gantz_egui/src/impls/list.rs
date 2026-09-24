use crate::{Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind};
use gantz_std::List;

impl NodeUi for List {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "list".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Collect the connected inputs into a list")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let framed =
            uictx.framed(|ui, _sockets| ui.add(egui::Label::new("list").selectable(false)));
        NodeUiResponse::new(framed)
    }

    fn inspector_rows(
        &mut self,
        _ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        let row_h = crate::widget::node_inspector::table_row_h(body.ui_mut());
        // The input count is structural. It changes the node's sockets.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("count");
            });
            row.col(|ui| {
                let mut n = self.count();
                let dv = egui::DragValue::new(&mut n)
                    .range(1..=List::MAX_COUNT)
                    .speed(0.1);
                if ui.add(dv).changed() {
                    self.set_count(n);
                    resp.mark_changed();
                }
            });
        });
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => Some(
                SocketDoc::ty("any")
                    .with_description(format!("item {ix}. Skipped when unconnected")),
            ),
            SocketKind::Output => Some(
                SocketDoc::ty("list")
                    .with_description("the connected inputs as a list, in socket order"),
            ),
        }
    }
}
