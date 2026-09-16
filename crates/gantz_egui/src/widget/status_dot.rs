//! A painter-drawn status indicator dot.
//!
//! Circle glyphs such as `\u{25CF}` are missing from egui's default fonts on
//! some platforms and render as placeholder boxes. Painting the dot avoids
//! fonts entirely.

/// A small filled circle tinted `color`. It is sized relative to body text
/// and senses hover, so a status label can hang off it.
pub fn status_dot(ui: &mut egui::Ui, color: egui::Color32) -> egui::Response {
    let h = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(h * 0.7, h), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().circle_filled(rect.center(), h * 0.22, color);
    }
    response
}
