//! A Comment node for documenting patches.

use crate::{NodeCtx, NodeUi};
use gantz_core::node;
use serde::{Deserialize, Serialize};
use steel::parser::ast::ExprKind;
use steel::steel_vm::engine::Engine;

/// A transparent comment node for documenting graphs.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct Comment {
    text: String,
}

impl Comment {
    /// Create a new Comment node with the given text.
    pub fn new(text: String) -> Self {
        Self { text }
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
        let stroke_color = if interaction.selected {
            style.visuals.selection.stroke.color
        } else if interaction.in_selection_rect || interaction.hovered {
            style.visuals.weak_text_color()
        } else {
            egui::Color32::TRANSPARENT
        };
        let stroke = egui::Stroke::new(stroke_w, stroke_color);

        // Use a custom, transparent frame for comment nodes.
        let frame = egui::Frame::new()
            .fill(egui::Color32::TRANSPARENT)
            .corner_radius(style.visuals.window_corner_radius)
            .stroke(stroke);

        // Use a transparent frame with resizable content
        let node_egui_id = uictx.egui_id();
        let resize_id = node_egui_id.with("resize");
        let min_resize = egui::Vec2::splat(style.interaction.interact_radius);
        let response = uictx.framed_with(frame, |ui| {
            egui::containers::Resize::default()
                .id(resize_id)
                .resizable(interaction.selected)
                .min_size(min_resize)
                .with_stroke(false)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.text)
                            .hint_text("Add comment...")
                            .frame(false)
                            .desired_width(f32::INFINITY)
                    )
                })
        });

        response
    }

}
