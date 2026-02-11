use crate::{NodeCtx, NodeUi};
use egui::scroll_area::ScrollAreaOutput;
use egui_extras::{Column, TableBuilder};
use gantz_core::node::{self, MetaCtx, Node};

/// A widget for presenting more detailed information and control for a node.
pub struct NodeInspector<'a, N> {
    node: &'a mut N,
    ctx: NodeCtx<'a>,
}

/// The response returned from [`NodeInspector::show`].
pub struct NodeInspectorResponse {
    pub scroll_area_output: ScrollAreaOutput<()>,
    pub node_response: Option<egui::Response>,
}

impl<'a, N> NodeInspector<'a, N>
where
    N: Node + NodeUi,
{
    pub fn new(node: &'a mut N, ctx: NodeCtx<'a>) -> Self {
        Self { node, ctx }
    }

    pub fn show(self, ui: &mut egui::Ui) -> NodeInspectorResponse {
        let Self { node, mut ctx } = self;
        let scroll_area_output = table(node, &mut ctx, ui);
        let node_response = node.inspector_ui(ctx, ui);
        NodeInspectorResponse {
            scroll_area_output,
            node_response,
        }
    }
}

pub fn table_row_h(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y
}

pub fn table(
    node: &mut (impl Node + NodeUi),
    ctx: &mut NodeCtx,
    ui: &mut egui::Ui,
) -> ScrollAreaOutput<()> {
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

    ui.strong(name);
    ui.add_space(ui.spacing().item_spacing.y);
    let row_h = table_row_h(ui);
    TableBuilder::new(ui)
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

            node.inspector_rows(ctx, &mut body);
        })
}

/// Format the node's path string.
pub fn path_string(path: &[node::Id]) -> String {
    path.iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}
