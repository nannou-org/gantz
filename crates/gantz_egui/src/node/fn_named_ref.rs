//! `Fn<NamedRef>` type alias and NodeUi implementation.

use super::{NameRegistry, NamedRef, outdated_color};
use crate::{NodeCtx, NodeUi, widget::node_inspector};

/// A function node wrapping a named reference.
pub type FnNamedRef = gantz_core::node::Fn<NamedRef>;

/// Trait for environments that can provide Fn-compatible node names.
pub trait FnNodeNames: NameRegistry {
    /// Names of nodes that can be used with Fn.
    /// Filters to: stateless, branchless, single-output nodes.
    fn fn_node_names(&self) -> Vec<String>;
}

impl<Env> NodeUi<Env> for FnNamedRef
where
    Env: FnNodeNames,
{
    fn name(&self, _env: &Env) -> &str {
        "fn"
    }

    fn ui(
        &mut self,
        ctx: NodeCtx<Env>,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        let env = ctx.env();
        let current_ca = env.name_ca(self.0.name());
        let is_outdated = current_ca
            .map(|ca| ca != self.0.content_addr())
            .unwrap_or(false);

        // Auto-sync if enabled and outdated.
        if self.0.sync && is_outdated {
            if let Some(ca) = current_ca {
                self.0.set_ref(gantz_core::node::Ref::new(ca));
            }
        }

        // Recalculate after potential sync.
        let is_outdated = env
            .name_ca(self.0.name())
            .map(|ca| ca != self.0.content_addr())
            .unwrap_or(false);

        uictx.framed(|ui| {
            ui.horizontal(|ui| {
                let fn_res = ui.add(egui::Label::new("λ").selectable(false));
                let name_text = if is_outdated {
                    egui::RichText::new(self.0.name()).color(outdated_color())
                } else {
                    egui::RichText::new(self.0.name())
                };
                let name_res = ui.add(egui::Label::new(name_text).selectable(false));
                fn_res.union(name_res)
            })
            .inner
        })
    }

    fn inspector_rows(&mut self, ctx: &mut NodeCtx<Env>, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());

        // ComboBox to select which node to reference.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("node");
            });
            row.col(|ui| {
                let env = ctx.env();
                let salt = format!("λ-node-select-{:?}", ctx.path());
                let names = env.fn_node_names();
                egui::ComboBox::from_id_salt(salt)
                    .selected_text(self.0.name())
                    .show_ui(ui, |ui| {
                        for name in names.iter() {
                            if ui.selectable_label(self.0.name() == name, name).clicked() {
                                if let Some(ca) = env.name_ca(name) {
                                    self.0 =
                                        NamedRef::new(name.clone(), gantz_core::node::Ref::new(ca));
                                }
                            }
                        }
                    });
            });
        });

        // Delegate to NamedRef's inspector rows for CA and update button.
        self.0.inspector_rows(ctx, body);
    }
}
