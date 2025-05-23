//! A collection of useful widgets for gantz.

pub use log_view::LogView;

pub mod log_view;

/// Simple shorthand for viewing steel code.
pub fn steel_view(ui: &mut egui::Ui, code: &str) {
    let language = "scm";
    let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
    egui_extras::syntax_highlighting::code_view_ui(ui, &theme, code, language);
}
