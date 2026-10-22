//! The "Style" settings subtab. The theme, palette and role mapping, plus the
//! visual configuration of the graph scene. See [`crate::style`].
//!
//! The palette section edits the palette of the selected theme. A palette
//! keeps the name of the preset or saved palette it came from, and the resets
//! restore that version. Each role picks a palette swatch through a popup, so
//! a style always draws from its palette. Hovering a swatch highlights the
//! roles that use it.
//!
//! The subtab also hosts the shape, dot-grid and edge controls. The
//! grid step feeds snap-to-grid, so it stays editable even when the grid is
//! hidden.

use super::{
    gantz::{EdgeConfig, GridConfig},
    section,
};
use crate::{
    StyleConfig,
    style::{
        CardStyle, ColorRef, GraphPaneStyle, Group, Palette, Palettes, Role, Roles, Shape, Swatch,
        base16,
        font::Font,
        palette::{from_hex, to_hex},
    },
};

/// Response from [`style_config`].
#[derive(Default)]
pub struct StyleConfigResponse {
    /// The "Export style" button was clicked.
    pub export: bool,
    /// The "Import style" button was clicked.
    pub import: bool,
    /// The "Import base16" button was clicked for the palette of this theme.
    pub import_palette: Option<egui::Theme>,
    /// The "Export base16" button was clicked for this palette.
    pub export_palette: Option<Palette>,
}

/// A line about the palette's saved state.
enum Status {
    Info(String),
    Edited(String),
    Error(String),
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
        ui.horizontal_wrapped(|ui| {
            res.export = ui
                .button("Export style…")
                .on_hover_text("Save the palettes, roles and shape to a file.")
                .clicked();
            res.import = ui
                .button("Import style…")
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
        palette_section(&mut style.palettes, theme, &mut res, ui)
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

    section(ui, "Shape", |ui| {
        shape_editor(&mut style.shape, ui);
        if ui
            .button("Reset shape")
            .on_hover_text("Restore the default shape.")
            .clicked()
        {
            style.shape = Shape::default();
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

    res
}

/// The palette of `theme`. Its menu, name, swatches and file actions. Returns
/// the swatch under the pointer.
fn palette_section(
    palettes: &mut Palettes,
    theme: egui::Theme,
    res: &mut StyleConfigResponse,
    ui: &mut egui::Ui,
) -> Option<Swatch> {
    palette_menu(palettes, theme, ui);
    name_edit(palettes, theme, ui);
    let name = palettes.get(theme).name.clone();
    let origin = palettes.named(&name);
    let hovered = swatches_editor(palettes.get_mut(theme), origin.as_ref(), ui);
    ui.horizontal_wrapped(|ui| {
        let reset_hover = match origin {
            Some(_) => format!("Restore \"{name}\"."),
            None => "Restore the default palette of this theme.".to_string(),
        };
        if ui
            .button("Reset palette")
            .on_hover_text(reset_hover)
            .clicked()
        {
            *palettes.get_mut(theme) = origin.unwrap_or_else(|| match theme {
                egui::Theme::Dark => Palette::dark(),
                egui::Theme::Light => Palette::light(),
            });
        }
        let saved = palettes.saved.iter().any(|p| p.name == name);
        if ui
            .add_enabled(saved, egui::Button::new("Delete"))
            .on_hover_text(format!(
                "Remove \"{name}\" from the saved palettes. The palette in use stays."
            ))
            .on_disabled_hover_text("Only a saved palette can be deleted.")
            .clicked()
        {
            palettes.delete(&name);
        }
        if ui
            .button("Import base16…")
            .on_hover_text(
                "Load this palette from a base16 or base24 scheme file. The neutrals \
                 take base00 to base07 and the accents take base08 to base0F.",
            )
            .clicked()
        {
            res.import_palette = Some(theme);
        }
        if ui
            .button("Export base16…")
            .on_hover_text("Save this palette to a base16 scheme file.")
            .clicked()
        {
            res.export_palette = Some(palettes.get(theme).clone());
        }
    });
    hovered
}

/// A menu of the presets and saved palettes. Picking one replaces the palette
/// of `theme`. The selected text flags a palette that differs from the
/// version it is named after.
fn palette_menu(palettes: &mut Palettes, theme: egui::Theme, ui: &mut egui::Ui) {
    let palette = palettes.get(theme);
    let selected = match palettes.named(&palette.name) {
        Some(origin) if origin != *palette => format!("{} (edited)", palette.name),
        Some(_) => palette.name.clone(),
        None => format!("{} (unsaved)", palette.name),
    };
    let mut pick = None;
    egui::ComboBox::from_id_salt("style_palette_menu")
        .selected_text(selected)
        .width(ui.available_width())
        .height(f32::INFINITY)
        .show_ui(ui, |ui| {
            let current = &palettes.get(theme).name;
            if !palettes.saved.is_empty() {
                ui.label(egui::RichText::new("Saved").small().weak());
            }
            for saved in &palettes.saved {
                if ui
                    .selectable_label(*current == saved.name, &saved.name)
                    .clicked()
                {
                    pick = Some(saved.clone());
                }
            }
            if !palettes.saved.is_empty() {
                ui.separator();
            }
            ui.label(egui::RichText::new("Presets").small().weak());
            for scheme in base16::PRESETS {
                if ui
                    .selectable_label(*current == scheme.name, scheme.name)
                    .clicked()
                {
                    pick = Some(Palette::from(scheme));
                }
            }
        })
        .response
        .on_hover_text("Load a preset or a saved palette.");
    if let Some(palette) = pick {
        *palettes.get_mut(theme) = palette;
    }
}

/// The palette's name. Typing holds a draft name. Enter or "Save" saves the
/// palette under it, and Escape drops it. A line below tells what a save
/// would do, or why it cannot.
fn name_edit(palettes: &mut Palettes, theme: egui::Theme, ui: &mut egui::Ui) {
    let id = ui.id().with(("style_palette_name", theme));
    let name = palettes.get(theme).name.clone();
    let mut draft = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| name.clone());
    let (status, can_save) = save_status(palettes, theme, &draft);

    let (edit, save) = ui
        .horizontal(|ui| {
            ui.label("Name");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save = ui
                    .add_enabled(can_save, egui::Button::new("Save"))
                    .on_hover_text("Save the palette under this name.");
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut draft)
                        .id(id.with("edit"))
                        .desired_width(ui.available_width())
                        .hint_text("Name to save as"),
                );
                (edit, save)
            })
            .inner
        })
        .inner;
    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    let mut keep = draft != name;
    if edit.lost_focus() && escape {
        keep = false;
    } else if can_save && (save.clicked() || (edit.lost_focus() && enter)) {
        keep = palettes.save(theme, &draft).is_err();
    }
    let status = match status {
        Status::Info(text) => egui::RichText::new(text).weak(),
        Status::Edited(text) => egui::RichText::new(text).color(ui.visuals().warn_fg_color),
        Status::Error(text) => egui::RichText::new(text).color(ui.visuals().error_fg_color),
    };
    ui.label(status.small());
    ui.data_mut(|d| {
        if keep {
            d.insert_temp(id, draft);
        } else {
            d.remove::<String>(id);
        }
    });
}

/// What saving the palette of `theme` as `draft` would do, and whether it
/// may.
fn save_status(palettes: &Palettes, theme: egui::Theme, draft: &str) -> (Status, bool) {
    let palette = palettes.get(theme);
    let target = draft.trim();
    if target != palette.name {
        return match Palettes::check_name(target) {
            Err(e) => (Status::Error(e.to_string()), false),
            Ok(()) if palettes.named(target).is_some() => (
                Status::Edited(format!("Enter saves over \"{target}\".")),
                true,
            ),
            Ok(()) => (Status::Info(format!("Enter saves as \"{target}\".")), true),
        };
    }
    let preset = base16::preset(target).is_some();
    match palettes.named(target) {
        None => (
            Status::Info("Not saved. Press Enter to save.".to_string()),
            true,
        ),
        Some(origin) if origin == *palette => {
            let text = if preset { "Preset." } else { "Saved." };
            (Status::Info(text.to_string()), false)
        }
        Some(_) if preset => (
            Status::Edited("Edited. Rename it to save the changes.".to_string()),
            false,
        ),
        Some(_) => (
            Status::Edited("Edited. Press Enter to save the changes.".to_string()),
            true,
        ),
    }
}

/// The palette's swatches. `origin` is the version the palette is named
/// after, which a swatch's context menu restores. Returns the swatch under
/// the pointer.
fn swatches_editor(
    palette: &mut Palette,
    origin: Option<&Palette>,
    ui: &mut egui::Ui,
) -> Option<Swatch> {
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
                let origin = origin.map(|o| (o.name.as_str(), o.get(swatch)));
                let resp = swatch_edit(ui, swatch, side, palette.get_mut(swatch), origin);
                if resp.hovered() {
                    hovered = Some(swatch);
                }
            }
        });
    }
    hovered
}

/// A swatch button that opens a colour picker for `color`. Its context menu
/// restores the colour from `origin`, the named palette and its colour for
/// this swatch.
fn swatch_edit(
    ui: &mut egui::Ui,
    swatch: Swatch,
    side: f32,
    color: &mut egui::Color32,
    origin: Option<(&str, egui::Color32)>,
) -> egui::Response {
    let resp = swatch_button(ui, *color, side, false).on_hover_text(format!(
        "{} {}\n\nRight-click to reset or copy.",
        swatch.label(),
        to_hex(*color)
    ));
    resp.context_menu(|ui| {
        match origin {
            Some((name, original)) => {
                let text = format!("Reset to {} from \"{name}\"", to_hex(original));
                if ui
                    .add_enabled(original != *color, egui::Button::new(text))
                    .clicked()
                {
                    *color = original;
                }
            }
            None => {
                ui.add_enabled(false, egui::Button::new("Reset"))
                    .on_disabled_hover_text(
                        "The palette is not saved, so it has no colour to reset to.",
                    );
            }
        }
        if ui.button("Copy hex").clicked() {
            ui.ctx().copy_text(to_hex(*color));
        }
    });
    egui::Popup::menu(&resp)
        .id(resp.id.with("picker"))
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
    let contrast = crate::style::role_contrast(roles, role, palette);
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
    let resp = chip_button(ui, color_ref.resolve(palette), color_ref_label(*color_ref))
        .on_hover_text(format!(
            "Pick the palette colour for {}.\n\nRight-click to reset.",
            role.label()
        ));
    resp.context_menu(|ui| {
        let default = Roles::default().get(role);
        let text = format!("Reset to {}", color_ref_label(default));
        if ui
            .add_enabled(default != *color_ref, egui::Button::new(text))
            .clicked()
        {
            *color_ref = default;
        }
    });
    let side = ui.spacing().interact_size.y * SWATCH_SCALE;
    egui::Popup::menu(&resp)
        .id(resp.id.with("picker"))
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

/// The corner, spacing, text and pane controls.
fn shape_editor(shape: &mut Shape, ui: &mut egui::Ui) {
    let radius = |ui: &mut egui::Ui, value: &mut u8, max: u8| {
        ui.add(egui::Slider::new(value, 0..=max).suffix(" px"));
    };
    let px = |ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>| {
        ui.add(
            egui::DragValue::new(value)
                .speed(0.1)
                .range(range)
                .fixed_decimals(1)
                .suffix(" px"),
        );
    };
    let heading = |ui: &mut egui::Ui, text: &str| {
        ui.label(egui::RichText::new(text).small().weak());
        ui.end_row();
    };
    egui::Grid::new("style_shape")
        .num_columns(2)
        .show(ui, |ui| {
            heading(ui, "corners");
            ui.label("panes")
                .on_hover_text("The corner radius of panes, windows, menus and popups.");
            radius(ui, &mut shape.pane_radius, 32);
            ui.end_row();
            ui.label("cards")
                .on_hover_text("The corner radius of group cards inside panes.");
            radius(ui, &mut shape.card_radius, 24);
            ui.end_row();
            ui.label("controls").on_hover_text(
                "The corner radius of buttons, fields, pills and other controls. A control \
                 is never rounder than a capsule.",
            );
            radius(ui, &mut shape.control_radius, 16);
            ui.end_row();
            ui.label("nodes").on_hover_text(
                "The corner radius of nodes. A node is never rounder than half its \
                 smallest height.",
            );
            radius(ui, &mut shape.node_radius, 16);
            ui.end_row();

            heading(ui, "spacing");
            ui.label("spacing")
                .on_hover_text("A scale on the gaps and padding between and within widgets.");
            let mut percent = shape.spacing * 100.0;
            let drag = egui::DragValue::new(&mut percent)
                .range(50.0..=200.0)
                .speed(1.0)
                .fixed_decimals(0)
                .suffix("%");
            if ui.add(drag).changed() {
                shape.spacing = percent / 100.0;
            }
            ui.end_row();
            ui.label("button padding")
                .on_hover_text("The space between a button's edge and its text, across and down.");
            ui.horizontal(|ui| {
                px(ui, &mut shape.button_padding.x, 0.0..=24.0);
                px(ui, &mut shape.button_padding.y, 0.0..=12.0);
            });
            ui.end_row();
            ui.label("pane gap").on_hover_text(
                "The gap between panes, where the scene shows through. Drag it to resize \
                 the panes on either side.",
            );
            px(ui, &mut shape.separator_width, 1.0..=24.0);
            ui.end_row();
            ui.label("window margin")
                .on_hover_text("The margin between the outer panes and the edge of the window.");
            px(ui, &mut shape.window_margin, 0.0..=48.0);
            ui.end_row();

            heading(ui, "text");
            ui.label("font").on_hover_text(
                "The font of all text. Plex mono sets text and code alike. egui sets text in \
                 a proportional face and code in a monospace face.",
            );
            super::kit::pills(
                ui,
                &mut shape.font,
                &[(Font::PlexMono, "plex mono"), (Font::Egui, "egui")],
            );
            ui.end_row();
            ui.label("text size")
                .on_hover_text("The size of body text. Other text keeps its size relative to it.");
            ui.add(
                egui::DragValue::new(&mut shape.text_size)
                    .range(8.0..=24.0)
                    .speed(0.1)
                    .fixed_decimals(1)
                    .suffix(" pt"),
            );
            ui.end_row();

            heading(ui, "panes");
            ui.label("group cards").on_hover_text(
                "How a group of settings, or a node in the inspector, stands apart from its \
                 pane. Nested shades it. Outline draws its edge.",
            );
            super::kit::pills(
                ui,
                &mut shape.cards,
                &[
                    (CardStyle::Nested, "nested"),
                    (CardStyle::Outline, "outline"),
                ],
            );
            ui.end_row();
            ui.label("graph panes").on_hover_text(
                "How a graph pane frames its graph. Open runs the graph up under the \
                 pane's title. Framed keeps it below the title, as in other panes.",
            );
            super::kit::pills(
                ui,
                &mut shape.graph_panes,
                &[
                    (GraphPaneStyle::Open, "open"),
                    (GraphPaneStyle::Framed, "framed"),
                ],
            );
            ui.end_row();
            ui.label("pane outline").on_hover_text(
                "The width of the outline around each pane. 0 draws none. Inked palettes \
                 need one, since their panes share the scene's colour.",
            );
            px(ui, &mut shape.pane_outline, 0.0..=4.0);
            ui.end_row();
            ui.label("shadows")
                .on_hover_text("Whether windows and popups cast a shadow.");
            super::kit::switch(ui, &mut shape.shadows);
            ui.end_row();
        });
}

/// A role's swatch, and its opacity when below full.
fn color_ref_label(color_ref: ColorRef) -> String {
    match color_ref.opacity < 1.0 {
        true => format!(
            "{} {:.0}%",
            color_ref.swatch.label(),
            color_ref.opacity * 100.0
        ),
        false => color_ref.swatch.label().to_string(),
    }
}
