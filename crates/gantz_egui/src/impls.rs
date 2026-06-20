//! A submodule providing implementations for core and std gantz nodes.

mod apply;
mod bang;
mod branch;
mod delay;
mod expr;
mod identity;
mod inlet;
mod log;
mod number;
mod outlet;

/// The `desired_width` a syntax-highlighted code [`egui::TextEdit`] needs so its
/// widest line does not wrap.
///
/// A multiline `TextEdit` lays out its text within `desired_width` minus its
/// horizontal `margin`, so we measure the same (unwrapped) highlighted galley
/// the editor renders and add the margin back, plus 1px for sub-pixel rounding.
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
