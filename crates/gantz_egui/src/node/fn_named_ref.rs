//! `Fn<NamedRef>` type alias and NodeUi implementation.

use super::{NamedRef, missing_color, outdated_color};
use crate::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind,
    widget::node_inspector,
};

/// A function node wrapping a named reference.
pub type FnNamedRef = gantz_core::node::Fn<NamedRef>;

// Declared on the wrapped type: the orphan rule forbids implementing
// `NodeTag` for the foreign `Fn<NamedRef>` here directly.
impl gantz_core::node::FnNodeTag for NamedRef {
    const FN_TAG: &'static str = "FnNamedRef";
}

impl NodeUi for FnNamedRef {
    fn name(&self, _registry: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "fn".into()
    }

    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let registry = ctx.env();
        let name_str = self.0.name().to_string();
        let ref_ca = self.0.content_addr();

        // Check if the referenced CA exists in registry.
        let is_missing = !registry.node_exists(&ref_ca);

        // Check if outdated (name points to different CA).
        let current_ca = registry.name_ca(&name_str);
        let is_outdated = !is_missing && current_ca.map(|ca| ca != ref_ca).unwrap_or(false);

        // Auto-sync if enabled and outdated (skip if missing). A silent
        // mutation that changes the node's CA.
        let mut changed = false;
        if self.0.sync && is_outdated {
            if let Some(ca) = current_ca {
                let synced = self.0.ref_().retarget(ca);
                self.0.set_ref(synced);
                changed = true;
            }
        }

        // Recalculate after potential sync.
        let ref_ca = self.0.content_addr();
        let is_missing = !registry.node_exists(&ref_ca);
        let is_outdated = !is_missing
            && registry
                .name_ca(&name_str)
                .map(|ca| ca != ref_ca)
                .unwrap_or(false);

        let framed = uictx.framed(|ui, _sockets| {
            ui.horizontal(|ui| {
                let fn_res = ui.add(egui::Label::new("λ").selectable(false));
                let name_text = if is_missing {
                    egui::RichText::new(&name_str).color(missing_color())
                } else if is_outdated {
                    egui::RichText::new(&name_str).color(outdated_color())
                } else {
                    egui::RichText::new(&name_str)
                };
                let name_res = ui.add(egui::Label::new(name_text).selectable(false));
                fn_res.union(name_res)
            })
            .inner
        });
        let mut resp = NodeUiResponse::new(framed);
        resp.set_changed(changed);
        resp
    }

    fn inspector_rows(
        &mut self,
        ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        let row_h = node_inspector::table_row_h(body.ui_mut());

        // ComboBox to select which node to reference.
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("node");
            });
            row.col(|ui| {
                let registry = ctx.env();
                let salt = format!("λ-node-select-{:?}", ctx.path());
                let names = registry.fn_node_names();
                let current = self.0.name().to_string();
                egui::ComboBox::from_id_salt(salt)
                    .selected_text(&current)
                    .show_ui(ui, |ui| {
                        for name in names.iter() {
                            if ui.selectable_label(current == *name, name).clicked() {
                                if let Some(ca) = registry.name_ca(name) {
                                    let name = name.parse().expect("infallible");
                                    self.0 = NamedRef::new(name, gantz_core::node::Ref::new(ca));
                                    resp.mark_changed();
                                }
                            }
                        }
                    });
            });
        });

        // Delegate to NamedRef's inspector rows for CA and update button.
        let inner = self.0.inspector_rows(ctx, body);
        resp.set_changed(inner.changed);
        resp.payloads.extend(inner.payloads);
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        Some(match kind {
            SocketKind::Input => SocketDoc::ty("bang")
                .with_description("trigger to emit the named graph as a lambda"),
            SocketKind::Output => {
                SocketDoc::ty("function").with_description("lambda wrapping the named graph")
            }
        })
    }
}
