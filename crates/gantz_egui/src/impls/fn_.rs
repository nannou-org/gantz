use crate::{NodeCtx, NodeUi, widget::node_inspector};

impl<Env> NodeUi<Env> for gantz_core::node::Fn {
    fn name(&self, _: &Env) -> &str {
        "fn"
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx<Env>,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        uictx.framed(|ui| {
            ui.vertical(|ui| {
                ui.add(egui::Label::new("fn").selectable(false));
                // Show the wrapped node name
                ui.add(egui::Label::new(
                    egui::RichText::new(&self.name).small()
                ).selectable(false));
            });
            ui.response()
        })
    }

    fn inspector_rows(&mut self, _ctx: &NodeCtx<Env>, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());

        // Row for node selection
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("Node");
            });
            row.col(|ui| {
                // TODO: Add combo box to select from available nodes
                // For now just show the current name
                ui.label(&self.name);
            });
        });

        // Row for content address (if it's a registry node)
        if let Some(graph_addr) = self.graph {
            body.row(row_h, |mut row| {
                row.col(|ui| {
                    ui.label("CA");
                });
                row.col(|ui| {
                    let ca_string = format!("{}", graph_addr.display_short());
                    ui.add(egui::Label::new(egui::RichText::new(ca_string).monospace()));
                });
            });
        }
    }
}