use crate::{NodeCtx, NodeUi, Registry, SocketDoc, SocketKind, widget::node_inspector};
use gantz_core::node;

impl NodeUi for gantz_core::node::graph::Inlet {
    fn name(&self, _: &dyn Registry) -> &str {
        "in"
    }

    fn description(&self) -> Option<&'static str> {
        Some("Marks an input to the enclosing graph.")
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
            SocketKind::Output => Some(socket_doc(&self.ty, &self.description, "input")),
            SocketKind::Input => None,
        }
    }
}

/// Build a [`SocketDoc`] from a marker's stored fields, defaulting the type
/// label when unset.
pub(crate) fn socket_doc(ty: &str, description: &str, default_ty: &'static str) -> SocketDoc {
    let mut doc = if ty.is_empty() {
        SocketDoc::ty(default_ty)
    } else {
        SocketDoc::ty(ty.to_string())
    };
    if !description.is_empty() {
        doc = doc.with_description(description.to_string());
    }
    doc
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
