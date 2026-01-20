//! A Comment node for documenting patches.

use crate::widget::node_inspector;
use crate::{NodeCtx, NodeUi};
use gantz_core::node;
use serde::{Deserialize, Serialize};
use steel::parser::ast::ExprKind;
use steel::steel_vm::engine::Engine;

/// A transparent comment node for documenting graphs.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Comment {
    text: String,
    // TODO: Remove this in favour of using a resizable frame. This will involve
    // tweaks upstream in egui_graph to enable.
    width: u16,
    rows: u16,
}

impl Comment {
    /// An arbitrary size for the default comment dimensions.
    pub const DEFAULT_WIDTH: u16 = 150;
    pub const DEFAULT_ROWS: u16 = 4;

    /// Create a new Comment node with the given text.
    pub fn new(text: String) -> Self {
        Self {
            text,
            width: Self::DEFAULT_WIDTH,
            rows: Self::DEFAULT_ROWS,
        }
    }
}

impl Default for Comment {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl<Env> gantz_core::Node<Env> for Comment {
    // Comments have no inputs or outputs - they're purely for documentation
    fn n_inputs(&self, _: &Env) -> usize {
        0
    }

    fn n_outputs(&self, _: &Env) -> usize {
        0
    }

    // Comments don't evaluate to anything
    fn expr(&self, _ctx: node::ExprCtx<Env>) -> ExprKind {
        // Return void/empty expression since comments don't compute anything
        Engine::emit_ast("void")
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
    }
}

impl gantz_ca::CaHash for Comment {
    fn hash(&self, hasher: &mut gantz_ca::Hasher) {
        "gantz_egui::Comment".hash(hasher);
        self.text.hash(hasher);
    }
}

impl<Env> NodeUi<Env> for Comment {
    fn name(&self, _env: &Env) -> &str {
        "comment"
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx<Env>,
        uictx: egui_graph::NodeCtx,
    ) -> egui::InnerResponse<egui::Response> {
        // Get interaction state
        let interaction = uictx.interaction();
        let style = uictx.style();
        // Use the default margin as the stroke width, as this will be the only
        // draggable part of the node.
        let stroke_w = style.spacing.window_margin.top as f32;
        let stroke = if interaction.selected {
            egui::Stroke::new(stroke_w, style.visuals.selection.stroke.color)
        } else if interaction.in_selection_rect || interaction.hovered {
            egui::Stroke::new(stroke_w, style.visuals.weak_text_color())
        } else {
            egui::Stroke::new(stroke_w, egui::Color32::TRANSPARENT)
        };

        // Use a custom, transparent frame for comment nodes.
        let frame = egui::Frame::new()
            .fill(egui::Color32::TRANSPARENT)
            .corner_radius(style.visuals.window_corner_radius)
            .stroke(stroke);

        // Use a transparent frame for comment nodes
        let response = uictx.framed_with(frame, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.text)
                    .desired_width(self.width as f32)
                    .hint_text("Add comment...")
                    .frame(false)
                    .desired_rows(self.rows.into())
                    .min_size(egui::vec2(self.width as f32, 10.0)),
            )
        });

        response
    }

    fn inspector_rows(&mut self, _ctx: &NodeCtx<Env>, body: &mut egui_extras::TableBody) {
        dbg!(&self);
        let row_h = node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("width");
            });
            row.col(|ui| {
                ui.add(egui::DragValue::new(&mut self.width).range(10..=3_000));
            });
        });
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("rows");
            });
            row.col(|ui| {
                ui.add(egui::DragValue::new(&mut self.rows).range(1..=50));
            });
        });
    }
}
