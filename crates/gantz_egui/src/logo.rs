//! The gantz logo, from the SVG files of the `gantz-logo` project.
//!
//! The files are white on transparent, so an image of one takes any colour
//! through `egui::Image::tint`. Drawing them needs the image loaders that
//! [`crate::style::apply`] installs.

/// The mark: the `g` alone.
pub const MARK: egui::ImageSource<'static> = egui::include_image!("../assets/logo/mark-white.svg");

/// The mark as a line along its outline. For large, quiet uses such as an
/// empty scene.
pub const MARK_OUTLINE: egui::ImageSource<'static> =
    egui::include_image!("../assets/logo/mark-outline-white.svg");

/// The wordmark: "gantz" in the letters of the mark.
pub const WORDMARK: egui::ImageSource<'static> =
    egui::include_image!("../assets/logo/wordmark-white.svg");

/// The width of the mark over its height.
const MARK_ASPECT: f32 = 337.0 / 399.0;

/// The width of the outline mark over its height.
const MARK_OUTLINE_ASPECT: f32 = 345.0 / 407.0;

/// The width of the wordmark over its height.
const WORDMARK_ASPECT: f32 = 1506.0 / 456.0;

/// The mark, `height` points tall, in `color`.
pub fn mark(height: f32, color: egui::Color32) -> egui::Image<'static> {
    sized(MARK, MARK_ASPECT, height, color)
}

/// The outline mark, `height` points tall, in `color`.
pub fn mark_outline(height: f32, color: egui::Color32) -> egui::Image<'static> {
    sized(MARK_OUTLINE, MARK_OUTLINE_ASPECT, height, color)
}

/// The wordmark, `height` points tall, in `color`.
pub fn wordmark(height: f32, color: egui::Color32) -> egui::Image<'static> {
    sized(WORDMARK, WORDMARK_ASPECT, height, color)
}

fn sized(
    source: egui::ImageSource<'static>,
    aspect: f32,
    height: f32,
    color: egui::Color32,
) -> egui::Image<'static> {
    egui::Image::new(source)
        .fit_to_exact_size(egui::vec2(height * aspect, height))
        .tint(color)
}
