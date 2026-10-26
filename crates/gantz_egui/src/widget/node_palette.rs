//! A generic node palette widget adapted from rerun's command palette.
//!
//! The palette floats over the scene. A search field leads it. Below, the
//! matching commands are listed by group on the left, and the details of the
//! highlighted or hovered command sit in a card on the right. A footer names
//! the keys the palette understands.

use crate::{style, widget::kit};
use egui::{Key, NumExt as _};
use std::borrow::Cow;
use std::collections::BTreeSet;

/// The trait a palette command type implements.
pub trait Command: Copy + Sized {
    /// The text used for display and fuzzy matching.
    fn text(&self) -> &str;
    /// A concise description shown inline, right of the name. Default none.
    fn description(&self) -> Option<Cow<'static, str>> {
        None
    }
    /// Detailed information for this command, rendered in the details card.
    /// Default renders nothing.
    fn info_ui(&self, _ui: &mut egui::Ui) {}
    /// The group the command is listed under, such as "signal". Commands
    /// without one are listed under no heading.
    fn group(&self) -> Option<Cow<'static, str>> {
        None
    }
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct NodePalette {
    visible: bool,
    query: String,
    /// The highlighted entry, or `None` when nothing is highlighted. Nothing is
    /// highlighted in the just-opened browse-and-click state. Typing or arrow
    /// navigation sets it.
    selected_alternative: Option<usize>,
}

/// The size of the palette, before it is fitted to the scene.
const SIZE: egui::Vec2 = egui::vec2(700.0, 440.0);

/// The share of the palette's width that the list takes.
const LIST_SHARE: f32 = 0.48;

/// The height of a row in the list.
const ROW_HEIGHT: f32 = 24.0;

impl NodePalette {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle(&mut self) {
        self.visible ^= true;
    }

    /// Show the palette from the next [`Self::show`] call.
    pub fn open(&mut self) {
        self.visible = true;
    }

    /// Whether the palette is shown.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Show the node palette, if it is visible.
    ///
    /// `area` is the rect the palette is centered over, for example the graph
    /// scene.
    #[must_use = "Returns the command that was selected"]
    pub fn show<T, I>(
        &mut self,
        egui_ctx: &egui::Context,
        area: egui::Rect,
        commands: I,
    ) -> Option<T>
    where
        T: Command,
        I: IntoIterator<Item = T>,
    {
        self.visible &= !egui_ctx.input_mut(|i| i.key_pressed(Key::Escape));
        if !self.visible {
            self.query.clear();
            return None;
        }

        let commands: Vec<T> = commands.into_iter().collect();
        let size = egui::vec2(
            SIZE.x.at_most(area.width() - 16.0),
            SIZE.y.at_most(area.height() - 16.0),
        );
        let shape = style::shape(egui_ctx);
        let window_response = egui::Window::new("Node Palette")
            .fixed_pos(area.center() - 0.5 * size)
            .fixed_size(size)
            .resizable(false)
            .scroll(false)
            .title_bar(false)
            .frame(
                egui::Frame::window(&egui_ctx.global_style())
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(shape.window_corner())
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(egui_ctx, |ui| self.window_content_ui(ui, &commands));

        // Close on a click outside the palette. `clicked_elsewhere` tests the
        // window's rect geometrically, so interacting with inner widgets never
        // reads as an outside click.
        if let Some(ref resp) = window_response {
            if resp.response.clicked_elsewhere() {
                self.visible = false;
                self.query.clear();
                return None;
            }
        }

        window_response?.inner?
    }

    fn window_content_ui<T: Command>(&mut self, ui: &mut egui::Ui, commands: &[T]) -> Option<T> {
        // Consume Enter before adding the `TextEdit`, so it does not steal it.
        let enter_pressed = ui.input_mut(|i| i.consume_key(Default::default(), Key::Enter));

        let matches = commands_that_match(&self.query.to_lowercase(), commands);
        let mut text_response = None;
        ui.horizontal(|ui| {
            let count = format!("{} found", matches.len());
            let count_w = ui.painter().layout_no_wrap(
                count.clone(),
                egui::TextStyle::Small.resolve(ui.style()),
                egui::Color32::PLACEHOLDER,
            );
            let edit = egui::TextEdit::singleline(&mut self.query)
                .hint_text("search nodes")
                .desired_width(ui.available_width() - count_w.size().x - 12.0)
                .lock_focus(true);
            text_response = Some(ui.add(edit));
            ui.label(egui::RichText::new(count).small().weak());
        });
        let text_response = text_response.expect("the search field is drawn");
        text_response.request_focus();
        let mut scroll_to_selected = false;
        if text_response.changed() {
            // Highlight the top match while filtering so Enter works. Show no
            // highlight for an empty query, since that is a browse-and-click
            // list.
            self.selected_alternative = (!self.query.is_empty()).then_some(0);
            scroll_to_selected = self.selected_alternative.is_some();
        }
        ui.add_space(6.0);

        let footer_height = ui.spacing().interact_size.y + 6.0;
        let body_height = (ui.available_height() - footer_height).at_least(0.0);
        let list_width = ui.available_width() * LIST_SHARE;
        let mut chosen = None;
        let mut shown = None;
        ui.horizontal_top(|ui| {
            let down = egui::Layout::top_down(egui::Align::Min);
            ui.allocate_ui_with_layout(egui::vec2(list_width, body_height), down, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("node-palette-list")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let (c, s) =
                            self.alternatives_ui(ui, &matches, enter_pressed, scroll_to_selected);
                        chosen = c;
                        shown = s;
                    });
            });
            let details_width = ui.available_width();
            ui.allocate_ui_with_layout(egui::vec2(details_width, body_height), down, |ui| {
                ui.set_min_size(egui::vec2(details_width, body_height));
                if let Some(command) = shown {
                    kit::card(ui, None, None, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("node-palette-details")
                            .auto_shrink([false, false])
                            .show(ui, |ui| command.info_ui(ui));
                    });
                }
            });
        });

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            kit::keycap(ui, "↑", false);
            kit::keycap(ui, "↓", false);
            ui.weak("choose");
            ui.add_space(10.0);
            kit::keycap(ui, "enter", false);
            ui.weak("add");
            ui.add_space(10.0);
            kit::keycap(ui, "esc", false);
            ui.weak("close");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak("adds at the pointer");
            });
        });

        if chosen.is_some() {
            *self = Self::new();
        }
        chosen
    }

    /// Render the matching entries under their group headings. Returns the
    /// command chosen by click or Enter, which closes the palette, and the
    /// command whose details to show: the hovered one, else the highlighted
    /// one, else the first.
    fn alternatives_ui<T: Command>(
        &mut self,
        ui: &mut egui::Ui,
        matches: &[FuzzyMatch<T>],
        enter_pressed: bool,
        mut scroll_to_selected: bool,
    ) -> (Option<T>, Option<T>) {
        scroll_to_selected |= ui.input(|i| i.key_pressed(Key::ArrowUp));
        scroll_to_selected |= ui.input(|i| i.key_pressed(Key::ArrowDown));

        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let small = egui::TextStyle::Small.resolve(ui.style());
        let shape = style::shape(ui.ctx());
        let mut chosen = None;
        let mut hovered = None;
        let mut group: Option<Cow<'static, str>> = None;

        for (i, m) in matches.iter().enumerate() {
            let command = m.command;
            let next_group = command.group();
            if next_group != group {
                if let Some(heading) = &next_group {
                    ui.add_space(if i == 0 { 0.0 } else { 6.0 });
                    ui.label(egui::RichText::new(heading.as_ref()).small().weak());
                }
                group = next_group;
            }

            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), ROW_HEIGHT),
                egui::Sense::click(),
            );
            if response.clicked() {
                chosen = Some(command);
            }
            if response.hovered() {
                hovered = Some(command);
            }
            let selected = self.selected_alternative == Some(i);
            if selected && enter_pressed {
                chosen = Some(command);
            }
            if selected && scroll_to_selected {
                ui.scroll_to_rect(rect, None);
            }

            let visuals = ui.visuals();
            let radius = shape.widget_corner(rect.height());
            let (name_color, desc_color) = if selected {
                let ink = style::color(ui, style::Role::Panel);
                ui.painter()
                    .rect_filled(rect, radius, visuals.strong_text_color());
                (ink, ink.gamma_multiply(0.75))
            } else {
                if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, radius, visuals.widgets.hovered.weak_bg_fill);
                }
                (visuals.strong_text_color(), visuals.weak_text_color())
            };

            let name = name_job(m, &font_id, name_color);
            let name = ui.painter().layout_job(name);
            let pad = 8.0;
            let name_pos = egui::pos2(rect.left() + pad, rect.center().y - name.size().y / 2.0);
            let name_w = name.size().x;
            ui.painter().galley(name_pos, name, name_color);
            if let Some(desc) = command.description() {
                let x = rect.left() + pad + name_w + 10.0;
                let max_w = (rect.right() - pad - x).at_least(0.0);
                let mut job = egui::text::LayoutJob::simple_singleline(
                    desc.into_owned(),
                    small.clone(),
                    desc_color,
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
                let galley = ui.painter().layout_job(job);
                let pos = egui::pos2(x, rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(pos, galley, desc_color);
            }
        }

        if matches.is_empty() {
            ui.weak("no matching nodes");
        }

        // Move up or down in the list. From no highlight, the first arrow press
        // starts at the top. Otherwise step and keep the highlight in range.
        let n = matches.len();
        let up = ui.input_mut(|i| i.count_and_consume_key(Default::default(), Key::ArrowUp));
        let down = ui.input_mut(|i| i.count_and_consume_key(Default::default(), Key::ArrowDown));
        self.selected_alternative = if n == 0 {
            None
        } else if up == 0 && down == 0 {
            self.selected_alternative.map(|sel| sel.min(n - 1))
        } else {
            let next = match self.selected_alternative {
                None => 0,
                Some(cur) => (cur + down).saturating_sub(up),
            };
            Some(next.min(n - 1))
        };

        let highlighted = self
            .selected_alternative
            .and_then(|i| matches.get(i))
            .map(|m| m.command);
        let shown = hovered
            .or(highlighted)
            .or_else(|| matches.first().map(|m| m.command));
        (chosen, shown)
    }
}

struct FuzzyMatch<T> {
    command: T,
    score: isize,
    fuzzy_match: Option<sublime_fuzzy::Match>,
}

/// The commands that match `query`, each group together. Groups keep the
/// order of their best match, and so do the commands within a group. An
/// empty query matches every command, in the given order.
fn commands_that_match<T: Command>(query: &str, commands: &[T]) -> Vec<FuzzyMatch<T>> {
    let mut matches: Vec<_> = if query.is_empty() {
        commands
            .iter()
            .map(|&command| FuzzyMatch {
                command,
                score: 0,
                fuzzy_match: None,
            })
            .collect()
    } else {
        let mut matches: Vec<_> = commands
            .iter()
            .filter_map(|&command| {
                let target_text = command.text();
                sublime_fuzzy::best_match(query, target_text).map(|fuzzy_match| FuzzyMatch {
                    command,
                    score: fuzzy_match.score(),
                    fuzzy_match: Some(fuzzy_match),
                })
            })
            .collect();
        matches.sort_by_key(|m| -m.score); // highest score first
        matches
    };
    // Gather each group at the place of its first, so best, member. The sort
    // is stable, so order within a group holds.
    let mut groups: Vec<Option<Cow<'static, str>>> = vec![];
    for m in &matches {
        let g = m.command.group();
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    matches.sort_by_key(|m| {
        let g = m.command.group();
        groups.iter().position(|x| *x == g).unwrap_or(usize::MAX)
    });
    matches
}

/// The command's name, with the letters that matched the query underlined.
fn name_job<T: Command>(
    m: &FuzzyMatch<T>,
    font_id: &egui::FontId,
    color: egui::Color32,
) -> egui::text::LayoutJob {
    let target_text = m.command.text();
    let matched: BTreeSet<usize> = m
        .fuzzy_match
        .as_ref()
        .map(|fm| fm.matched_indices().copied().collect())
        .unwrap_or_default();
    let mut job = egui::text::LayoutJob::default();
    for (i, c) in target_text.chars().enumerate() {
        let mut format = egui::text::TextFormat::simple(font_id.clone(), color);
        if matched.contains(&i) {
            format.underline = egui::Stroke::new(1.5f32, color);
        }
        job.append(&c.to_string(), 0.0, format);
    }
    job
}
