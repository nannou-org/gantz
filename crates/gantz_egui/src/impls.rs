//! [`crate::NodeUi`] implementations for nodes defined in other crates.

use crate::syntax;

mod apply;
mod branch;
mod delay;
pub(crate) mod expr;
mod identity;
mod inlet;
mod list;
mod log;
mod main_bang;
pub(crate) mod named_ref;
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
    theme: &syntax::Theme,
    code: &str,
    margin: egui::Margin,
) -> f32 {
    let galley = code_galley(ui, theme, code, f32::INFINITY);
    galley.rect.width().ceil() + margin.sum().x + 1.0
}

/// Scheme `code` laid out in its syntax colours and wrapped at `wrap_width`.
fn code_galley(
    ui: &egui::Ui,
    theme: &syntax::Theme,
    code: &str,
    wrap_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = syntax::job(code, syntax::Mode::Code, theme);
    job.wrap.max_width = wrap_width;
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// The text of an inlet or outlet node, such as `in[0] tick`. The index comes
/// first as it is the identity of the socket, and `ty` is what a reader wants
/// to know about it.
fn marker_label(name: &str, ix: usize, ty: &str) -> String {
    match ty {
        "" => format!("{name}[{ix}]"),
        ty => format!("{name}[{ix}] {ty}"),
    }
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

#[cfg(test)]
mod tests {
    use super::marker_label;

    #[test]
    fn a_marker_shows_its_index_then_its_type() {
        assert_eq!(marker_label("in", 1, "clock"), "in[1] clock");
        assert_eq!(marker_label("out", 0, ""), "out[0]");
    }
}
