use crate::{NodeCtx, NodeUi, widget::node_inspector};
use gantz_core::node;

impl<Env> NodeUi<Env> for gantz_core::node::graph::Outlet {
    fn name(&self, _: &Env) -> &str {
        "out"
    }

    fn ui(&mut self, ctx: NodeCtx<Env>, ui: &mut egui::Ui) -> egui::Response {
        let name = self.name(ctx.env());
        let ix = outlet_ix(ctx.path(), ctx.outlets());
        let text = format!("{}[{}]", name, ix);
        ui.add(egui::Label::new(text).selectable(false))
    }

    fn inspector_rows(&mut self, ctx: &NodeCtx<Env>, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("index");
            });
            row.col(|ui| {
                let ix = outlet_ix(ctx.path(), ctx.inlets());
                ui.label(format!("{ix}"));
            });
        });
    }
}

/// Determine the outlet's index.
///
/// Outlets are ordered by their appearance within the graph indices.
fn outlet_ix(path: &[node::Id], outlets: &[node::Id]) -> usize {
    let id = path.last().expect("inlet must have non-inlet path");
    outlets
        .iter()
        .position(|in_id| id == in_id)
        .expect("inlet ID must exist")
}
