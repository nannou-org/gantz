//! Global egui appearance, edited in the Style tab of the settings panel.
//!
//! A style has three parts. A [`Palette`] per theme holds the colours. The
//! [`Roles`] select a palette colour for each semantic slot of the UI, and
//! one mapping serves both themes. The [`Shape`] holds the settings that are
//! not colours. [`style`] derives an `egui::Style` from these, so every
//! colour in the UI comes from the palette.
//!
//! egui omits its own styles when serializing its `Options`, since
//! `dark_style` and `light_style` are `#[serde(skip)]`. So gantz owns them.
//! [`StyleConfig`] rides [`GantzState`][crate::widget::GantzState], which
//! both hosts already persist. [`apply`] applies it to a context. Owning the
//! preference also keeps pop-out pane windows in step, as each is a separate
//! `egui::Context` with its own options.

pub use palette::{ColorRef, Palette, Swatch};
pub use role::{Group, Role, Roles};

pub mod palette;
pub mod role;

/// File extension for exported style files, without the leading dot.
pub const FILE_EXTENSION: &str = "ron";

/// The id under which a context tracks the config last applied to it.
const APPLIED_ID: &str = "gantz-applied-style";

/// The number of roles.
const ROLES: usize = Role::ALL.len();

/// The theme preference, palettes, role mapping and shape of the UI.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct StyleConfig {
    /// Whether to use the dark palette, the light palette, or follow the
    /// system.
    #[serde(default = "default_theme")]
    pub theme: egui::ThemePreference,
    #[serde(default)]
    pub palettes: Palettes,
    #[serde(default)]
    pub roles: Roles,
    #[serde(default)]
    pub shape: Shape,
}

/// The palette of each theme.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Palettes {
    pub dark: Palette,
    pub light: Palette,
}

/// The style settings that are not colours.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Shape {
    /// The width of the border between panes. It is also the gap between
    /// them.
    pub separator_width: f32,
}

/// Every role's colour in one theme, as last applied to a context.
#[derive(Clone)]
struct Colors([egui::Color32; ROLES]);

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            palettes: Palettes::default(),
            roles: Roles::default(),
            shape: Shape::default(),
        }
    }
}

impl Default for Palettes {
    fn default() -> Self {
        Self {
            dark: Palette::dark(),
            light: Palette::light(),
        }
    }
}

impl Default for Shape {
    fn default() -> Self {
        Self {
            separator_width: 2.0,
        }
    }
}

impl StyleConfig {
    /// The colour `role` takes under `theme`.
    pub fn resolve(&self, theme: egui::Theme, role: Role) -> egui::Color32 {
        self.roles.resolve(role, self.palettes.get(theme))
    }
}

impl Palettes {
    pub fn get(&self, theme: egui::Theme) -> &Palette {
        match theme {
            egui::Theme::Dark => &self.dark,
            egui::Theme::Light => &self.light,
        }
    }

    pub fn get_mut(&mut self, theme: egui::Theme) -> &mut Palette {
        match theme {
            egui::Theme::Dark => &mut self.dark,
            egui::Theme::Light => &mut self.light,
        }
    }
}

/// gantz's default theme preference.
///
/// Dark rather than egui's `System`. `bevy_egui` reports no system theme, so
/// following the system would leave the app and the `gantz_egui` demo
/// disagreeing.
fn default_theme() -> egui::ThemePreference {
    egui::ThemePreference::Dark
}

/// The egui style that `cfg` specifies for `theme`.
///
/// Settings that the config does not cover keep egui's defaults for the
/// theme.
pub fn style(cfg: &StyleConfig, theme: egui::Theme) -> egui::Style {
    egui::Style {
        visuals: visuals(cfg.palettes.get(theme), &cfg.roles, theme),
        ..theme.default_style()
    }
}

/// The egui visuals for `theme`, with every colour selected from `palette`
/// by `roles`.
///
/// The visuals' `dark_mode` follows `theme`, not the palette's brightness. So
/// [`theme_of`] recovers which palette a style came from.
pub fn visuals(palette: &Palette, roles: &Roles, theme: egui::Theme) -> egui::Visuals {
    use Role::*;
    let c = |role| roles.resolve(role, palette);
    let mut v = theme.default_visuals();
    v.override_text_color = None;
    v.weak_text_color = Some(c(WeakText));
    v.hyperlink_color = c(Hyperlink);
    v.faint_bg_color = c(Faint);
    v.extreme_bg_color = c(Canvas);
    v.text_edit_bg_color = None;
    v.code_bg_color = c(Code);
    v.warn_fg_color = c(Warning);
    v.error_fg_color = c(Error);
    v.window_fill = c(Window);
    v.window_stroke.color = c(Border);
    v.panel_fill = c(Panel);
    v.selection.bg_fill = c(Selection);
    v.selection.stroke.color = c(SelectionStroke);
    v.text_cursor.stroke.color = c(SelectionStroke);

    let fills = |w: &mut egui::style::WidgetVisuals, fill| {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
    };
    let w = &mut v.widgets;
    fills(&mut w.noninteractive, c(Panel));
    w.noninteractive.bg_stroke.color = c(Border);
    w.noninteractive.fg_stroke.color = c(Text);
    fills(&mut w.inactive, c(WidgetFill));
    w.inactive.fg_stroke.color = c(WidgetText);
    fills(&mut w.hovered, c(HoverFill));
    w.hovered.bg_stroke.color = c(HoverStroke);
    w.hovered.fg_stroke.color = c(HoverText);
    fills(&mut w.active, c(ActiveFill));
    w.active.bg_stroke.color = c(StrongText);
    w.active.fg_stroke.color = c(StrongText);
    w.open.bg_fill = c(Panel);
    w.open.weak_bg_fill = c(ActiveFill);
    w.open.bg_stroke.color = c(Border);
    w.open.fg_stroke.color = c(HoverText);
    v
}

/// The theme whose palette `visuals` came from. See [`visuals`].
pub fn theme_of(visuals: &egui::Visuals) -> egui::Theme {
    egui::Theme::from_dark_mode(visuals.dark_mode)
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
        ctx.set_style_of(theme, style(cfg, theme));
        let colors = Colors(std::array::from_fn(|i| cfg.resolve(theme, Role::ALL[i])));
        ctx.data_mut(|d| d.insert_temp(colors_id(theme), colors));
    }
    ctx.data_mut(|d| d.insert_temp(id, cfg.clone()));
    // The `Ui`s built so far this pass carry the old style.
    ctx.request_repaint();
}

/// The colour of `role` in the style of `ui`.
///
/// Prefer `ui.visuals()` for a role that egui's visuals carry. This serves the
/// roles that only gantz uses.
pub fn color(ui: &egui::Ui, role: Role) -> egui::Color32 {
    color_of(ui.ctx(), theme_of(ui.visuals()), role)
}

/// The colour of `role` under `theme` in the config last applied to `ctx`, or
/// in the default config before any is applied.
pub fn color_of(ctx: &egui::Context, theme: egui::Theme, role: Role) -> egui::Color32 {
    ctx.data(|d| d.get_temp::<Colors>(colors_id(theme)))
        .map(|Colors(colors)| colors[role as usize])
        .unwrap_or_else(|| StyleConfig::default().resolve(theme, role))
}

/// The WCAG contrast ratio of `fg` drawn over the opaque `bg`, from 1 to 21.
pub fn contrast(fg: egui::Color32, bg: egui::Color32) -> f32 {
    let (a, b) = (luminance(over(fg, bg)), luminance(bg));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Serialize a config as the text of an exported style file.
pub fn to_ron(cfg: &StyleConfig) -> Result<String, ron::Error> {
    ron::ser::to_string_pretty(cfg, ron::ser::PrettyConfig::default())
}

/// Parse the text of an exported style file.
pub fn from_ron(s: &str) -> Result<StyleConfig, ron::de::SpannedError> {
    ron::de::from_str(s)
}

/// The relative luminance of an opaque colour.
fn luminance(color: egui::Color32) -> f32 {
    let linear = egui::Rgba::from(color);
    0.2126 * linear.r() + 0.7152 * linear.g() + 0.0722 * linear.b()
}

/// The premultiplied `fg` composited over the opaque `bg`.
fn over(fg: egui::Color32, bg: egui::Color32) -> egui::Color32 {
    let a = f32::from(fg.a()) / 255.0;
    let mix = |f: u8, b: u8| (f32::from(f) + f32::from(b) * (1.0 - a)).round().min(255.0) as u8;
    egui::Color32::from_rgb(
        mix(fg.r(), bg.r()),
        mix(fg.g(), bg.g()),
        mix(fg.b(), bg.b()),
    )
}

fn colors_id(theme: egui::Theme) -> egui::Id {
    egui::Id::new(APPLIED_ID).with(theme)
}

#[cfg(test)]
mod tests {
    use super::{Role, StyleConfig, Swatch, apply, color_of, contrast, from_ron, style, to_ron};

    /// A config with both palettes and a role customised.
    fn customised() -> StyleConfig {
        let mut cfg = StyleConfig::default();
        cfg.palettes.dark.neutrals[3] = egui::Color32::from_rgb(1, 2, 3);
        cfg.palettes.light.accents[0] = egui::Color32::from_rgb(4, 5, 6);
        cfg.roles.text.swatch = Swatch::N6;
        cfg.roles.weak_text.opacity = 0.3;
        cfg.shape.separator_width = 5.0;
        cfg
    }

    /// A customised config must survive the RON round-trip used by export,
    /// import and GUI-state persistence.
    #[test]
    fn style_config_round_trips_through_ron() {
        let cfg = customised();
        let text = to_ron(&cfg).expect("serialize StyleConfig");
        assert_eq!(cfg, from_ron(&text).expect("deserialize StyleConfig"));
    }

    /// A config stored before palettes existed still loads. It keeps its theme
    /// preference and takes defaults for the rest, so the GUI state that holds
    /// it is not lost.
    #[test]
    fn legacy_config_loads_with_defaults() {
        let legacy = "(theme: Light, dark: Some((override_text_style: None)), light: None, \
                      separator: (color: ExtremeBg, hover: None, width: 2.0))";
        let cfg = from_ron(legacy).expect("parse legacy StyleConfig");
        let expected = StyleConfig {
            theme: egui::ThemePreference::Light,
            ..StyleConfig::default()
        };
        assert_eq!(cfg, expected);
    }

    /// The derived visuals take their colours from the theme's palette.
    #[test]
    fn visuals_follow_the_palette() {
        let cfg = customised();
        let dark = style(&cfg, egui::Theme::Dark).visuals;
        assert!(dark.dark_mode);
        assert_eq!(dark.text_color(), cfg.palettes.dark.get(Swatch::N6));
        let light = style(&cfg, egui::Theme::Light).visuals;
        assert!(!light.dark_mode);
        assert_eq!(light.error_fg_color, egui::Color32::from_rgb(4, 5, 6));
    }

    /// Applying installs both themes' styles and colours and the preference,
    /// whichever theme is active. So a pop-out window's fresh context matches
    /// the primary's after a single call.
    #[test]
    fn apply_installs_theme_styles_and_colors() {
        let mut cfg = customised();
        cfg.theme = egui::ThemePreference::Light;
        let ctx = egui::Context::default();
        apply(&ctx, &cfg);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            assert_eq!(ctx.style_of(theme).visuals, style(&cfg, theme).visuals);
            for &role in Role::ALL {
                assert_eq!(color_of(&ctx, theme, role), cfg.resolve(theme, role));
            }
        }
    }

    #[test]
    fn contrast_spans_one_to_twenty_one() {
        let (black, white) = (egui::Color32::BLACK, egui::Color32::WHITE);
        assert!((contrast(white, black) - 21.0).abs() < 0.01);
        assert!((contrast(black, white) - 21.0).abs() < 0.01);
        assert!((contrast(white, white) - 1.0).abs() < 0.01);
        // A transparent colour vanishes into its backdrop.
        assert!((contrast(egui::Color32::TRANSPARENT, black) - 1.0).abs() < 0.01);
    }

    /// The default style passes every contrast check of the style editor in
    /// both themes.
    #[test]
    fn default_style_is_readable() {
        let cfg = StyleConfig::default();
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for &role in Role::ALL {
                let Some(backdrop) = role.backdrop() else {
                    continue;
                };
                let fg = cfg.resolve(theme, role);
                let bg = cfg.resolve(theme, backdrop);
                let ratio = contrast(fg, bg);
                assert!(ratio >= role.min_contrast(), "{theme:?} {role:?} {ratio}");
            }
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
        assert_eq!(ctx.style_of(egui::Theme::Dark).spacing, meddled.spacing);

        // A changed config applies again, restoring what it specifies.
        cfg.theme = egui::ThemePreference::Light;
        apply(&ctx, &cfg);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).spacing,
            style(&cfg, egui::Theme::Dark).spacing
        );
    }
}
