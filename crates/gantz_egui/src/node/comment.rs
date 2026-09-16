//! A Comment node for documenting patches.

use super::size_sync::{self, fitted_size};
use crate::widget::node_inspector;
use crate::{InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse};
use gantz_core::node::{self, ExprCtx, ExprResult, MetaCtx};
use gantz_nodetag::NodeTag;
use serde::{Deserialize, Serialize};

/// Buffered text edits stored in egui memory. Flushing per keystroke would
/// mint a commit per keystroke, so the buffer flushes to the node on focus
/// loss. See [`NodeUi`] for the `changed` contract.
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
/// Both `text` and `size` are part of the content address. Editing the note or
/// resizing it produces a new commit, so a resize is undoable just like a text
/// edit.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, NodeTag)]
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
    fn n_inputs(&self, _ctx: MetaCtx) -> usize {
        0
    }

    fn n_outputs(&self, _ctx: MetaCtx) -> usize {
        0
    }

    fn expr(&self, _ctx: ExprCtx<'_, '_>) -> ExprResult {
        node::parse_expr("void")
    }
}

impl NodeUi for Comment {
    fn name(&self, _registry: &crate::Env<'_>) -> std::borrow::Cow<'_, str> {
        "comment".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("A free-floating text note")
    }

    fn ui(&mut self, _ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // Set when a flushed text change or a settled resize lands this frame.
        let mut changed = false;
        let interaction = uictx.interaction();
        let style = uictx.style();
        // Match the regular node selection outline with a thin stroke at the
        // node's edge. The large draggable band lives in the invisible inner
        // margin below, so the border stays subtle while the node remains easy
        // to grab.
        let stroke_w = style.visuals.selection.stroke.width;
        let stroke_color = if interaction.selected {
            style.visuals.selection.stroke.color
        } else if interaction.in_selection_rect || interaction.hovered {
            style.visuals.weak_text_color()
        } else {
            egui::Color32::TRANSPARENT
        };
        let stroke = egui::Stroke::new(stroke_w, stroke_color);

        // Use a custom, transparent frame. The window margin becomes an inner
        // margin. That invisible band around the text is the node's only
        // draggable region, since the text itself captures the pointer.
        let frame = egui::Frame::new()
            .fill(egui::Color32::TRANSPARENT)
            .inner_margin(style.spacing.window_margin)
            .corner_radius(style.visuals.window_corner_radius)
            .stroke(stroke);

        let node_egui_id = uictx.egui_id();
        let resize_id = node_egui_id.with("resize");
        let size_sync_id = node_egui_id.with("size_sync");
        let min_resize = egui::Vec2::splat(style.interaction.interact_radius);
        let default_size = egui::vec2(self.size[0] as f32, self.size[1] as f32);
        let framed = uictx.framed_with(frame, |ui, _sockets| {
            let size_sync::Decisions {
                resizing,
                push_external,
                drag_released,
            } = size_sync::begin(ui, size_sync_id, resize_id, self.size);

            let resize = egui::containers::Resize::default()
                .id(resize_id)
                .with_stroke(false);
            let resize = if push_external {
                // One-frame push of the committed size into the displayed
                // resize state. `fixed_size` sets min and max to the size.
                // egui clamps the stored state through min and max on
                // `begin` and stores it back on `end`, so this overrides
                // persisted state. It also leaves the corner unregistered
                // this frame, which cancels any in-flight drag. External
                // changes win.
                ui.ctx().request_repaint();
                let w = (self.size[0] as f32).max(min_resize.x);
                let h = (self.size[1] as f32).max(min_resize.y);
                resize.fixed_size(egui::vec2(w, h))
            } else {
                // Width is user-resizable and persists. Height auto-fits the
                // text. While the corner is dragged it follows the cursor
                // instead, then snaps back to fit on release.
                resize
                    .resizable(egui::Vec2b::new(
                        interaction.selected,
                        interaction.selected && resizing,
                    ))
                    .default_size(default_size)
                    .min_size(min_resize)
            };
            let inner = resize.show(ui, |ui| {
                // The width the user has dragged to. The height auto-fits the
                // text below, so it is not read from `available_size`.
                let width = ui.available_width();

                let text_id = node_egui_id.with("comment_text");

                let mut state: CommentEditState = ui
                    .memory_mut(|m| m.data.remove_temp(text_id))
                    .unwrap_or_default();

                // Sync from the node when its text changed externally, for
                // example by undo.
                let current_hash = text_hash(&self.text);
                if current_hash != state.text_hash {
                    state.text_hash = current_hash;
                    state.text = self.text.clone();
                }

                // Render the TextEdit against the buffered string. With
                // auto-height the box always fits its text, so no scroll
                // area is needed. The TextEdit reports its wrapped height.
                let response = ui.add(
                    egui::TextEdit::multiline(&mut state.text)
                        .desired_rows(1)
                        .hint_text("Add comment...")
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                );

                let time = ui.input(|i| i.time);
                if response.changed() {
                    state.last_edit_time = time;
                }

                let buffer_dirty = text_hash(&state.text) != state.text_hash;

                // Flush on focus loss, after 5 seconds without an edit, or on
                // any mouse activity while the buffer is dirty.
                let timed_out = buffer_dirty && (time - state.last_edit_time >= 5.0);
                let mouse_active = buffer_dirty
                    && ui.input(|i| {
                        i.pointer.is_moving() || i.pointer.any_pressed() || i.pointer.any_released()
                    });
                let should_flush = response.lost_focus() || timed_out || mouse_active;

                let mut text_flushed = false;
                if should_flush {
                    // A flush that alters the stored text is a CA edit.
                    text_flushed = self.text != state.text;
                    changed |= text_flushed;
                    self.text = state.text.clone();
                    state.text_hash = text_hash(&self.text);
                }

                // Schedule a repaint at the timeout for reactive mode.
                if buffer_dirty && !should_flush {
                    let remaining = 10.0 - (time - state.last_edit_time);
                    if remaining > 0.0 {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_secs_f64(remaining));
                    }
                }

                ui.memory_mut(|m| m.data.insert_temp(text_id, state));

                // The fitted size is the dragged width and the content
                // height the box auto-fits to. `size` is part of the content
                // address, so only genuine local interaction writes it. That
                // is a flushed text change, where the auto-fit height rides
                // along, or a settled corner-drag release. External changes
                // such as undo or collab sync are pushed into the resize
                // state above instead. A locally drifting auto-fit height
                // from fonts, DPI or rounding must never correct the
                // committed value. That would mint spurious commits and loop
                // between collaborating peers. The displayed height still
                // auto-fits every frame. The committed height goes stale
                // until the next genuine edit.
                let fitted = fitted_size(width, ui.min_rect().height());
                let text_committed = text_flushed && !resizing && !push_external;
                if (text_committed || drag_released) && self.size != fitted {
                    self.size = fitted;
                    changed = true;
                }

                response
            });

            size_sync::store(ui, size_sync_id, self.size, push_external, resizing);

            inner
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

    /// The node's erased (data-layer) content address.
    fn content_addr(c: &Comment) -> gantz_ca::ContentAddr {
        gantz_core::data::erase_node_typed(c)
            .unwrap()
            .content_addr()
    }

    /// `size` is part of the content address, so a resize is a genuine edit.
    /// Identical fields produce an identical address.
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

    /// Text is part of the content address.
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
