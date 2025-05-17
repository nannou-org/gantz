use crate::{NodeCtx, NodeUi};

/// A widget used to allow for editing and parsing a steel expression.
pub struct ExprEdit<'a> {
    expr: &'a mut gantz_core::node::Expr,
    pub id: egui::Id,
}

impl<'a> egui::Widget for ExprEdit<'a> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let Self { expr, id } = self;
        let code_id = id.with("code");

        // Take the working string if there is one, or use the expr src.
        let mut code = ui
            .memory_mut(|m| {
                m.data
                    .get_persisted_mut_or::<Option<String>>(code_id, None)
                    .take()
            })
            .unwrap_or_else(|| expr.src().to_string());

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
            ui.fonts(|f| f.layout_job(layout_job))
        };

        // Find the longest line width.
        let mut max_line_width: f32 = 0.0;
        let font_sel = egui::FontSelection::from(egui::TextStyle::Monospace);
        let font_id = font_sel.resolve(ui.style());
        ui.fonts(|fonts| {
            for line in code.split('\n').clone() {
                // Use the layout_no_wrap function to get width without wrapping
                let galley = fonts.layout_no_wrap(
                    line.to_string(),
                    font_id.clone(),
                    egui::Color32::PLACEHOLDER,
                );
                max_line_width = max_line_width.max(galley.rect.width());
            }
        });
        max_line_width += 7.0;

        let response = ui.add(
            egui::TextEdit::multiline(&mut code)
                .id(id)
                .code_editor()
                .font(font_id)
                .desired_rows(1)
                .desired_width(max_line_width)
                .hint_text("(+ $l $r)")
                .layouter(&mut layouter),
        );
        if response.changed() {
            if let Ok(new_expr) = gantz_core::node::expr(&code) {
                *expr = new_expr;
            }
        }

        // Persist the WIP editing code.
        ui.memory_mut(|m| m.data.insert_persisted(code_id, Some(code)));

        response
    }
}

impl NodeUi for gantz_core::node::Expr {
    fn ui(&mut self, ctx: NodeCtx, ui: &mut egui::Ui) -> egui::Response {
        let id = egui::Id::new("ExprEdit").with(ctx.path());
        ui.add(ExprEdit::new(self, id))
    }
}

impl<'a> ExprEdit<'a> {
    /// Create a new Steel code editing widget.
    pub fn new(expr: &'a mut gantz_core::node::Expr, id: egui::Id) -> Self {
        Self { expr, id }
    }
}
