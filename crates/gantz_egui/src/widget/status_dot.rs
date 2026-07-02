//! A painter-drawn status indicator dot.
//!
//! Circle glyphs (e.g. `\u{25CF}`) are not covered by egui's default fonts
//! on all platforms and can render as placeholder boxes; painting the dot
//! sidesteps fonts entirely.

/// A small filled circle tinted `color`, sized relative to body text and
/// hoverable (e.g. for a status label).
pub fn status_dot(ui: &mut egui::Ui, color: egui::Color32) -> egui::Response {
    let h = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(h * 0.7, h), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().circle_filled(rect.center(), h * 0.22, color);
    }
    response
}
