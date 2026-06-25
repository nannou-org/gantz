use crate::{NodeCtx, NodeUi, Registry, SocketDoc, SocketKind};

/// A widget used to allow for editing and parsing a steel expression.
pub struct ExprEdit<'a> {
    expr: &'a mut gantz_core::node::Expr,
    pub id: egui::Id,
}

#[derive(Clone, Default)]
struct ExprEditState {
    expr_hash: u64,
    code: String,
}

impl<'a> egui::Widget for ExprEdit<'a> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let Self { expr, id } = self;
        let code_id = id.with("code");

        // Retrieve the working state.
        let mut state: ExprEditState = ui
            .memory_mut(|m| m.data.remove_temp(code_id))
            .unwrap_or_default();

        // If the input hash has changed, reset the working code string.
        let expr_hash = expr_hash(expr);
        if expr_hash != state.expr_hash {
            state.expr_hash = expr_hash;
            state.code = expr.src().to_string();
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

        // Size the editor to its widest line. A multiline `TextEdit` wraps its
        // text within `desired_width` minus its horizontal margin, so measure
        // the same (unwrapped) highlighted layout the editor renders and pass a
        // matching `desired_width` and `margin`.
        let font_id = egui::FontSelection::from(egui::TextStyle::Monospace).resolve(ui.style());
        let margin = egui::Margin::symmetric(4, 2);
        let desired_width =
            super::code_edit_desired_width(ui, &theme, &state.code, language, margin);

        let response = ui.add(
            egui::TextEdit::multiline(&mut state.code)
                .id(id)
                .code_editor()
                .font(font_id)
                .margin(margin)
                .desired_rows(1)
                .desired_width(desired_width)
                .hint_text("(+ $l $r)")
                .layouter(&mut layouter),
        );
        if response.changed() {
            if let Ok(new_expr) = gantz_core::node::expr(&state.code) {
                *expr = new_expr;
            }
        }

        // Persist the WIP editing code.
        ui.memory_mut(|m| m.data.insert_temp(code_id, state));

        response
    }
}

impl NodeUi for gantz_core::node::Expr {
    fn name(&self, _: &dyn crate::Registry) -> &str {
        "expr"
    }

    fn ui(
        &mut self,
        ctx: NodeCtx,
        uictx: egui_graph::NodeCtx,
    ) -> egui_graph::FramedResponse<egui::Response> {
        uictx.framed(|ui, _sockets| {
            let id = egui::Id::new("ExprEdit").with(ctx.path());
            ui.add(ExprEdit::new(self, id))
        })
    }

    fn inspector_rows(&mut self, _ctx: &mut NodeCtx, body: &mut egui_extras::TableBody) {
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
                }
            });
        });
    }

    fn socket_doc(&self, _: &dyn Registry, kind: SocketKind, ix: usize) -> Option<SocketDoc> {
        match kind {
            SocketKind::Input => match self.vars().get(ix) {
                Some(var) => {
                    let desc = if var.starts_with("$?") {
                        "optional input; bound as (Some value) or (None)"
                    } else {
                        "substituted into the expression"
                    };
                    Some(SocketDoc::ty(var.clone()).with_description(desc))
                }
                // The synthetic trigger input present when there are no `$vars`.
                None => Some(
                    SocketDoc::ty("trigger")
                        .with_description("ignored; forces the expression to evaluate"),
                ),
            },
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
        Self { expr, id }
    }
}

fn expr_hash(expr: &gantz_core::node::Expr) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::default();
    expr.hash(&mut hasher);
    hasher.finish()
}
