//! The fonts of the UI. IBM Plex Mono sets all text by default, with egui's
//! own fonts as fallbacks for the glyphs it lacks. See [`Font`].

/// The family of the semibold weight, for titles and headings. See
/// [`strong`].
pub const STRONG: &str = "gantz-strong";

/// The id under which a context records the font it has installed.
const INSTALLED_ID: &str = "gantz-fonts-installed";

const REGULAR: &str = "ibm-plex-mono";
const SEMIBOLD: &str = "ibm-plex-mono-semibold";

/// The font of all text.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize,
)]
pub enum Font {
    /// IBM Plex Mono, for text and code alike.
    #[default]
    PlexMono,
    /// egui's own monospace face, for text and code alike.
    EguiMono,
    /// egui's own fonts: a proportional face for text and a monospace face
    /// for code.
    Egui,
}

/// The font definitions of the UI in `font`.
pub fn definitions(font: Font) -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    let fallbacks = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let strong = match font {
        Font::PlexMono => {
            fonts.font_data.insert(
                REGULAR.to_string(),
                egui::FontData::from_static(include_bytes!("../../fonts/IBMPlexMono-Regular.ttf"))
                    .into(),
            );
            fonts.font_data.insert(
                SEMIBOLD.to_string(),
                egui::FontData::from_static(include_bytes!("../../fonts/IBMPlexMono-SemiBold.ttf"))
                    .into(),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, REGULAR.to_string());
            }
            std::iter::once(SEMIBOLD.to_string())
                .chain(fallbacks)
                .collect()
        }
        // egui ships no semibold face, so titles keep the regular weight.
        Font::EguiMono => {
            let mono = fonts
                .families
                .get(&egui::FontFamily::Monospace)
                .cloned()
                .unwrap_or_default();
            let text: Vec<String> = mono.into_iter().chain(fallbacks).collect();
            fonts
                .families
                .insert(egui::FontFamily::Proportional, text.clone());
            text
        }
        Font::Egui => fallbacks,
    };
    fonts
        .families
        .insert(egui::FontFamily::Name(STRONG.into()), strong);
    fonts
}

/// Install `font` and the image loaders in `ctx`, unless done before.
///
/// Cheap enough to call every frame. A pop-out window's fresh context
/// installs them on its first frame.
pub fn install(ctx: &egui::Context, font: Font) {
    let id = egui::Id::new(INSTALLED_ID);
    let installed = ctx.data(|d| d.get_temp::<Font>(id));
    if installed == Some(font) {
        return;
    }
    ctx.set_fonts(definitions(font));
    if installed.is_none() {
        egui_extras::install_image_loaders(ctx);
    }
    ctx.data_mut(|d| d.insert_temp(id, font));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// egui mono sets text in egui's monospace face, with egui's own text
    /// fonts after it for the glyphs it lacks.
    #[test]
    fn egui_mono_sets_text_in_the_monospace_face() {
        let fonts = definitions(Font::EguiMono);
        let family = |f: egui::FontFamily| fonts.families[&f].clone();
        let mono = family(egui::FontFamily::Monospace);
        let text = family(egui::FontFamily::Proportional);
        assert_eq!(text.first(), mono.first());
        let defaults = egui::FontDefinitions::default();
        let proportional = &defaults.families[&egui::FontFamily::Proportional];
        assert!(proportional.iter().all(|f| text.contains(f)));
    }
}
