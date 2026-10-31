//! Small widgets that the panes, settings and menus share: group cards,
//! option pills, switches, keycaps and event dots.
//!
//! They draw with the applied style's roles and [`Shape`][crate::style::Shape], so a theme or
//! roundness change reaches every one of them.

use crate::style::{self, CardStyle, Role};

/// The width of a card, pill group or keycap outline.
pub const OUTLINE: f32 = 1.5;

/// The size of a [`switch`], as multiples of the interact height.
const SWITCH: egui::Vec2 = egui::vec2(1.65, 1.0);

/// The room between a group of [`pills`] and its outline.
const PILLS_MARGIN: i8 = 1;

/// The gap between a node and a tag that hangs under it. See [`paint_tag`].
pub const TAG_GAP: f32 = 8.0;

/// The most chars of a message that a tag shows. See [`elide`].
pub const TAG_CHARS: usize = 40;

/// The space between a field's outline and its text. See [`field_visuals`].
pub const FIELD_MARGIN: egui::Margin = egui::Margin::symmetric(8, 3);

/// The radius of an [`event_dot`].
const DOT_RADIUS: f32 = 4.0;

/// The room around an [`event_dot`] that takes its clicks.
const DOT_ROOM: f32 = 4.0;

/// The opacity of a fired [`event_dot`]'s halo.
const DOT_HALO: f32 = 0.22;

/// How long an [`event_dot`] glows after it fires, in seconds.
const DOT_GLOW: f64 = 0.24;

/// The key of the time an [`event_dot`] last fired, beside its id.
const DOT_FIRED: &str = "fired";

/// A group card: an optional title, optional context to its right, and
/// `add_contents` below. [`Shape::cards`][crate::style::Shape::cards] chooses whether it is shaded or
/// outlined.
pub fn card<R>(
    ui: &mut egui::Ui,
    title: Option<&str>,
    context: Option<&str>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let frame = card_frame(ui, false);
    frame
        .show(ui, |ui| {
            exact_width(ui, |ui| {
                card_controls(ui);
                if title.is_some() || context.is_some() {
                    ui.horizontal(|ui| {
                        if let Some(title) = title {
                            let color = ui.visuals().strong_text_color();
                            ui.label(style::font::strong_text(ui, title).color(color));
                        }
                        if let Some(context) = context {
                            let layout = egui::Layout::right_to_left(egui::Align::Center);
                            ui.with_layout(layout, |ui| {
                                ui.label(egui::RichText::new(context).small().weak());
                            });
                        }
                    });
                }
                add_contents(ui)
            })
        })
        .inner
}

/// Run `add_contents` in a ui exactly the width available, clipped to it.
/// Contents wider than that are cut off rather than widening what holds
/// them, so that a card keeps to the margin of the pane it sits in.
pub fn exact_width<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    /// How far past the width contents may paint, for the outline of a
    /// control that fills the width.
    const BLEED: f32 = 2.0;
    let width = ui.available_width().max(0.0);
    let available = ui.available_rect_before_wrap();
    let top_left = available.min;
    let max_rect = egui::Rect::from_min_size(top_left, egui::vec2(width, available.height()));
    let builder = egui::UiBuilder::new()
        .id_salt(ui.next_auto_id())
        .max_rect(max_rect)
        .layout(*ui.layout());
    let mut child = ui.new_child(builder);
    child.set_min_width(width);
    let clip = ui.clip_rect();
    let x_range = max_rect.x_range().expand(BLEED);
    child.set_clip_rect(egui::Rect::from_x_y_ranges(x_range, clip.y_range()).intersect(clip));
    let inner = add_contents(&mut child);
    let height = child.min_rect().height();
    ui.allocate_rect(
        egui::Rect::from_min_size(top_left, egui::vec2(width, height)),
        egui::Sense::hover(),
    );
    inner
}

/// Fill the controls in a nested card with the pane's colour, so that they
/// stand apart from the card as wells. On an outlined card they keep their
/// own fill, as the card is the pane's colour.
pub fn card_controls(ui: &mut egui::Ui) {
    if style::shape(ui.ctx()).cards != CardStyle::Nested {
        return;
    }
    let well = style::color(ui, Role::Panel);
    let inactive = &mut ui.visuals_mut().widgets.inactive;
    inactive.bg_fill = well;
    inactive.weak_bg_fill = well;
}

/// The frame of a group card. A `selected` card takes the strong outline in
/// either style.
pub fn card_frame(ui: &egui::Ui, selected: bool) -> egui::Frame {
    let shape = style::shape(ui.ctx());
    let visuals = ui.visuals();
    let (fill, stroke) = match shape.cards {
        CardStyle::Nested => (style::color(ui, Role::Card), egui::Stroke::NONE),
        CardStyle::Outline => (
            egui::Color32::TRANSPARENT,
            egui::Stroke::new(OUTLINE, visuals.widgets.noninteractive.bg_stroke.color),
        ),
    };
    // Every card keeps an outline's width, clear when not drawn, so that a
    // selection outline never shifts its contents.
    let stroke = match selected {
        true => egui::Stroke::new(OUTLINE, visuals.strong_text_color()),
        false if stroke.is_empty() => egui::Stroke::new(OUTLINE, egui::Color32::TRANSPARENT),
        false => stroke,
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(shape.card_corner())
        .inner_margin(egui::Margin::symmetric(10, 8))
}

/// A row of option pills, one per `(value, label)` in `options`. The pill of
/// the current value is filled in the strong text colour. Clicking a pill
/// selects its value.
pub fn pills<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, &str)],
) -> egui::Response {
    let shape = style::shape(ui.ctx());
    let height = ui.spacing().interact_size.y;
    let stroke = egui::Stroke::new(OUTLINE, ui.visuals().widgets.noninteractive.bg_stroke.color);
    let inner = egui::Frame::new()
        .stroke(stroke)
        .corner_radius(shape.widget_corner(height + 4.0))
        .inner_margin(egui::Margin::same(PILLS_MARGIN))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                let mut changed: Option<egui::Response> = None;
                for &(option, label) in options {
                    let resp = pill(ui, *value == option, label);
                    if resp.clicked() && *value != option {
                        *value = option;
                        let mut resp = resp;
                        resp.mark_changed();
                        changed = Some(resp);
                    }
                }
                changed
            })
            .inner
        });
    match inner.inner {
        Some(resp) => resp,
        None => inner.response,
    }
}

/// A row with `label` and then [`pills`]. The label centres on the pills'
/// height.
pub fn pill_row<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    options: &[(T, &str)],
) -> egui::Response {
    ui.horizontal(|ui| {
        // The row is only as high as the widgets so far, so it takes the
        // pills' height before the label centres in it.
        ui.set_min_height(pill_height(ui) + 2.0 * (f32::from(PILLS_MARGIN) + OUTLINE));
        ui.label(label);
        pills(ui, value, options)
    })
    .inner
}

/// One pill: a label that is filled in the strong text colour while
/// `selected`, and weak otherwise.
pub fn pill(
    ui: &mut egui::Ui,
    selected: bool,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    let pad_x = pill_pad(ui);
    let galley = text.into().into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let size = egui::vec2(galley.size().x + pad_x * 2.0, pill_height(ui));
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let shape = style::shape(ui.ctx());
        let radius = shape.widget_corner(rect.height());
        let visuals = ui.visuals();
        let (fill, text_color) = match (selected, resp.hovered()) {
            (true, _) => (visuals.strong_text_color(), style::color(ui, Role::Panel)),
            (false, true) => (
                visuals.widgets.hovered.weak_bg_fill,
                visuals.strong_text_color(),
            ),
            (false, false) => (egui::Color32::TRANSPARENT, visuals.weak_text_color()),
        };
        ui.painter().rect_filled(rect, radius, fill);
        let pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(pos, galley, text_color);
    }
    resp
}

/// An on and off switch. A click flips `on`.
pub fn switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = ui.spacing().interact_size.y * SWITCH;
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool(resp.id, *on);
        let visuals = ui.visuals();
        let radius = rect.height() / 2.0;
        let track = match *on {
            true => visuals.strong_text_color(),
            false => visuals.widgets.noninteractive.bg_stroke.color,
        };
        let knob = match *on {
            true => style::color(ui, Role::Panel),
            false => visuals.weak_text_color(),
        };
        let knob = match resp.hovered() && !*on {
            true => visuals.text_color(),
            false => knob,
        };
        ui.painter().rect_filled(rect, radius, track);
        let r = radius - 3.0;
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), r, knob);
    }
    resp
}

/// A switch with a label to its left that fills the row, as in a settings
/// card.
pub fn switch_row(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            switch(ui, on)
        })
        .inner
    })
    .inner
}

/// A key or key combination drawn as a keycap, such as "ctrl c".
pub fn keycap(ui: &mut egui::Ui, text: &str, strong: bool) -> egui::Response {
    let color = match strong {
        true => ui.visuals().strong_text_color(),
        false => ui.visuals().weak_text_color(),
    };
    keycap_in(ui, text, color, egui::Sense::hover())
}

/// A keycap in `color` that responds to clicks, for a binding the user can
/// edit.
pub fn keycap_button(ui: &mut egui::Ui, text: &str, color: egui::Color32) -> egui::Response {
    let resp = keycap_in(ui, text, color, egui::Sense::click());
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn keycap_in(
    ui: &mut egui::Ui,
    text: &str,
    color: egui::Color32,
    sense: egui::Sense,
) -> egui::Response {
    let font = egui::FontId::monospace(ui.style().text_styles[&egui::TextStyle::Body].size * 0.88);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let pad = egui::vec2(6.0, 1.0);
    let size = galley.size() + pad * 2.0;
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let line = match resp.hovered() && sense.senses_click() {
            true => visuals.widgets.hovered.bg_stroke.color,
            false => visuals.widgets.noninteractive.bg_stroke.color,
        };
        let radius = style::shape(ui.ctx()).widget_corner(rect.height()) * 0.7;
        ui.painter().rect_stroke(
            rect,
            radius,
            egui::Stroke::new(OUTLINE, line),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(rect.min + pad, galley, color);
    }
    resp
}

/// A dot that marks an event, such as a bang, a load or a tick. It is weak
/// at rest, strong while the pointer is over it if it takes clicks, and it
/// glows for a moment after [`fire_dot`] with the same `id`. The response
/// senses `sense` over a little room around the dot.
pub fn event_dot(ui: &mut egui::Ui, id: egui::Id, sense: egui::Sense) -> egui::Response {
    // Exactly the dot's width, so that the gaps around it are the node's own
    // margin and item spacing. As high as a control, so that it centres on the
    // row like the value beside it.
    let size = egui::vec2(DOT_RADIUS * 2.0, ui.spacing().interact_size.y);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let resp = ui.interact(rect.expand(DOT_ROOM), id, sense);
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let glow = dot_glow(ui.ctx(), id);
        // Only a dot that takes clicks lights up under the pointer. Elsewhere
        // the dot only marks that the node pushes.
        let lit = sense.senses_click() && resp.hovered();
        let color = match lit || glow > 0.0 {
            true => visuals.strong_text_color(),
            false => visuals.weak_text_color(),
        };
        let center = rect.center();
        if glow > 0.0 {
            let halo = color.gamma_multiply(DOT_HALO * glow);
            ui.painter().circle_filled(center, DOT_RADIUS * 2.0, halo);
            ui.ctx().request_repaint();
        }
        ui.painter().circle_filled(center, DOT_RADIUS, color);
    }
    resp
}

/// Start the glow of the [`event_dot`] with `id`.
pub fn fire_dot(ctx: &egui::Context, id: egui::Id) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(id.with(DOT_FIRED), now));
}

/// How strongly the dot `id` glows, from `1.0` as it fires to `0.0` once
/// [`DOT_GLOW`] has passed.
fn dot_glow(ctx: &egui::Context, id: egui::Id) -> f32 {
    let Some(fired) = ctx.data(|d| d.get_temp::<f64>(id.with(DOT_FIRED))) else {
        return 0.0;
    };
    let elapsed = ctx.input(|i| i.time) - fired;
    (1.0 - elapsed / DOT_GLOW).clamp(0.0, 1.0) as f32
}

/// An event node's content: its [`event_dot`], then its `name` if it has
/// one. A bang has no name. `load!`, `tick!` and `update!` show theirs.
pub fn event_label(
    ui: &mut egui::Ui,
    id: egui::Id,
    name: Option<&str>,
    sense: egui::Sense,
) -> egui::Response {
    ui.horizontal(|ui| match name {
        Some(name) => {
            let dot = event_dot(ui, id, sense);
            ui.add(egui::Label::new(name).selectable(false));
            dot
        }
        // A dot alone sits in a square, so that a bang is round.
        None => {
            let pad = (ui.spacing().interact_size.y - DOT_RADIUS * 2.0) * 0.5;
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(pad);
            let dot = event_dot(ui, id, sense);
            ui.add_space(pad);
            dot
        }
    })
    .inner
}

/// A menu item with `shortcut`, if any, right-aligned in the weak text
/// colour.
pub fn menu_item(ui: &mut egui::Ui, label: &str, shortcut: Option<String>) -> egui::Response {
    let button = egui::Button::new(label).shortcut_text(shortcut.unwrap_or_default());
    ui.add(button)
}

/// A menu item for an action that removes or resets something, in the
/// error colour.
pub fn danger_item(ui: &mut egui::Ui, label: &str, shortcut: Option<String>) -> egui::Response {
    let text = egui::RichText::new(label).color(ui.visuals().error_fg_color);
    let button = egui::Button::new(text).shortcut_text(shortcut.unwrap_or_default());
    ui.add(button)
}

/// A quiet heading at the top of a menu, such as the name of the node it
/// acts on.
pub fn menu_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().weak());
}

/// Paint a small tag of `text` in `text_color` on `fill`, with its top-left
/// corner at `pos`. A node's error hangs under it on one. Returns the tag's
/// rect.
pub fn paint_tag(
    painter: &egui::Painter,
    style: &egui::Style,
    pos: egui::Pos2,
    text: &str,
    fill: egui::Color32,
    text_color: egui::Color32,
) -> egui::Rect {
    let font = egui::TextStyle::Small.resolve(style);
    let galley = painter.layout_no_wrap(text.to_string(), font, text_color);
    let pad = egui::vec2(6.0, 2.0);
    let rect = egui::Rect::from_min_size(pos, galley.size() + pad * 2.0);
    let radius = style::shape(painter.ctx()).widget_corner(rect.height());
    painter.rect_filled(rect, radius, fill);
    painter.galley(rect.min + pad, galley, text_color);
    rect
}

/// The first line of `text`, cut to at most `max` chars with an ellipsis.
pub fn elide(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("");
    match line.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_string(),
    }
}

/// The height of a [`pill`]: a line of button text with a little room. Tabs
/// take it too, so that every pill in the app is the same height.
pub fn pill_height(ui: &egui::Ui) -> f32 {
    let text = ui.text_style_height(&egui::TextStyle::Button);
    (text + 4.0).max(ui.spacing().interact_size.y - 2.0)
}

/// The width of a [`pill`] that shows `text`.
pub fn pill_width(ui: &egui::Ui, text: &str) -> f32 {
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    galley.size().x + 2.0 * pill_pad(ui)
}

/// The space between a pill's sides and its text.
fn pill_pad(ui: &egui::Ui) -> f32 {
    ui.spacing().button_padding.x + 2.0
}

/// The width of `text` in the body font.
pub fn text_width(ui: &egui::Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    ui.painter()
        .layout_no_wrap(text.to_string(), font, color)
        .size()
        .x
}

/// Size the sliders of `ui` to fill the rest of a row, once `beside` points
/// of other widgets and each slider's value box, as wide as `widest`, have
/// their room. A wider slider would push its card past the pane's margin.
/// Their rails take the opaque outline colour, so that they read on a card.
pub fn fit_sliders(ui: &mut egui::Ui, beside: f32, widest: &str) {
    let rail = ui.visuals().widgets.noninteractive.bg_stroke.color;
    ui.visuals_mut().widgets.inactive.bg_fill = rail;
    let value = text_width(ui, widest) + 2.0 * ui.spacing().button_padding.x;
    let gaps = ui.spacing().item_spacing.x;
    ui.spacing_mut().slider_width = (ui.available_width() - beside - value - gaps).max(32.0);
}

/// Draw the text fields of `ui` outlined on the pane, in place of egui's
/// dark fill. The outline keeps one width in every state, so that hover and
/// focus only recolour it and never move the text. Buttons in `ui` take the
/// outline too, so scope it to the fields.
pub fn field_visuals(ui: &mut egui::Ui) {
    let v = ui.visuals_mut();
    let line = v.widgets.noninteractive.bg_stroke.color;
    let hover = v.widgets.hovered.bg_stroke.color;
    let focus = v.strong_text_color();
    v.text_edit_bg_color = Some(egui::Color32::TRANSPARENT);
    let w = &mut v.widgets;
    for (w, color) in [
        (&mut w.inactive, line),
        (&mut w.hovered, hover),
        (&mut w.active, focus),
    ] {
        w.bg_stroke = egui::Stroke::new(OUTLINE, color);
        w.expansion = 0.0;
    }
    v.selection.stroke = egui::Stroke::new(OUTLINE, focus);
}
