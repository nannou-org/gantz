//! A Comment node for documenting patches.

use crate::widget::node_inspector;
use crate::{NodeCtx, NodeUi};
use gantz_ca::CaHash;
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx};
use serde::{Deserialize, Serialize};

/// Temporary editing state stored in egui memory to buffer text edits.
///
/// This prevents every keystroke from mutating the node's text (and thus
/// triggering a new content-addressed commit). The buffer is flushed to
/// the node on focus loss.
#[derive(Clone, Default)]
struct CommentEditState {
    text_hash: u64,
    text: String,
    last_edit_time: f64,
}

fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::default();
    Hash::hash(&text, &mut hasher);
    hasher.finish()
}

/// A transparent comment node for documenting graphs.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.comment")]
pub struct Comment {
    text: String,
    #[cahash(skip)]
    size: [u16; 2],
}

impl Comment {
    /// The default size if none is loaded from state.
    pub const DEFAULT_SIZE: [u16; 2] = [100, 40];

    /// Create a new Comment node with the given text.
    pub fn new(text: String) -> Self {
        let size = Self::DEFAULT_SIZE;
        Self { text, size }
    }
}

impl Default for Comment {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl gantz_core::Node for Comment {
    // Comments have no inputs or outputs - they're purely for documentation
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        0
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        0
    }

    // Comments don't evaluate to anything
    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        // Return void/empty expression since comments don't compute anything
        node::parse_expr("void")
    }
}

impl NodeUi for Comment {
    fn name(&self, _registry: &dyn crate::Registry) -> &str {
        "comment"
    }

    fn ui(
        &mut self,
        _ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
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
        let default_size = egui::vec2(self.size[0] as f32, self.size[1] as f32);
        let response = uictx.framed_with(frame, |ui, _sockets| {
            egui::containers::Resize::default()
                .id(resize_id)
                .resizable(interaction.selected)
                .default_size(default_size)
                .min_size(min_resize)
                .with_stroke(false)
                .show(ui, |ui| {
                    let size = ui.available_size();
                    self.size = [size.x as u16, size.y as u16];
                    let row_height = {
                        let font_id = egui::FontSelection::default().resolve(ui.style());
                        ui.fonts_mut(|f| f.row_height(&font_id))
                    };
                    egui::ScrollArea::vertical()
                        .min_scrolled_height(row_height)
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .auto_shrink(false)
                        .show(ui, |ui| {
                            let text_id = node_egui_id.with("comment_text");

                            // Load or initialize the editing state.
                            let mut state: CommentEditState = ui
                                .memory_mut(|m| m.data.remove_temp(text_id))
                                .unwrap_or_default();

                            // Sync from node if the node's text changed externally (undo, etc.).
                            let current_hash = text_hash(&self.text);
                            if current_hash != state.text_hash {
                                state.text_hash = current_hash;
                                state.text = self.text.clone();
                            }

                            // Render the TextEdit against the buffered string.
                            let response = ui.add(
                                egui::TextEdit::multiline(&mut state.text)
                                    .desired_rows(1)
                                    .hint_text("Add comment...")
                                    .frame(false)
                                    .desired_width(f32::INFINITY),
                            );

                            // Track when the buffer was last edited.
                            let time = ui.input(|i| i.time);
                            if response.changed() {
                                state.last_edit_time = time;
                            }

                            // Determine whether the buffer has uncommitted changes.
                            let buffer_dirty = text_hash(&state.text) != state.text_hash;

                            // Flush conditions:
                            // 1. Focus lost (existing)
                            // 2. 5+ seconds since last edit with dirty buffer
                            // 3. Any mouse activity with dirty buffer
                            let timed_out = buffer_dirty && (time - state.last_edit_time >= 5.0);
                            let mouse_active = buffer_dirty
                                && ui.input(|i| {
                                    i.pointer.is_moving()
                                        || i.pointer.any_pressed()
                                        || i.pointer.any_released()
                                });
                            let should_flush = response.lost_focus() || timed_out || mouse_active;

                            if should_flush {
                                self.text = state.text.clone();
                                state.text_hash = text_hash(&self.text);
                            }

                            // Schedule a repaint at the timeout for reactive mode.
                            if buffer_dirty && !should_flush {
                                let remaining = 10.0 - (time - state.last_edit_time);
                                if remaining > 0.0 {
                                    ui.ctx().request_repaint_after(
                                        std::time::Duration::from_secs_f64(remaining),
                                    );
                                }
                            }

                            // Persist the editing state.
                            ui.memory_mut(|m| m.data.insert_temp(text_id, state));

                            response
                        })
                        .inner
                })
        });

        response
    }

    fn inspector_rows(&mut self, _ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("size");
            });
            row.col(|ui| {
                ui.label(format!("{:?}", self.size));
            });
        });
    }
}
