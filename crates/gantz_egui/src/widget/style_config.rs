//! The "Style" settings subtab. The theme, palette and role mapping, plus the
//! visual configuration of the graph scene. See [`crate::style`].
//!
//! The palette section edits the palette of the selected theme. Each role
//! picks a palette swatch through a popup, so a style always draws from its
//! palette. Hovering a swatch highlights the roles that use it.
//!
//! The subtab also hosts the dot-grid, edge and pane separator controls. The
//! grid step feeds snap-to-grid, so it stays editable even when the grid is
//! hidden.

use super::{
    gantz::{EdgeConfig, GridConfig},
    section,
};
use crate::{
    StyleConfig,
    style::{
        ColorRef, Group, Palette, Role, Roles, Swatch,
        palette::{from_hex, to_hex},
    },
};

/// Response from [`style_config`].
#[derive(Default)]
pub struct StyleConfigResponse {
    /// The "Export" button was clicked.
    pub export: bool,
    /// The "Import" button was clicked.
    pub import: bool,
}

/// The side of a palette swatch button, as a multiple of the interact height.
const SWATCH_SCALE: f32 = 1.3;

/// Render the style configuration controls. `style`, `grid` and `edge` are
/// mutated in place. All apply to the whole UI, including all open heads.
pub fn style_config(
    style: &mut StyleConfig,
    grid: &mut GridConfig,
    edge: &mut EdgeConfig,
    ui: &mut egui::Ui,
) -> StyleConfigResponse {
    let mut res = StyleConfigResponse::default();
    section(ui, "Theme", |ui| {
        style.theme.radio_buttons(ui);
        ui.horizontal(|ui| {
            res.export = ui
                .button("Export…")
                .on_hover_text("Save the palettes, roles and shape to a file.")
                .clicked();
            res.import = ui
                .button("Import…")
                .on_hover_text("Load the palettes, roles and shape from an exported file.")
                .clicked();
        });
    });

    // The palette section edits the selected theme's palette. `ctx.theme()`
    // only reflects a preference change on the next frame, so resolve
    // directly.
    let theme = match style.theme {
        egui::ThemePreference::Dark => egui::Theme::Dark,
        egui::ThemePreference::Light => egui::Theme::Light,
        egui::ThemePreference::System => ui.ctx().theme(),
    };
    let title = match theme {
        egui::Theme::Dark => "Dark palette",
        egui::Theme::Light => "Light palette",
    };
    let hovered = section(ui, title, |ui| {
        let palette = style.palettes.get_mut(theme);
        let hovered = palette_editor(palette, ui);
        if ui
            .button("Reset palette")
            .on_hover_text("Restore the default palette of this theme.")
            .clicked()
        {
            *palette = match theme {
                egui::Theme::Dark => Palette::dark(),
                egui::Theme::Light => Palette::light(),
            };
        }
        hovered
    });

    section(ui, "Roles", |ui| {
        ui.label(
            egui::RichText::new("Each role selects its colour from the palette.")
                .small()
                .weak(),
        );
        roles_editor(&mut style.roles, style.palettes.get(theme), hovered, ui);
        if ui
            .button("Reset roles")
            .on_hover_text("Restore the default palette colour of every role.")
            .clicked()
        {
            style.roles = Roles::default();
        }
    });

    section(ui, "Grid", |ui| {
        ui.checkbox(&mut grid.show, "Show grid")
            .on_hover_text("Draw the dot grid behind the graph.");
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut grid.step)
                    .speed(0.5)
                    .range(1.0..=500.0)
                    .suffix(" px"),
            )
            .on_hover_text(
                "Base spacing of the dot grid, in graph-space units. Snap-to-grid \
                 uses a fraction of this (see Global > Snap), so it applies even \
                 when the grid is hidden.",
            );
            ui.label("Grid step");
        });
    });

    section(ui, "Edges", |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut edge.curvature, 0.0..=1.0))
                .on_hover_text("How far each edge curves. 0 draws straight lines.");
            ui.label("Curvature");
        });
    });

    section(ui, "Pane separator", |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut style.shape.separator_width)
                    .speed(0.1)
                    .range(0.5..=16.0)
                    .suffix(" px"),
            )
            .on_hover_text(
                "The border width. It is also the gap between panes. The Chrome \
                 roles set its colours.",
            );
            ui.label("Width");
        });
    });

    res
}

/// The palette's name and swatches. Returns the swatch under the pointer.
fn palette_editor(palette: &mut Palette, ui: &mut egui::Ui) -> Option<Swatch> {
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut palette.name);
    });
    // Shrink the swatches to keep each row of eight on one line.
    let gap = ui.spacing().item_spacing.x;
    let fit = (ui.available_width() - gap * 7.0) / 8.0;
    let side = fit.min(ui.spacing().interact_size.y * SWATCH_SCALE);
    let mut hovered = None;
    for (label, swatches) in [
        ("Neutrals", &Swatch::NEUTRALS[..]),
        ("Accents", &Swatch::ACCENTS[..]),
    ] {
        ui.label(egui::RichText::new(label).small().weak());
        ui.horizontal_wrapped(|ui| {
            for &swatch in swatches {
                let resp = swatch_edit(ui, swatch, side, palette.get_mut(swatch));
                if resp.hovered() {
                    hovered = Some(swatch);
                }
            }
        });
    }
    hovered
}

/// A swatch button that opens a colour picker for `color`.
fn swatch_edit(
    ui: &mut egui::Ui,
    swatch: Swatch,
    side: f32,
    color: &mut egui::Color32,
) -> egui::Response {
    let resp = swatch_button(ui, *color, side, false).on_hover_text(format!(
        "{} {}",
        swatch.label(),
        to_hex(*color)
    ));
    egui::Popup::menu(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().slider_width = 200.0;
            ui.horizontal(|ui| {
                ui.strong(swatch.label());
                hex_edit(ui, resp.id.with("hex"), color);
            });
            egui::color_picker::color_picker_color32(ui, color, egui::color_picker::Alpha::Opaque);
        });
    resp
}

/// A text field for `color` as `#rrggbb`. Partial input is kept while the
/// field has focus, and applies once it parses.
fn hex_edit(ui: &mut egui::Ui, id: egui::Id, color: &mut egui::Color32) {
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| to_hex(*color));
    let edit = egui::TextEdit::singleline(&mut text)
        .id(id)
        .desired_width(ui.spacing().interact_size.x * 1.5)
        .font(egui::TextStyle::Monospace);
    let resp = ui
        .add(edit)
        .on_hover_text("The colour as hex. Paste a colour here.");
    if resp.changed()
        && let Some(parsed) = from_hex(&text)
    {
        *color = parsed;
    }
    if resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
}

/// The roles in one grid, under a heading row per group. A row whose swatch
/// is `hovered` is highlighted.
fn roles_editor(roles: &mut Roles, palette: &Palette, hovered: Option<Swatch>, ui: &mut egui::Ui) {
    egui::Grid::new("style_roles")
        .num_columns(2)
        .show(ui, |ui| {
            for group in Group::ALL {
                ui.label(egui::RichText::new(group.label()).small().weak());
                ui.end_row();
                for &role in Role::ALL.iter().filter(|r| r.group() == group) {
                    role_row(roles, role, palette, hovered, ui);
                }
            }
        });
}

/// One role's label and swatch picker. A contrast warning follows when the
/// role is hard to read over its backdrop.
fn role_row(
    roles: &mut Roles,
    role: Role,
    palette: &Palette,
    hovered: Option<Swatch>,
    ui: &mut egui::Ui,
) {
    let contrast = role.backdrop().map(|backdrop| {
        let fg = roles.resolve(role, palette);
        let bg = roles.resolve(backdrop, palette);
        (crate::style::contrast(fg, bg), backdrop)
    });
    let min = role.min_contrast();
    let mut label = egui::RichText::new(role.label());
    if hovered == Some(roles.get(role).swatch) {
        label = label
            .strong()
            .background_color(ui.visuals().selection.bg_fill);
    }
    let help = match contrast {
        Some((ratio, backdrop)) => format!(
            "{}\n\nContrast over {} is {ratio:.1}:1. It reads well at {min}:1 or more.",
            role.help(),
            backdrop.label(),
        ),
        None => role.help().to_string(),
    };
    ui.label(label).on_hover_text(help);
    ui.horizontal(|ui| {
        color_ref_edit(ui, role, roles.get_mut(role), palette);
        if let Some((ratio, backdrop)) = contrast
            && ratio < min
        {
            let color = if ratio >= min * 0.75 {
                ui.visuals().warn_fg_color
            } else {
                ui.visuals().error_fg_color
            };
            ui.label(
                egui::RichText::new(format!("{ratio:.1}:1"))
                    .small()
                    .color(color),
            )
            .on_hover_text(format!(
                "Low contrast over {}. It reads well at {min}:1 or more.",
                backdrop.label(),
            ));
        }
    });
    ui.end_row();
}

/// A button showing `color_ref`, opening a popup to pick its swatch and
/// opacity.
fn color_ref_edit(ui: &mut egui::Ui, role: Role, color_ref: &mut ColorRef, palette: &Palette) {
    let text = if color_ref.opacity < 1.0 {
        let percent = color_ref.opacity * 100.0;
        format!("{} {percent:.0}%", color_ref.swatch.label())
    } else {
        color_ref.swatch.label().to_string()
    };
    let resp = chip_button(ui, color_ref.resolve(palette), text)
        .on_hover_text(format!("Pick the palette colour for {}.", role.label()));
    let side = ui.spacing().interact_size.y * SWATCH_SCALE;
    egui::Popup::menu(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            for swatches in [&Swatch::NEUTRALS[..], &Swatch::ACCENTS[..]] {
                ui.horizontal(|ui| {
                    for &swatch in swatches {
                        let color = palette.get(swatch);
                        let selected = color_ref.swatch == swatch;
                        if swatch_button(ui, color, side, selected)
                            .on_hover_text(format!("{} {}", swatch.label(), to_hex(color)))
                            .clicked()
                        {
                            color_ref.swatch = swatch;
                        }
                    }
                });
            }
            let mut percent = color_ref.opacity * 100.0;
            let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
                .suffix("%")
                .fixed_decimals(0)
                .text("Opacity");
            if ui.add(slider).changed() {
                color_ref.opacity = percent / 100.0;
            }
        });
}

/// A square button filled with `color`. A `selected` swatch has a strong
/// outline.
fn swatch_button(
    ui: &mut egui::Ui,
    color: egui::Color32,
    side: f32,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        paint_chip(ui, rect, color, selected || resp.hovered());
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A button of a colour chip followed by `text`.
fn chip_button(ui: &mut egui::Ui, color: egui::Color32, text: String) -> egui::Response {
    let side = ui.spacing().interact_size.y;
    let gap = ui.spacing().item_spacing.x;
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let size = egui::vec2(side + gap + galley.size().x, side);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let chip = egui::Rect::from_min_size(rect.min, egui::Vec2::splat(side));
        paint_chip(ui, chip, color, resp.hovered());
        let y = rect.center().y - galley.size().y / 2.0;
        let text_color = ui.style().interact(&resp).text_color();
        ui.painter()
            .galley(egui::pos2(chip.max.x + gap, y), galley, text_color);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Fill `rect` with `color` and outline it. A `highlighted` chip has a strong
/// outline.
fn paint_chip(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32, highlighted: bool) {
    let visuals = ui.visuals();
    let stroke = if highlighted {
        egui::Stroke::new(2.0f32, visuals.strong_text_color())
    } else {
        visuals.widgets.noninteractive.bg_stroke
    };
    egui::color_picker::show_color_at(ui.painter(), color, rect);
    ui.painter().rect_stroke(
        rect,
        visuals.widgets.inactive.corner_radius,
        stroke,
        egui::StrokeKind::Inside,
    );
}
