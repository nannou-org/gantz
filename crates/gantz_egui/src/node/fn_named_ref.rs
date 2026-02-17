//! `Fn<NamedRef>` type alias and NodeUi implementation.

use super::{NameRegistry, NamedRef, missing_color, outdated_color};
use crate::{NodeCtx, NodeUi, Registry, widget::node_inspector};

/// A function node wrapping a named reference.
pub type FnNamedRef = gantz_core::node::Fn<NamedRef>;

/// Trait for environments that can provide Fn-compatible node names.
pub trait FnNodeNames: NameRegistry {
    /// Names of nodes that can be used with Fn.
    /// Filters to: stateless, branchless, single-output nodes.
    fn fn_node_names(&self) -> Vec<String>;
}

impl NodeUi for FnNamedRef {
    fn name(&self, _registry: &dyn Registry) -> &str {
        "fn"
    }

    fn ui(
        &mut self,
        ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        let registry = ctx.registry();
        let ref_ca = self.0.content_addr();

        // Check if the referenced CA exists in registry.
        let is_missing = !registry.node_exists(&ref_ca);

        // Check if outdated (name points to different CA).
        let current_ca = registry.name_ca(self.0.name());
        let is_outdated = !is_missing && current_ca.map(|ca| ca != ref_ca).unwrap_or(false);

        // Auto-sync if enabled and outdated (skip if missing).
        if self.0.sync && is_outdated {
            if let Some(ca) = current_ca {
                self.0.set_ref(gantz_core::node::Ref::new(ca));
            }
        }

        // Recalculate after potential sync.
        let ref_ca = self.0.content_addr();
        let is_missing = !registry.node_exists(&ref_ca);
        let is_outdated = !is_missing
            && registry
                .name_ca(self.0.name())
                .map(|ca| ca != ref_ca)
                .unwrap_or(false);

        uictx.framed(|ui| {
            ui.horizontal(|ui| {
                let fn_res = ui.add(egui::Label::new("λ").selectable(false));
                let name_text = if is_missing {
                    egui::RichText::new(self.0.name()).color(missing_color())
                } else if is_outdated {
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

    fn inspector_rows(&mut self, ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());

        // ComboBox to select which node to reference.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("node");
            });
            row.col(|ui| {
                let registry = ctx.registry();
                let salt = format!("λ-node-select-{:?}", ctx.path());
                let names = registry.fn_node_names();
                egui::ComboBox::from_id_salt(salt)
                    .selected_text(self.0.name())
                    .show_ui(ui, |ui| {
                        for name in names.iter() {
                            if ui.selectable_label(self.0.name() == name, name).clicked() {
                                if let Some(ca) = registry.name_ca(name) {
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
