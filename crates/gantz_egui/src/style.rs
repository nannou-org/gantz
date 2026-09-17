//! Global egui appearance. The theme preference and the per-theme
//! [`egui::Style`], edited in the Style tab of the settings panel.
//!
//! egui omits its own styles when serializing its `Options`, since
//! `dark_style` and `light_style` are `#[serde(skip)]`. So gantz owns them.
//! [`StyleConfig`] rides [`GantzState`][crate::widget::GantzState], which
//! both hosts already persist. [`apply`] applies it to a context. Owning the
//! preference also keeps pop-out pane windows in step, as each is a separate
//! `egui::Context` with its own options.

/// File extension for exported style files, without the leading dot.
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
    /// The pane separator colours.
    #[serde(default)]
    pub separator: SeparatorConfig,
}

/// The colour and width of the resize handle between panes.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SeparatorConfig {
    #[serde(default = "default_separator_color")]
    pub color: VisualsColor,
    /// While hovered or dragged. `None` uses `color`.
    #[serde(default)]
    pub hover: Option<VisualsColor>,
    #[serde(default = "default_separator_width")]
    pub width: f32,
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            dark: None,
            light: None,
            separator: SeparatorConfig::default(),
        }
    }
}

impl Default for SeparatorConfig {
    fn default() -> Self {
        Self {
            color: default_separator_color(),
            hover: None,
            width: default_separator_width(),
        }
    }
}

impl PartialEq for StyleConfig {
    fn eq(&self, other: &Self) -> bool {
        self.theme == other.theme
            && self.separator == other.separator
            && [egui::Theme::Dark, egui::Theme::Light]
                .into_iter()
                .all(|theme| match (slot(self, theme), slot(other, theme)) {
                    (None, None) => true,
                    (Some(a), Some(b)) => eq_style(a, b),
                    _ => false,
                })
    }
}

/// A colour slot of [`egui::Visuals`], so gantz chrome follows the palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum VisualsColor {
    ExtremeBg,
    FaintBg,
    PanelFill,
    WindowFill,
    NoninteractiveBgStroke,
    WeakText,
    Text,
    HoveredFgStroke,
    Hyperlink,
    Selection,
}

impl VisualsColor {
    pub const ALL: [Self; 10] = [
        Self::ExtremeBg,
        Self::FaintBg,
        Self::PanelFill,
        Self::WindowFill,
        Self::NoninteractiveBgStroke,
        Self::WeakText,
        Self::Text,
        Self::HoveredFgStroke,
        Self::Hyperlink,
        Self::Selection,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ExtremeBg => "Extreme background",
            Self::FaintBg => "Faint background",
            Self::PanelFill => "Panel fill",
            Self::WindowFill => "Window fill",
            Self::NoninteractiveBgStroke => "Noninteractive stroke",
            Self::WeakText => "Weak text",
            Self::Text => "Text",
            Self::HoveredFgStroke => "Hovered text",
            Self::Hyperlink => "Hyperlink",
            Self::Selection => "Selection",
        }
    }

    pub fn resolve(self, visuals: &egui::Visuals) -> egui::Color32 {
        match self {
            Self::ExtremeBg => visuals.extreme_bg_color,
            Self::FaintBg => visuals.faint_bg_color,
            Self::PanelFill => visuals.panel_fill,
            Self::WindowFill => visuals.window_fill,
            Self::NoninteractiveBgStroke => visuals.widgets.noninteractive.bg_stroke.color,
            Self::WeakText => visuals.weak_text_color(),
            Self::Text => visuals.text_color(),
            Self::HoveredFgStroke => visuals.widgets.hovered.fg_stroke.color,
            Self::Hyperlink => visuals.hyperlink_color,
            Self::Selection => visuals.selection.bg_fill,
        }
    }
}

/// gantz's default theme preference.
///
/// Dark rather than egui's `System`. gantz's remaining hand-picked colours are
/// dark-tuned, see #203. `bevy_egui` reports no system theme, so following the
/// system would leave the app and the `gantz_egui` demo disagreeing.
fn default_theme() -> egui::ThemePreference {
    egui::ThemePreference::Dark
}

fn default_separator_color() -> VisualsColor {
    VisualsColor::ExtremeBg
}

fn default_separator_width() -> f32 {
    2.0
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
/// context's temporary data. A context without one is brought back into line
/// on its next frame. A freshly spawned pop-out window is one example. The
/// primary context after a wholesale `Memory` restore is another.
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
/// `Arc` identity, so styles that are otherwise identical never compare equal.
/// A deserialized style against a fresh default is one example. The formatter
/// is neither editable nor serialized, so normalise it out of the comparison.
pub(crate) fn eq_style(a: &egui::Style, b: &egui::Style) -> bool {
    let mut b = b.clone();
    b.number_formatter = a.number_formatter.clone();
    *a == b
}

#[cfg(test)]
mod tests {
    use super::{
        StyleConfig, VisualsColor, apply, eq_style, from_ron, set_style_of, style_of, to_ron,
    };

    #[test]
    fn visuals_color_round_trips_through_ron() {
        for color in VisualsColor::ALL {
            let text = ron::to_string(&color).expect("serialize VisualsColor");
            assert_eq!(
                color,
                ron::from_str(&text).expect("deserialize VisualsColor")
            );
        }
    }

    /// A config with both themes customised, distinguishably.
    fn customised() -> StyleConfig {
        let mut cfg = StyleConfig::default();
        for (theme, gap) in [(egui::Theme::Dark, 3.0), (egui::Theme::Light, 7.0)] {
            let mut style = style_of(&cfg, theme);
            style.spacing.item_spacing.x = gap;
            set_style_of(&mut cfg, theme, style);
        }
        cfg.separator.hover = Some(VisualsColor::Selection);
        cfg
    }

    /// A customised config must survive the RON round-trip used by export,
    /// import and GUI-state persistence. Guards against an egui bump breaking
    /// `Style`'s serde.
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
    /// theme is active. So a pop-out window's fresh context matches the
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
