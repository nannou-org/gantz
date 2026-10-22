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

pub mod base16;
pub mod font;
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

/// The palette of each theme, and the palettes the user saved.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Palettes {
    pub dark: Palette,
    pub light: Palette,
    /// The saved palettes, sorted by name. None shares a name with a preset.
    pub saved: Vec<Palette>,
}

/// Why a palette cannot be saved under a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SaveError {
    #[error("A saved palette needs a name.")]
    NoName,
    #[error("Presets cannot be changed. Choose another name.")]
    Preset,
}

/// The style settings that are not colours.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Shape {
    /// The gap between panes, where the scene shows through. Dragging it
    /// resizes the panes on either side.
    pub separator_width: f32,
    /// The margin between the outer panes and the edge of the window.
    pub window_margin: f32,
    /// The corner radius of panes, windows, menus and popups.
    pub pane_radius: u8,
    /// The corner radius of group cards inside panes.
    pub card_radius: u8,
    /// The corner radius of buttons, fields, pills and other controls. A
    /// control is never rounder than a capsule.
    pub control_radius: u8,
    /// The corner radius of nodes in the graph. A node is never rounder than
    /// half its smallest height.
    pub node_radius: u8,
    /// The space between a button's edge and its text, across and down.
    pub button_padding: egui::Vec2,
    /// How a group of settings, or a node's entry in the inspector, stands
    /// apart from the pane around it.
    pub cards: CardStyle,
    /// The width of the outline around each pane. 0 draws none. An outline
    /// suits a palette whose panes share the scene's colour.
    pub pane_outline: f32,
    /// How a graph pane frames its graph's scene.
    pub graph_panes: GraphPaneStyle,
    /// A scale on the gaps and padding between and within widgets.
    pub spacing: f32,
    /// The font of all text.
    pub font: font::Font,
    /// The size of body text in points. The other text styles keep their
    /// size relative to it.
    pub text_size: f32,
    /// Whether windows and popups cast a shadow.
    pub shadows: bool,
}

/// How a group card stands apart from the pane around it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum CardStyle {
    /// A shaded card, one step from the pane.
    #[default]
    Nested,
    /// An outlined card on the pane's own fill.
    Outline,
}

/// How a graph pane frames its graph's scene.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum GraphPaneStyle {
    /// The scene runs up under the pane's title, inside the pane's frame.
    #[default]
    Open,
    /// The scene sits below the pane's title, as in other panes.
    Framed,
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
            saved: vec![],
        }
    }
}

impl Default for Shape {
    fn default() -> Self {
        Self {
            separator_width: 8.0,
            window_margin: 8.0,
            pane_radius: 16,
            card_radius: 10,
            control_radius: 8,
            node_radius: 10,
            button_padding: egui::vec2(8.0, 2.0),
            cards: CardStyle::Nested,
            pane_outline: 0.0,
            graph_panes: GraphPaneStyle::Open,
            spacing: 1.0,
            font: font::Font::PlexMono,
            text_size: 13.0,
            shadows: true,
        }
    }
}

impl Shape {
    /// The corner radius of a control `height` points tall: the control
    /// radius, but never rounder than a capsule.
    pub fn widget_corner(&self, height: f32) -> f32 {
        f32::from(self.control_radius).min(height / 2.0)
    }

    /// The corner radius of panes, windows, menus and popups.
    pub fn window_corner(&self) -> f32 {
        f32::from(self.pane_radius)
    }

    /// The corner radius of a group card inside a pane.
    pub fn card_corner(&self) -> f32 {
        f32::from(self.card_radius)
    }

    /// The corner radius of a node whose smallest height is `min_height`.
    pub fn node_corner(&self, min_height: f32) -> f32 {
        f32::from(self.node_radius).min(min_height / 2.0)
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

    /// The preset or saved palette called `name`.
    ///
    /// A palette in use keeps the name of the palette it was loaded from or
    /// saved as. So this is the version that a reset restores, and that an
    /// edited palette differs from.
    pub fn named(&self, name: &str) -> Option<Palette> {
        match base16::preset(name) {
            Some(scheme) => Some(Palette::from(scheme)),
            None => self.saved.iter().find(|p| p.name == name).cloned(),
        }
    }

    /// Whether `name` may name a saved palette.
    pub fn check_name(name: &str) -> Result<(), SaveError> {
        if name.trim().is_empty() {
            Err(SaveError::NoName)
        } else if base16::preset(name.trim()).is_some() {
            Err(SaveError::Preset)
        } else {
            Ok(())
        }
    }

    /// Name the palette of `theme` and save a copy of it. A saved palette of
    /// the same name is replaced.
    pub fn save(&mut self, theme: egui::Theme, name: &str) -> Result<(), SaveError> {
        Self::check_name(name)?;
        let palette = self.get_mut(theme);
        palette.name = name.trim().to_string();
        let palette = palette.clone();
        match self.saved.binary_search_by(|p| p.name.cmp(&palette.name)) {
            Ok(i) => self.saved[i] = palette,
            Err(i) => self.saved.insert(i, palette),
        }
        Ok(())
    }

    /// Remove the saved palette called `name`. The palettes in use are kept.
    pub fn delete(&mut self, name: &str) {
        self.saved.retain(|p| p.name != name);
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
    let style = egui::Style {
        visuals: visuals(cfg.palettes.get(theme), &cfg.roles, theme),
        ..theme.default_style()
    };
    shaped(style, &cfg.shape)
}

/// `style` with the corners, spacing, text size and shadows of `shape`.
fn shaped(mut style: egui::Style, shape: &Shape) -> egui::Style {
    let k = shape.spacing;
    let widget = corner(shape.widget_corner(style.spacing.interact_size.y * k));
    let window = corner(shape.window_corner());
    let v = &mut style.visuals;
    let w = &mut v.widgets;
    for w in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        w.corner_radius = widget;
    }
    v.window_corner_radius = window;
    v.menu_corner_radius = window;
    if !shape.shadows {
        v.window_shadow = egui::Shadow::NONE;
        v.popup_shadow = egui::Shadow::NONE;
    }

    let s = &mut style.spacing;
    s.item_spacing *= k;
    s.button_padding = shape.button_padding * k;
    s.interact_size *= k;
    s.indent *= k;
    s.icon_spacing *= k;
    s.menu_spacing *= k;
    s.window_margin = s.window_margin * k;
    s.menu_margin = s.menu_margin * k;
    // A scroll fade paints the pane's colour over the cards in a pane, which
    // reads as a shadow, so scroll areas end crisply instead.
    s.scroll.fade.strength = 0.0;

    if let Some(body) = style.text_styles.get(&egui::TextStyle::Body) {
        let k = shape.text_size / body.size;
        for font in style.text_styles.values_mut() {
            font.size *= k;
        }
    }
    style
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
    // egui_graph fills the scene with the noninteractive fill.
    fills(&mut w.noninteractive, c(Scene));
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
    font::install(ctx, cfg.shape.font);
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
    ctx.data_mut(|d| d.insert_temp(shape_id(), cfg.shape));
    ctx.data_mut(|d| d.insert_temp(id, cfg.clone()));
    // The `Ui`s built so far this pass carry the old style.
    ctx.request_repaint();
}

/// The shape last applied to `ctx`, or the default shape before any is
/// applied.
pub fn shape(ctx: &egui::Context) -> Shape {
    ctx.data(|d| d.get_temp::<Shape>(shape_id()))
        .unwrap_or_default()
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

/// The contrast of `role` over its [`Role::backdrop`], with the backdrop it
/// was measured over. A translucent backdrop, such as a widget fill, is first
/// composited over the pane it usually sits on.
pub fn role_contrast(roles: &Roles, role: Role, palette: &Palette) -> Option<(f32, Role)> {
    let backdrop = role.backdrop()?;
    let fg = roles.resolve(role, palette);
    let pane = roles.resolve(Role::Panel, palette);
    let bg = over(roles.resolve(backdrop, palette), pane);
    Some((contrast(fg, bg), backdrop))
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
pub(crate) fn luminance(color: egui::Color32) -> f32 {
    let linear = egui::Rgba::from(color);
    0.2126 * linear.r() + 0.7152 * linear.g() + 0.0722 * linear.b()
}

/// The premultiplied `fg` composited over the opaque `bg`.
pub fn over(fg: egui::Color32, bg: egui::Color32) -> egui::Color32 {
    let a = f32::from(fg.a()) / 255.0;
    let mix = |f: u8, b: u8| (f32::from(f) + f32::from(b) * (1.0 - a)).round().min(255.0) as u8;
    egui::Color32::from_rgb(
        mix(fg.r(), bg.r()),
        mix(fg.g(), bg.g()),
        mix(fg.b(), bg.b()),
    )
}

/// A uniform corner radius, rounded to the whole points egui stores.
fn corner(radius: f32) -> egui::CornerRadius {
    egui::CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8)
}

fn shape_id() -> egui::Id {
    egui::Id::new(APPLIED_ID).with("shape")
}

fn colors_id(theme: egui::Theme) -> egui::Id {
    egui::Id::new(APPLIED_ID).with(theme)
}

#[cfg(test)]
mod tests {
    use super::{
        Palettes, Role, SaveError, StyleConfig, Swatch, apply, color_of, contrast, from_ron,
        role_contrast, style, to_ron,
    };

    /// A config with both palettes and a role customised.
    fn customised() -> StyleConfig {
        let mut cfg = StyleConfig::default();
        cfg.palettes.dark.neutrals[3] = egui::Color32::from_rgb(1, 2, 3);
        cfg.palettes.light.accents[0] = egui::Color32::from_rgb(4, 5, 6);
        cfg.roles.text.swatch = Swatch::N6;
        cfg.roles.weak_text.opacity = 0.3;
        cfg.shape.separator_width = 5.0;
        cfg.palettes
            .save(egui::Theme::Dark, "Mine")
            .expect("save a palette");
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
            let palette = cfg.palettes.get(theme);
            for &role in Role::ALL {
                let Some((ratio, _)) = role_contrast(&cfg.roles, role, palette) else {
                    continue;
                };
                assert!(ratio >= role.min_contrast(), "{theme:?} {role:?} {ratio}");
            }
        }
    }

    /// Saving names the palette in use and keeps a copy, which a later save
    /// under the same name replaces. Presets stay read-only.
    #[test]
    fn saved_palettes_track_their_name() {
        let mut palettes = Palettes::default();
        let preset = palettes.dark.clone();
        assert_eq!(palettes.named(&preset.name), Some(preset));
        assert_eq!(
            palettes.save(egui::Theme::Dark, "  "),
            Err(SaveError::NoName)
        );
        assert_eq!(
            palettes.save(egui::Theme::Dark, "egui Light"),
            Err(SaveError::Preset)
        );

        palettes.dark.accents[0] = egui::Color32::BLACK;
        palettes
            .save(egui::Theme::Dark, " Mine ")
            .expect("save a palette");
        assert_eq!(palettes.dark.name, "Mine");
        assert_eq!(palettes.named("Mine").as_ref(), Some(&palettes.dark));

        palettes.dark.accents[0] = egui::Color32::WHITE;
        palettes
            .save(egui::Theme::Dark, "Mine")
            .expect("save it again");
        palettes
            .save(egui::Theme::Dark, "Another")
            .expect("save another");
        let names: Vec<_> = palettes.saved.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Another", "Mine"]);
        assert_eq!(
            palettes.named("Mine").map(|p| p.accents[0]),
            Some(egui::Color32::WHITE)
        );

        palettes.delete("Mine");
        assert_eq!(palettes.named("Mine"), None);
        assert_eq!(palettes.dark.name, "Another");
    }

    /// The default shape keeps egui's default spacing, but for the button
    /// padding and the scroll fade, and its text sizes.
    #[test]
    fn default_shape_keeps_egui_metrics() {
        let cfg = StyleConfig::default();
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let (ours, mut egui) = (style(&cfg, theme), theme.default_style());
            egui.spacing.button_padding = cfg.shape.button_padding;
            egui.spacing.scroll.fade = ours.spacing.scroll.fade;
            assert_eq!(ours.spacing, egui.spacing);
            assert_eq!(ours.text_styles, egui.text_styles);
            assert_eq!(ours.visuals.window_shadow, egui.visuals.window_shadow);
        }
    }

    /// The shape scales spacing and every text style, rounds corners and can
    /// drop shadows.
    #[test]
    fn shape_applies_to_the_style() {
        let mut cfg = StyleConfig::default();
        cfg.shape.spacing = 2.0;
        cfg.shape.text_size = 26.0;
        cfg.shape.control_radius = 5;
        cfg.shape.pane_radius = 9;
        cfg.shape.shadows = false;
        let base = egui::Theme::Dark.default_style();
        let shaped = style(&cfg, egui::Theme::Dark);
        assert_eq!(shaped.spacing.item_spacing, base.spacing.item_spacing * 2.0);
        for (text_style, font) in &shaped.text_styles {
            assert_eq!(font.size, base.text_styles[text_style].size * 2.0);
        }
        let v = &shaped.visuals;
        assert_eq!(v.widgets.hovered.corner_radius, egui::CornerRadius::same(5));
        assert_eq!(v.window_corner_radius, egui::CornerRadius::same(9));
        assert_eq!(v.popup_shadow, egui::Shadow::NONE);
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
