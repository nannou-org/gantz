//! The fonts of the UI. IBM Plex Mono sets all text, with egui's own fonts as
//! fallbacks for the glyphs it lacks.

/// The family of the semibold weight, for titles and headings. See
/// [`strong`].
pub const STRONG: &str = "gantz-strong";

/// The id under which a context records that the fonts are installed.
const INSTALLED_ID: &str = "gantz-fonts-installed";

const REGULAR: &str = "ibm-plex-mono";
const SEMIBOLD: &str = "ibm-plex-mono-semibold";

/// The font definitions of the UI.
pub fn definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        REGULAR.to_string(),
        egui::FontData::from_static(include_bytes!("../../fonts/IBMPlexMono-Regular.ttf")).into(),
    );
    fonts.font_data.insert(
        SEMIBOLD.to_string(),
        egui::FontData::from_static(include_bytes!("../../fonts/IBMPlexMono-SemiBold.ttf")).into(),
    );
    let fallbacks = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, REGULAR.to_string());
    }
    let strong = std::iter::once(SEMIBOLD.to_string())
        .chain(fallbacks)
        .collect();
    fonts
        .families
        .insert(egui::FontFamily::Name(STRONG.into()), strong);
    fonts
}

/// Install the fonts and image loaders in `ctx`, unless done before.
///
/// Cheap enough to call every frame. A pop-out window's fresh context
/// installs them on its first frame.
pub fn install(ctx: &egui::Context) {
    let id = egui::Id::new(INSTALLED_ID);
    if ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    ctx.set_fonts(definitions());
    egui_extras::install_image_loaders(ctx);
    ctx.data_mut(|d| d.insert_temp(id, true));
}

/// The semibold font of `style` at its size.
pub fn strong(ui: &egui::Ui, style: egui::TextStyle) -> egui::FontId {
    egui::FontId::new(style.resolve(ui.style()).size, strong_family(ui.ctx()))
}

/// `text` in the semibold weight.
pub fn strong_text(ui: &egui::Ui, text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).family(strong_family(ui.ctx()))
}

/// The semibold family, or the proportional family on the first frame of a
/// context, before the fonts that [`install`] sets take effect.
fn strong_family(ctx: &egui::Context) -> egui::FontFamily {
    let strong = egui::FontFamily::Name(STRONG.into());
    match ctx.fonts(|f| f.definitions().families.contains_key(&strong)) {
        true => strong,
        false => egui::FontFamily::Proportional,
    }
}
