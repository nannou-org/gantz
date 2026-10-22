use crate::{
    Env, InspectorRowsResponse, NodeCtx, NodeUi, NodeUiResponse, NodeViewResponse, SocketDoc,
    SocketKind, style, widget::kit,
};

/// A widget for editing and parsing a Steel expression.
pub struct ExprEdit<'a> {
    expr: &'a mut gantz_core::node::Expr,
    pub id: egui::Id,
    /// When `true`, the editor fills the available width and height instead
    /// of sizing to its widest line. The detached view uses this.
    fill: bool,
}

#[derive(Clone, Default)]
struct ExprEditState {
    expr_hash: u64,
    code: String,
    /// Why the working code does not parse, if it does not.
    error: Option<String>,
}

impl<'a> egui::Widget for ExprEdit<'a> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let Self { expr, id, fill } = self;
        let code_id = id.with("code");

        let mut state: ExprEditState = ui
            .memory_mut(|m| m.data.remove_temp(code_id))
            .unwrap_or_default();

        // If the input hash has changed, reset the working code string.
        let expr_hash = expr_hash(expr);
        if expr_hash != state.expr_hash {
            state.expr_hash = expr_hash;
            state.code = expr.src().to_string();
            state.error = None;
        }

        let language = "scm";
        let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());

        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut layout_job = egui_extras::syntax_highlighting::highlight(
                ui.ctx(),
                ui.style(),
                &theme,
                buf.as_str(),
                language,
            );
            layout_job.wrap.max_width = wrap_width;
            ui.fonts_mut(|fonts| fonts.layout_job(layout_job))
        };

        // `margin` is passed to both the editor and the width measurement.
        let font_id = egui::FontSelection::from(egui::TextStyle::Monospace).resolve(ui.style());
        let margin = egui::Margin::symmetric(4, 2);
        // Fill the pane in the detached view, or size to the widest line in
        // the graph.
        let (desired_width, desired_rows) = if fill {
            let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
            let rows = ((ui.available_height() / row_h).floor() as usize).max(1);
            (ui.available_width(), rows)
        } else {
            (
                super::code_edit_desired_width(ui, &theme, &state.code, language, margin),
                1,
            )
        };

        // The field keeps one outline width and no expansion in every state, so
        // that hover and focus only recolour its outline and never move its
        // text.
        let response = ui
            .scope(|ui| {
                let width = ui.visuals().selection.stroke.width;
                let widgets = &mut ui.visuals_mut().widgets;
                for w in [
                    &mut widgets.inactive,
                    &mut widgets.hovered,
                    &mut widgets.active,
                ] {
                    w.bg_stroke.width = width;
                    w.expansion = 0.0;
                }
                ui.add(
                    egui::TextEdit::multiline(&mut state.code)
                        .id(id)
                        .code_editor()
                        .font(font_id)
                        .margin(margin)
                        .desired_rows(desired_rows)
                        .desired_width(desired_width)
                        .hint_text("(+ $l $r)")
                        .layouter(&mut layouter),
                )
            })
            .inner;
        if response.changed() {
            match gantz_core::node::expr(&state.code) {
                Ok(new_expr) => {
                    // Preserve the user-set output count across text edits. Only
                    // the source, and thus the `$var` inputs, follows the edit.
                    *expr = new_expr.with_outputs(expr.outputs());
                    state.error = None;
                }
                Err(e) => state.error = Some(e.to_string()),
            }
        }

        ui.memory_mut(|m| m.data.insert_temp(code_id, state));

        response
    }
}

impl NodeUi for gantz_core::node::Expr {
    fn name(&self, _: &crate::Env<'_>) -> std::borrow::Cow<'_, str> {
        "expr".into()
    }

    fn description(&self) -> Option<&'static str> {
        Some("Evaluate a Steel expression")
    }

    fn ui(&mut self, ctx: NodeCtx, uictx: egui_graph::NodeCtx) -> NodeUiResponse {
        // `src` is part of the content address. Detect a real edit by hashing
        // the node before and after the editor. A keystroke that fails to parse
        // changes the buffer but not the node, so it must not mark `changed`.
        let before = expr_hash(self);
        let id = egui::Id::new("ExprEdit").with(ctx.path());
        let framed = uictx.framed(|ui, _sockets| ui.add(ExprEdit::new(self, id)));
        let node = &framed.inner.response;
        if let Some(error) = ExprEdit::error(&node.ctx, id) {
            unparsed_tag(node, &error);
        }
        let mut resp = NodeUiResponse::new(framed);
        resp.set_changed(expr_hash(self) != before);
        resp
    }

    fn view_ui(&mut self, _ctx: NodeCtx, ui: &mut egui::Ui) -> NodeViewResponse {
        // The same code editor as the in-graph node, but filling the pane. The
        // pane keeps its margin. The pane scopes `ui.id`, so the distinct id
        // keeps the WIP edit state separate from the in-graph editor.
        let before = expr_hash(self);
        let id = ui.id().with("expr-view");
        let res = ui.add(ExprEdit::new(self, id).fill(true));
        let mut resp = NodeViewResponse::default();
        resp.inner = Some(res);
        resp.set_changed(expr_hash(self) != before);
        resp
    }

    fn inspector_rows(
        &mut self,
        _ctx: &mut NodeCtx,
        body: &mut egui_extras::TableBody,
    ) -> InspectorRowsResponse {
        let mut resp = InspectorRowsResponse::default();
        let row_h = crate::widget::node_inspector::table_row_h(body.ui_mut());
        body.row(row_h, |mut row| {
            row.col(|ui| {
                ui.label("outputs");
            });
            row.col(|ui| {
                let mut n = self.outputs() as i32;
                if ui
                    .add(egui::DragValue::new(&mut n).range(1..=16).speed(0.1))
                    .changed()
                {
                    self.set_outputs(n.clamp(1, 16) as u8);
                    resp.mark_changed();
                }
            });
        });
        resp
    }

    fn socket_doc(&self, _: &Env<'_>, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => {
                match self.vars().get(ix) {
                    Some(var) => {
                        let desc = if var.starts_with("$?") {
                            "optional input. Bound as (Some value) when connected, else (None)"
                        } else {
                            "substituted into the expression"
                        };
                        Some(SocketDoc::ty(var.clone()).with_description(desc))
                    }
                    // The synthetic trigger input present when there are no `$vars`.
                    None => Some(SocketDoc::ty("trigger").with_description(
                        "ignored. A push into it forces the expression to evaluate",
                    )),
                }
            }
            SocketKind::Output if self.outputs() <= 1 => {
                Some(SocketDoc::ty("any").with_description("expression result"))
            }
            SocketKind::Output => {
                Some(SocketDoc::ty("any").with_description(format!("result element {ix}")))
            }
        }
    }
}

impl<'a> ExprEdit<'a> {
    /// Create a new Steel code editing widget.
    pub fn new(expr: &'a mut gantz_core::node::Expr, id: egui::Id) -> Self {
        Self {
            expr,
            id,
            fill: false,
        }
    }

    /// Fill the available width and height rather than sizing to the widest
    /// line. The detached view uses this.
    pub fn fill(mut self, fill: bool) -> Self {
        self.fill = fill;
        self
    }

    /// Why the working code of the editor `id` does not parse, if it does
    /// not.
    pub fn error(ctx: &egui::Context, id: egui::Id) -> Option<String> {
        ctx.data(|d| d.get_temp::<ExprEditState>(id.with("code")))
            .and_then(|state| state.error)
    }
}

fn expr_hash(expr: &gantz_core::node::Expr) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::default();
    expr.hash(&mut hasher);
    hasher.finish()
}

/// Hang a tag under the expr `node` that says why its working code does not
/// parse. The node keeps its last code that parsed until then.
fn unparsed_tag(node: &egui::Response, error: &str) {
    let mut painter = node.ctx.layer_painter(node.layer_id);
    // Node layers are transformed, so their clip is layer-local.
    painter.set_clip_rect(node.rect.expand2(egui::vec2(320.0, 40.0)));
    let style = node.ctx.global_style();
    let text = style::color_of(
        &node.ctx,
        style::theme_of(&style.visuals),
        style::Role::Panel,
    );
    let pos = node.rect.left_bottom() + egui::vec2(0.0, kit::TAG_GAP);
    let fill = style.visuals.warn_fg_color;
    kit::paint_tag(
        &painter,
        &style,
        pos,
        &kit::elide(error, kit::TAG_CHARS),
        fill,
        text,
    );
    node.clone().on_hover_text(format!("not applied: {error}"));
}
