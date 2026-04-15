use crate::{NodeCtx, NodeUi, Registry, widget::node_inspector};

impl NodeUi for gantz_core::node::Apply {
    fn name(&self, _: &dyn Registry) -> &str {
        "apply"
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| ui.add(egui::Label::new("apply").selectable(false)))
    }

    fn inspector_rows(&mut self, ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("arg inputs");
            });
            row.col(|ui| {
                let current = self.fixed_arg_count();
                let selected_text = current
                    .map(|arg_count| arg_count.to_string())
                    .unwrap_or_else(|| "list".to_string());
                let salt = format!("apply-arg-inputs-{:?}", ctx.path());
                egui::ComboBox::from_id_salt(salt)
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current.is_none(), "list").clicked() {
                            if current.is_some() {
                                *self = gantz_core::node::Apply::list();
                            }
                        }

                        for arg_count in 1..=gantz_core::node::Apply::MAX_FIXED_ARGS {
                            if ui
                                .selectable_label(current == Some(arg_count), arg_count.to_string())
                                .clicked()
                            {
                                if current != Some(arg_count) {
                                    *self = gantz_core::node::Apply::fixed(arg_count)
                                        .expect("apply inspector only exposes valid arg counts");
                                }
                            }
                        }
                    });
            });
        });
    }
}
