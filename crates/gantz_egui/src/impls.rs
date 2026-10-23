//! [`crate::NodeUi`] implementations for nodes defined in other crates.

mod apply;
mod branch;
mod delay;
mod expr;
mod identity;
mod inlet;
mod list;
mod log;
mod main_bang;
mod named_ref;
mod outlet;

/// The `desired_width` a syntax-highlighted code [`egui::TextEdit`] needs so its
/// widest line does not wrap.
///
/// A multiline `TextEdit` lays out its text within `desired_width` minus its
/// horizontal `margin`. This measures the same unwrapped highlighted galley the
/// editor renders and adds the margin back, plus 1px for sub-pixel rounding.
/// Pass the same `margin` that is set on the editor via [`egui::TextEdit::margin`].
fn code_edit_desired_width(
    ui: &egui::Ui,
    theme: &egui_extras::syntax_highlighting::CodeTheme,
    code: &str,
    language: &str,
    margin: egui::Margin,
) -> f32 {
    let mut job =
        egui_extras::syntax_highlighting::highlight(ui.ctx(), ui.style(), theme, code, language);
    job.wrap.max_width = f32::INFINITY;
    let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
    galley.rect.width().ceil() + margin.sum().x + 1.0
}

/// Show the type and description of an inlet or outlet `node` while the
/// pointer is over it, at once and without the usual tooltip delay. An empty
/// type or description says so.
fn marker_hover(node: &egui::Response, ty: &str, description: &str) {
    if !node.hovered() || node.ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    egui::Tooltip::for_widget(node).show(|ui| {
        match ty.is_empty() {
            true => ui.weak("no type"),
            false => ui.strong(ty),
        };
        match description.is_empty() {
            true => ui.weak("no description"),
            false => ui.label(description),
        };
    });
}
