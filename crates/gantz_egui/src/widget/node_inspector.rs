use crate::{NodeCtx, NodeUi};
use egui::scroll_area::ScrollAreaOutput;
use egui_extras::{Column, TableBuilder};
use gantz_core::node::{self, Node};

/// A widget for presenting more detailed information and control for a node.
pub struct NodeInspector<'a, Env, N> {
    node: &'a mut N,
    ctx: NodeCtx<'a, Env>,
}

/// The response returned from [`NodeInspector::show`].
pub struct NodeInspectorResponse {
    pub scroll_area_output: ScrollAreaOutput<()>,
    pub node_response: Option<egui::Response>,
}

impl<'a, Env, N> NodeInspector<'a, Env, N>
where
    N: Node<Env> + NodeUi<Env>,
{
    pub fn new(node: &'a mut N, ctx: NodeCtx<'a, Env>) -> Self {
        Self { node, ctx }
    }

    pub fn show(self, ui: &mut egui::Ui) -> NodeInspectorResponse {
        let Self { node, ctx } = self;
        let scroll_area_output = table(node, &ctx, ui);
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

pub fn table<Env>(
    node: &mut (impl Node<Env> + NodeUi<Env>),
    ctx: &NodeCtx<Env>,
    ui: &mut egui::Ui,
) -> ScrollAreaOutput<()> {
    ui.strong(node.name(ctx.env()));
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
                    ui.monospace(path_string(ctx.path()));
                });
            });

            body.row(row_h, |mut row| {
                row.col(|ui| {
                    ui.label("i/o");
                });
                row.col(|ui| {
                    ui.label(format!(
                        "{} inputs, {} outputs",
                        node.n_inputs(ctx.env()),
                        node.n_outputs(ctx.env())
                    ));
                });
            });

            let eval = match (
                !node.push_eval(ctx.env()).is_empty(),
                !node.pull_eval(ctx.env()).is_empty(),
            ) {
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

            if node.stateful() {
                body.row(row_h, |mut row| {
                    row.col(|ui| {
                        ui.label("state");
                    });
                    row.col(|ui| match ctx.extract_value() {
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
