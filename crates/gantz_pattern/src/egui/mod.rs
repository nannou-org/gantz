//! The egui surface for the pattern node set. Only egui-flavoured items
//! live here, so the crate's `egui` feature holds with a single cfg gate.

use crate::{Pm, mini};
use gantz_egui::{Env, NodeCtx, NodeUi, NodeUiResponse, SocketDoc, SocketKind};
use std::hash::{Hash, Hasher};

/// The buffered edit state for a [`Pm`] node's notation editor, held in
/// egui temp memory so keystrokes do not commit a new content address
/// each frame. Mirrors the comment node's flush-on-settle behaviour.
#[derive(Clone, Default)]
struct PmEditState {
    src_hash: u64,
    text: String,
    last_edit_time: f64,
}

/// Seconds of no typing before a dirty buffer flushes to the node.
const FLUSH_TIMEOUT: f64 = 2.0;

fn hash_str(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::default();
    s.hash(&mut h);
    h.finish()
}

impl NodeUi for Pm {
    fn name(&self, _: &Env<'_>) -> std::borrow::Cow<'_, str> {
        "pm".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Parse mini-notation into a pattern at compile time")
    }

    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        let mut changed = false;
        let framed = uictx.framed(|ui, _sockets| {
            let state_id = egui::Id::new("PmEdit").with(ctx.path());
            let mut state: PmEditState = ui
                .memory_mut(|m| m.data.remove_temp(state_id))
                .unwrap_or_default();

            // Resync the buffer when the node changed externally (undo,
            // collab, a flushed edit elsewhere).
            let src_hash = hash_str(self.src());
            if src_hash != state.src_hash {
                state.src_hash = src_hash;
                state.text = self.src().to_string();
            }

            // A live parse check on the buffer: tint the text while the
            // notation is malformed (it compiles to silence).
            let parses = mini::steel_src(&state.text).is_some();
            let font_id = egui::FontSelection::from(egui::TextStyle::Monospace).resolve(ui.style());

            // Size the editor to the live buffer like the expr node: a
            // TextEdit lays its text out within `desired_width` minus its
            // horizontal margin, so measure the unwrapped galley and add
            // the margin back, plus 1px for sub-pixel rounding. The hint's
            // width is the floor so an empty node still shows it.
            const HINT: &str = "bd(3,8) ~ [sn sn]";
            let margin = egui::Margin::symmetric(4, 2);
            let measure = |ui: &egui::Ui, text: &str| {
                ui.ctx().fonts_mut(|f| {
                    f.layout_no_wrap(text.to_string(), font_id.clone(), egui::Color32::WHITE)
                        .rect
                        .width()
                })
            };
            let text_w = measure(ui, &state.text).max(measure(ui, HINT));
            let desired_width = text_w.ceil() + margin.sum().x + 1.0;

            let mut edit = egui::TextEdit::singleline(&mut state.text)
                .font(font_id.clone())
                .hint_text(HINT)
                .margin(margin)
                .desired_width(desired_width);
            if !parses {
                edit = edit.text_color(ui.visuals().error_fg_color);
            }
            let response = ui.add(edit);
            let response = if parses {
                response
            } else {
                response
                    .on_hover_text("not a valid pattern - the node keeps its last valid notation")
            };

            let time = ui.input(|i| i.time);
            if response.changed() {
                state.last_edit_time = time;
            }
            let buffer_dirty = state.text != self.src();
            let timed_out = buffer_dirty && (time - state.last_edit_time >= FLUSH_TIMEOUT);
            let mouse_active = buffer_dirty
                && ui.input(|i| {
                    i.pointer.is_moving() || i.pointer.any_pressed() || i.pointer.any_released()
                });
            // Only a parsing buffer commits, mirroring the expr editor:
            // an invalid buffer changes the text but never the node, so
            // the last valid pattern keeps playing while mid-edit.
            if (response.lost_focus() || timed_out || mouse_active) && parses {
                if buffer_dirty {
                    self.set_src(state.text.clone());
                    state.src_hash = hash_str(self.src());
                    changed = true;
                }
            } else if buffer_dirty {
                let remaining = FLUSH_TIMEOUT - (time - state.last_edit_time);
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(remaining.max(0.0)));
            }

            ui.memory_mut(|m| m.data.insert_temp(state_id, state));
            response
        });
        let mut resp = NodeUiResponse::new(framed);
        resp.set_changed(changed);
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, _ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => {
                Some(SocketDoc::ty("bang").with_description("triggers emission of the pattern"))
            }
            SocketKind::Output => Some(
                SocketDoc::ty("pattern")
                    .with_description("the parsed pattern, silence when malformed"),
            ),
        }
    }
}
