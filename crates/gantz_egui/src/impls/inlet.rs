use crate::{NodeCtx, NodeUi, Registry, SetInterfaceDoc, SocketDocKind, widget::node_inspector};
use gantz_core::node;

impl NodeUi for gantz_core::node::graph::Inlet {
    fn name(&self, _: &dyn Registry) -> &str {
        "in"
    }

    fn ui(
        &mut self,
        ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| {
            let name = self.name(ctx.registry());
            let ix = inlet_ix(ctx.path(), ctx.inlets());
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
                let ix = inlet_ix(ctx.path(), ctx.inlets());
                ui.label(format!("{ix}"));
            });
        });
    }

    fn inspector_ui(&mut self, mut ctx: NodeCtx, ui: &mut egui::Ui) -> Option<egui::Response> {
        let ix = inlet_ix(ctx.path(), ctx.inlets());
        let current = ctx
            .interface_docs()
            .and_then(|d| d.inlets.get(&ix))
            .cloned();
        ui.separator();
        ui.label("docs");
        let edit = node_inspector::socket_doc_editor(ui, ctx.path(), current.as_ref());
        let (doc, resp) = edit?;
        ctx.response(SetInterfaceDoc {
            kind: SocketDocKind::Inlet,
            ix,
            doc,
        });
        Some(resp)
    }
}

/// Determine the inlet's index.
///
/// Inlets are ordered by their appearance within the graph indices.
fn inlet_ix(path: &[node::Id], inlets: &[node::Id]) -> usize {
    let id = path.last().expect("inlet must have non-inlet path");
    inlets
        .iter()
        .position(|in_id| id == in_id)
        .expect("inlet ID must exist")
}
