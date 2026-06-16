use crate::{NodeCtx, NodeUi};
use egui::scroll_area::ScrollAreaOutput;
use egui_extras::{Column, TableBuilder};
use gantz_core::node::{self, MetaCtx, Node};

/// A widget for presenting more detailed information and control for a node.
pub struct NodeInspector<'a, N> {
    node: &'a mut N,
    ctx: NodeCtx<'a>,
    immutable: bool,
}

/// The response returned from [`NodeInspector::show`].
pub struct NodeInspectorResponse {
    pub scroll_area_output: ScrollAreaOutput<()>,
    pub node_response: Option<egui::Response>,
    pub label_response: egui::Response,
}

impl<'a, N> NodeInspector<'a, N>
where
    N: Node + NodeUi,
{
    pub fn new(node: &'a mut N, ctx: NodeCtx<'a>, immutable: bool) -> Self {
        Self {
            node,
            ctx,
            immutable,
        }
    }

    pub fn show(self, ui: &mut egui::Ui) -> NodeInspectorResponse {
        let Self {
            node,
            mut ctx,
            immutable,
        } = self;
        let (scroll_area_output, label_response) = table(node, &mut ctx, immutable, ui);
        if immutable {
            ui.disable();
        }
        let node_response = node.inspector_ui(ctx, ui);
        NodeInspectorResponse {
            scroll_area_output,
            node_response,
            label_response,
        }
    }
}

pub fn table_row_h(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y
}

pub fn table(
    node: &mut (impl Node + NodeUi),
    ctx: &mut NodeCtx,
    immutable: bool,
    ui: &mut egui::Ui,
) -> (ScrollAreaOutput<()>, egui::Response) {
    // Extract info we need upfront before the closure borrows ctx.
    let registry = ctx.registry();
    let get_node = |ca: &gantz_ca::ContentAddr| registry.node(ca);
    let meta_ctx = MetaCtx::new(&get_node);

    // Compute all node metadata before the table closure.
    let name = node.name(registry);
    let path = ctx.path().to_vec();
    let n_inputs = node.n_inputs(meta_ctx);
    let n_outputs = node.n_outputs(meta_ctx);
    let push_eval = !node.push_eval(meta_ctx).is_empty();
    let pull_eval = !node.pull_eval(meta_ctx).is_empty();
    let is_stateful = node.stateful(meta_ctx);
    let state_value = if is_stateful {
        Some(ctx.extract_value())
    } else {
        None
    };

    let label_response = ui.add(
        egui::Label::new(egui::RichText::new(name).strong())
            .selectable(false)
            .sense(egui::Sense::click()),
    );
    ui.add_space(ui.spacing().item_spacing.y);
    let row_h = table_row_h(ui);
    let scroll_area_output = TableBuilder::new(ui)
        .vscroll(false)
        .column(Column::auto().at_least(50.0).resizable(true))
        .column(Column::remainder().at_least(120.0))
        .body(|mut body| {
            body.row(row_h, |mut row| {
                row.col(|ui| {
                    ui.label("path");
                });
                row.col(|ui| {
                    ui.monospace(path_string(&path));
                });
            });

            body.row(row_h, |mut row| {
                row.col(|ui| {
                    ui.label("i/o");
                });
                row.col(|ui| {
                    ui.label(format!("{} inputs, {} outputs", n_inputs, n_outputs));
                });
            });

            let eval = match (push_eval, pull_eval) {
                (true, true) => Some("push, pull"),
                (true, false) => Some("push"),
                (false, true) => Some("pull"),
                (false, false) => None,
            };

            if let Some(eval) = eval {
                body.row(row_h, |mut row| {
                    row.col(|ui| {
                        ui.label("eval");
                    });
                    row.col(|ui| {
                        ui.label(eval);
                    });
                });
            }

            if let Some(ref state_result) = state_value {
                body.row(row_h, |mut row| {
                    row.col(|ui| {
                        ui.label("state");
                    });
                    row.col(|ui| match state_result {
                        Ok(Some(state)) => {
                            ui.label(format!("{state:#?}"));
                        }
                        Ok(None) => {
                            ui.weak("None");
                        }
                        Err(_) => {
                            ui.weak("Error");
                        }
                    });
                });
            }

            if immutable {
                body.ui_mut().disable();
            }
            node.inspector_rows(ctx, &mut body);
        });
    (scroll_area_output, label_response)
}

/// Format the node's path string.
pub fn path_string(path: &[node::Id]) -> String {
    path.iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A small editor for an inlet/outlet's [`SocketDoc`](crate::SocketDoc) (a type
/// label and an optional description).
///
/// The fields are seeded from `current` each frame; on edit, returns the new
/// doc (`None` when both fields are blank, i.e. cleared) along with the
/// triggering response. `id_salt` scopes the text edit state to the node.
pub(crate) fn socket_doc_editor(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    current: Option<&crate::SocketDoc>,
) -> Option<(Option<crate::SocketDoc>, egui::Response)> {
    let id = egui::Id::new("socket-doc-editor").with(&id_salt);
    let mut ty = current.map(|d| d.ty.to_string()).unwrap_or_default();
    let mut desc = current
        .and_then(|d| d.description.as_deref())
        .unwrap_or_default()
        .to_string();
    let ty_resp = ui.add(
        egui::TextEdit::singleline(&mut ty)
            .id(id.with("ty"))
            .hint_text("type")
            .desired_width(f32::INFINITY),
    );
    let desc_resp = ui.add(
        egui::TextEdit::multiline(&mut desc)
            .id(id.with("desc"))
            .hint_text("description")
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    let changed = ty_resp.changed() || desc_resp.changed();
    let resp = ty_resp.union(desc_resp);
    if !changed {
        return None;
    }
    let ty = ty.trim();
    let desc = desc.trim();
    let doc = if ty.is_empty() && desc.is_empty() {
        None
    } else {
        let mut d = crate::SocketDoc::ty(ty.to_string());
        if !desc.is_empty() {
            d = d.with_description(desc.to_string());
        }
        Some(d)
    };
    Some((doc, resp))
}
