use crate::{NodeCtx, NodeUi};
use egui::scroll_area::ScrollAreaOutput;
use egui_extras::{Column, TableBuilder};
use gantz_core::node::{self, Node};

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
        let Self { node, ctx } = self;
        let scroll_area_output = table(node, &ctx, ui);
        let node_response = node.inspector_ui(ctx, ui);
        NodeInspectorResponse {
            scroll_area_output,
            node_response,
        }
    }
}

pub fn table(
    node: &(impl Node + NodeUi),
    ctx: &NodeCtx,
    ui: &mut egui::Ui,
) -> ScrollAreaOutput<()> {
    ui.strong(node.name());
    ui.add_space(ui.spacing().item_spacing.y);
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
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
                        node.n_inputs(),
                        node.n_outputs()
                    ));
                });
            });

            let eval = match (node.push_eval(), node.pull_eval()) {
                (Some(_), Some(_)) => Some("push, pull"),
                (Some(_), None) => Some("push"),
                (None, Some(_)) => Some("pull"),
                (None, None) => None,
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
        })
}

/// Format the node's path string.
pub fn path_string(path: &[node::Id]) -> String {
    path.iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}
