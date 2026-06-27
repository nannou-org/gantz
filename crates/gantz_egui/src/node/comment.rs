//! A Comment node for documenting patches.

use crate::widget::node_inspector;
use crate::{InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse};
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
///
/// Both `text` and `size` are part of the content address: editing the note or
/// resizing it are genuine edits that produce a new commit (and ride the export
/// pipeline), so a resize is undoable just like a text edit.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, CaHash)]
#[cahash("gantz.comment")]
pub struct Comment {
    text: String,
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

    fn description(&self) -> Option<&'static str> {
        Some("A free-floating text note")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // Set when a CA-affecting edit settles this frame: a flushed text change
        // or a settled resize (see the writes inside the closure below).
        let mut changed = false;
        // Get interaction state
        let interaction = uictx.interaction();
        let style = uictx.style();
        // Match the regular node selection outline: a thin stroke at the node's
        // edge. The large draggable band lives in the (invisible) inner margin
        // below, so the border stays subtle while the node remains easy to grab.
        let stroke_w = style.visuals.selection.stroke.width;
        let stroke_color = if interaction.selected {
            style.visuals.selection.stroke.color
        } else if interaction.in_selection_rect || interaction.hovered {
            style.visuals.weak_text_color()
        } else {
            egui::Color32::TRANSPARENT
        };
        let stroke = egui::Stroke::new(stroke_w, stroke_color);

        // Use a custom, transparent frame for comment nodes. The window margin
        // becomes an inner margin: an invisible band around the text that is the
        // node's only draggable region (the text itself captures the pointer).
        let frame = egui::Frame::new()
            .fill(egui::Color32::TRANSPARENT)
            .inner_margin(style.spacing.window_margin)
            .corner_radius(style.visuals.window_corner_radius)
            .stroke(stroke);

        // Use a transparent frame with resizable content
        let node_egui_id = uictx.egui_id();
        let resize_id = node_egui_id.with("resize");
        let min_resize = egui::Vec2::splat(style.interaction.interact_radius);
        let default_size = egui::vec2(self.size[0] as f32, self.size[1] as f32);
        let framed = uictx.framed_with(frame, |ui, _sockets| {
            // `Resize` registers its corner interaction under this salt (egui
            // 0.34.x internal, see `containers/resize.rs`). Reading the previous
            // frame's response tells us whether the corner is being dragged.
            let corner_id = resize_id.with("__resize_corner");
            let resizing = ui
                .ctx()
                .read_response(corner_id)
                .is_some_and(|r| r.dragged());

            // While dragging, keep requesting repaints so the frame *after*
            // release - when the height snaps back to fit the text - is drawn.
            if resizing {
                ui.ctx().request_repaint();
            }

            egui::containers::Resize::default()
                .id(resize_id)
                // Width is user-resizable (and persists). Height auto-fits the
                // text - except while the corner is actively dragged, when it
                // follows the cursor and snaps back to fit on release.
                .resizable(egui::Vec2b::new(
                    interaction.selected,
                    interaction.selected && resizing,
                ))
                .default_size(default_size)
                .min_size(min_resize)
                .with_stroke(false)
                .show(ui, |ui| {
                    // The width the user has dragged to; the height auto-fits the
                    // text below (so we don't read it from `available_size`).
                    let width = ui.available_width();

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

                    // Render the TextEdit against the buffered string. With
                    // auto-height the box always fits its text, so no scroll
                    // area is needed - the TextEdit reports its wrapped height.
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut state.text)
                            .desired_rows(1)
                            .hint_text("Add comment...")
                            .frame(egui::Frame::NONE)
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
                        // A flush that alters the stored text is a CA edit.
                        changed |= self.text != state.text;
                        self.text = state.text.clone();
                        state.text_hash = text_hash(&self.text);
                    }

                    // Schedule a repaint at the timeout for reactive mode.
                    if buffer_dirty && !should_flush {
                        let remaining = 10.0 - (time - state.last_edit_time);
                        if remaining > 0.0 {
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_secs_f64(
                                    remaining,
                                ));
                        }
                    }

                    // Persist the editing state.
                    ui.memory_mut(|m| m.data.insert_temp(text_id, state));

                    // The fitted size: the dragged width and the content height
                    // the box auto-fits to. `size` is part of the content
                    // address, so only commit it once *settled* - never while
                    // the corner is actively dragged (which would churn a new
                    // commit every frame of the drag). On release the height
                    // snaps back to fit, giving a single settled value.
                    let new_size = [width as u16, ui.min_rect().height() as u16];
                    if !resizing && self.size != new_size {
                        self.size = new_size;
                        changed = true;
                    }

                    response
                })
        });

        let mut resp = NodeUiResponse::new(framed);
        resp.set_changed(changed);
        resp
    }

    fn inspector_rows(
        &mut self,
        _ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let row_h = node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("size");
            });
            row.col(|ui| {
                ui.label(format!("{:?}", self.size));
            });
        });
        InspectorRowsResponse::default()
    }
}

#[cfg(test)]
mod tests {
    use super::Comment;
    use gantz_ca::content_addr;

    /// `size` is now part of the content address, so a resize is a genuine edit
    /// (this is what lets the `changed` signal at a settled resize map to a real
    /// commit). Identical fields still produce an identical address.
    #[test]
    fn size_is_part_of_content_address() {
        let a = Comment {
            text: "hi".into(),
            size: [100, 40],
        };
        let b = Comment {
            text: "hi".into(),
            size: [200, 40],
        };
        let c = Comment {
            text: "hi".into(),
            size: [100, 40],
        };
        assert_ne!(content_addr(&a), content_addr(&b));
        assert_eq!(content_addr(&a), content_addr(&c));
    }

    /// Text remains part of the content address.
    #[test]
    fn text_is_part_of_content_address() {
        let a = Comment {
            text: "hi".into(),
            size: [100, 40],
        };
        let b = Comment {
            text: "bye".into(),
            size: [100, 40],
        };
        assert_ne!(content_addr(&a), content_addr(&b));
    }
}
