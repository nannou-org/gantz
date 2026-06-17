use crate::{NodeCtx, NodeUi, Registry, SocketDoc, SocketKind, widget::node_inspector};
use gantz_core::node;

impl NodeUi for gantz_core::node::graph::Outlet {
    fn name(&self, _: &dyn Registry) -> &str {
        "out"
    }

    fn ui(
        &mut self,
        ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| {
            let name = self.name(ctx.registry());
            let ix = outlet_ix(ctx.path(), ctx.outlets());
            let text = format!("{}[{}]", name, ix);
            ui.add(egui::Label::new(text).selectable(false))
        })
    }

    fn inspector_rows(&mut self, ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("index");
            });
            row.col(|ui| {
                let ix = outlet_ix(ctx.path(), ctx.outlets());
                ui.label(format!("{ix}"));
            });
        });
    }

    fn inspector_ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> Option<egui::Response> {
        ui.separator();
        Some(node_inspector::socket_doc_editor(
            ui,
            ctx.path(),
            &mut self.ty,
            &mut self.description,
        ))
    }

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => Some(super::inlet::socket_doc(
                &self.ty,
                &self.description,
                "output",
            )),
            SocketKind::Output => None,
        }
    }
}

/// Determine the outlet's index.
///
/// Outlets are ordered by their appearance within the graph indices.
fn outlet_ix(path: &[node::Id], outlets: &[node::Id]) -> usize {
    let id = path.last().expect("outlet must have non-outlet path");
    outlets
        .iter()
        .position(|in_id| id == in_id)
        .expect("outlet ID must exist")
}
