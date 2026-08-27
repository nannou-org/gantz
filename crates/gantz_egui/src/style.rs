//! Global egui appearance: the theme preference and the per-theme
//! [`egui::Style`], edited in `Settings -> Style`.
//!
//! egui omits its own styles when serializing its `Options` (`dark_style` and
//! `light_style` are `#[serde(skip)]`), so gantz owns them: [`StyleConfig`]
//! rides [`GantzState`][crate::widget::GantzState] - which both hosts already
//! persist - and is applied to a context by [`apply`]. Owning the preference
//! also keeps pop-out pane windows in step, as each is a separate
//! `egui::Context` with its own options.

/// File extension for exported style files (without the leading dot).
pub const FILE_EXTENSION: &str = "ron";

/// The id under which a context tracks the config last applied to it.
const APPLIED_ID: &str = "gantz-applied-style";

/// The theme preference and per-theme style overrides.
///
/// A `None` style means egui's default for that theme, so an untouched config
/// costs nothing to store and follows egui's defaults across version bumps.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct StyleConfig {
    /// Whether to use the dark style, the light style, or follow the system.
    #[serde(default = "default_theme")]
    pub theme: egui::ThemePreference,
    /// The dark style, when customised.
    #[serde(default)]
    pub dark: Option<egui::Style>,
    /// The light style, when customised.
    #[serde(default)]
    pub light: Option<egui::Style>,
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            dark: None,
            light: None,
        }
    }
}

impl PartialEq for StyleConfig {
    fn eq(&self, other: &Self) -> bool {
        self.theme == other.theme
            && [egui::Theme::Dark, egui::Theme::Light]
                .into_iter()
                .all(|theme| match (slot(self, theme), slot(other, theme)) {
                    (None, None) => true,
                    (Some(a), Some(b)) => eq_style(a, b),
                    _ => false,
                })
    }
}

/// gantz's default theme preference.
///
/// Dark rather than egui's `System`: gantz's remaining hand-picked colours are
/// dark-tuned (#203), and `bevy_egui` reports no system theme, so following the
/// system would leave the app and the `gantz_egui` demo disagreeing.
fn default_theme() -> egui::ThemePreference {
    egui::ThemePreference::Dark
}

/// The style `cfg` specifies for `theme`, or egui's default for it.
pub fn style_of(cfg: &StyleConfig, theme: egui::Theme) -> egui::Style {
    slot(cfg, theme)
        .clone()
        .unwrap_or_else(|| theme.default_style())
}

/// Customise `theme`'s style.
///
/// A style equal to egui's default for the theme clears the customisation
/// rather than storing a redundant copy.
pub fn set_style_of(cfg: &mut StyleConfig, theme: egui::Theme, style: egui::Style) {
    let custom = (!eq_style(&style, &theme.default_style())).then_some(style);
    match theme {
        egui::Theme::Dark => cfg.dark = custom,
        egui::Theme::Light => cfg.light = custom,
    }
}

/// Drop `theme`'s customisation, restoring egui's default style for it.
pub fn reset_theme(cfg: &mut StyleConfig, theme: egui::Theme) {
    match theme {
        egui::Theme::Dark => cfg.dark = None,
        egui::Theme::Light => cfg.light = None,
    }
}

/// Apply `cfg` to `ctx`, unless it is already what was last applied to it.
///
/// Cheap enough to call every frame. The last-applied config is tracked in the
/// context's *temporary* data, so a context without one - a freshly spawned
/// pop-out window, or the primary context after a wholesale `Memory` restore -
/// is brought back into line on its next frame.
pub fn apply(ctx: &egui::Context, cfg: &StyleConfig) {
    let id = egui::Id::new(APPLIED_ID);
    if ctx.data(|d| d.get_temp::<StyleConfig>(id)).as_ref() == Some(cfg) {
        return;
    }
    ctx.set_theme(cfg.theme);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.set_style_of(theme, style_of(cfg, theme));
    }
    ctx.data_mut(|d| d.insert_temp(id, cfg.clone()));
    // The `Ui`s built so far this pass carry the old style.
    ctx.request_repaint();
}

/// Serialize a config as the text of an exported style file.
pub fn to_ron(cfg: &StyleConfig) -> Result<String, ron::Error> {
    ron::ser::to_string_pretty(cfg, ron::ser::PrettyConfig::default())
}

/// Parse the text of an exported style file.
pub fn from_ron(s: &str) -> Result<StyleConfig, ron::de::SpannedError> {
    ron::de::from_str(s)
}

/// The slot holding `theme`'s customisation.
fn slot(cfg: &StyleConfig, theme: egui::Theme) -> &Option<egui::Style> {
    match theme {
        egui::Theme::Dark => &cfg.dark,
        egui::Theme::Light => &cfg.light,
    }
}

/// Whether two styles are equal in everything a user can edit or store.
///
/// `egui::Style`'s own `PartialEq` compares its `number_formatter` callback by
/// `Arc` identity, so styles that are otherwise identical - a deserialized one
/// against a fresh default, say - never compare equal. The formatter is neither
/// editable nor serialized, so normalise it out of the comparison.
pub(crate) fn eq_style(a: &egui::Style, b: &egui::Style) -> bool {
    let mut b = b.clone();
    b.number_formatter = a.number_formatter.clone();
    *a == b
}

#[cfg(test)]
mod tests {
    use super::{StyleConfig, apply, eq_style, from_ron, set_style_of, style_of, to_ron};

    /// A config with both themes customised, distinguishably.
    fn customised() -> StyleConfig {
        let mut cfg = StyleConfig::default();
        for (theme, gap) in [(egui::Theme::Dark, 3.0), (egui::Theme::Light, 7.0)] {
            let mut style = style_of(&cfg, theme);
            style.spacing.item_spacing.x = gap;
            set_style_of(&mut cfg, theme, style);
        }
        cfg
    }

    /// A customised config must survive the RON round-trip used by
    /// export/import and by GUI-state persistence. Guards against an egui bump
    /// breaking `Style`'s serde.
    #[test]
    fn style_config_round_trips_through_ron() {
        let cfg = customised();
        assert!(cfg.dark.is_some() && cfg.light.is_some());
        let text = to_ron(&cfg).expect("serialize StyleConfig");
        assert_eq!(cfg, from_ron(&text).expect("deserialize StyleConfig"));
    }

    /// Storing egui's default style for a theme leaves the config untouched, so
    /// an unmodified theme never bloats persisted state.
    #[test]
    fn default_style_clears_customisation() {
        let mut cfg = StyleConfig::default();
        set_style_of(
            &mut cfg,
            egui::Theme::Dark,
            egui::Theme::Dark.default_style(),
        );
        assert!(cfg.dark.is_none());
    }

    /// Applying installs both themes' styles and the preference, whichever
    /// theme is active - so a pop-out window's fresh context matches the
    /// primary's after a single call.
    #[test]
    fn apply_installs_theme_and_both_styles() {
        let mut cfg = customised();
        cfg.theme = egui::ThemePreference::Light;
        let ctx = egui::Context::default();
        apply(&ctx, &cfg);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            assert!(eq_style(&ctx.style_of(theme), &style_of(&cfg, theme)));
        }
    }

    /// Re-applying an unchanged config is a no-op, so the per-frame call costs
    /// nothing and does not fight a style set elsewhere in the same session.
    #[test]
    fn apply_skips_an_unchanged_config() {
        let mut cfg = customised();
        let ctx = egui::Context::default();
        apply(&ctx, &cfg);

        let mut meddled = ctx.style_of(egui::Theme::Dark).as_ref().clone();
        meddled.spacing.item_spacing.x = 99.0;
        ctx.set_style_of(egui::Theme::Dark, meddled.clone());
        apply(&ctx, &cfg);
        assert!(eq_style(&ctx.style_of(egui::Theme::Dark), &meddled));

        // A changed config applies again, restoring what it specifies.
        cfg.theme = egui::ThemePreference::Light;
        apply(&ctx, &cfg);
        assert!(eq_style(
            &ctx.style_of(egui::Theme::Dark),
            &style_of(&cfg, egui::Theme::Dark)
        ));
    }
}
