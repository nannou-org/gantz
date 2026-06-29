//! A generic node palette widget adapted from rerun's command palette.

use egui::{Align2, Key, NumExt as _};
use std::borrow::Cow;
use std::collections::BTreeSet;

/// Trait that must be implemented by command types
pub trait Command: Copy + Sized {
    /// The text used for display and fuzzy matching
    fn text(&self) -> &str;
    /// A concise description shown inline, right of the name. Default none.
    fn description(&self) -> Option<Cow<'static, str>> {
        None
    }
    /// Detailed information for this command, rendered within the per-entry
    /// hover tooltip. Default renders nothing.
    fn info_ui(&self, _ui: &mut egui::Ui) {}
    /// Optional keyboard shortcut for the command
    fn formatted_kb_shortcut(&self, _ctx: &egui::Context) -> Option<String> {
        None
    }
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct NodePalette {
    visible: bool,
    query: String,
    /// The highlighted entry, or `None` when nothing is highlighted (the
    /// just-opened, browse-and-click state). Set by typing or arrow navigation.
    selected_alternative: Option<usize>,
}

impl NodePalette {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle(&mut self) {
        self.visible ^= true;
    }

    /// Show the node palette, if it is visible.
    ///
    /// `area` is the rect the palette is centered over (e.g. the graph scene).
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

        // Collect commands early since we'll need them multiple times
        let commands: Vec<T> = commands.into_iter().collect();

        // A modest widening to fit each entry's inline description; the full
        // details remain available via the entry's hover tooltip.
        let width = 400.0.at_most(area.width());
        let max_height = 320.0.at_most(area.height());

        let window_response = egui::Window::new("Node Palette")
            .fixed_pos(area.center() - 0.5 * max_height * egui::Vec2::Y)
            .fixed_size([width, max_height])
            .pivot(egui::Align2::CENTER_TOP)
            .resizable(false)
            .scroll(false)
            .title_bar(false)
            .show(egui_ctx, |ui| {
                egui::Frame {
                    inner_margin: 2.0.into(),
                    ..Default::default()
                }
                .show(ui, |ui| self.window_content_ui(ui, &commands))
                .inner
            });

        // Close on a click outside the palette. `clicked_elsewhere` tests the
        // window's rect geometrically, so interacting with inner widgets (the
        // text field, list items, scrollbar) never reads as an outside click.
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
        // Check _before_ we add the `TextEdit`, so it doesn't steal it.
        let enter_pressed = ui.input_mut(|i| i.consume_key(Default::default(), Key::Enter));

        let text_response = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .desired_width(f32::INFINITY)
                .lock_focus(true),
        );
        text_response.request_focus();
        let mut scroll_to_selected_alternative = false;
        if text_response.changed() {
            // Highlight the top match while filtering (so Enter works), but show
            // no highlight for an empty query - it's a browse-and-click list.
            self.selected_alternative = (!self.query.is_empty()).then_some(0);
            scroll_to_selected_alternative = self.selected_alternative.is_some();
        }

        let selected_command = egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                self.alternatives_ui(ui, commands, enter_pressed, scroll_to_selected_alternative)
            })
            .inner;

        if selected_command.is_some() {
            *self = Self::new();
        }

        selected_command
    }

    /// Render the matching entries. Returns the command chosen by click/Enter
    /// (which closes the palette).
    fn alternatives_ui<T: Command>(
        &mut self,
        ui: &mut egui::Ui,
        commands: &[T],
        enter_pressed: bool,
        mut scroll_to_selected_alternative: bool,
    ) -> Option<T> {
        scroll_to_selected_alternative |= ui.input(|i| i.key_pressed(Key::ArrowUp));
        scroll_to_selected_alternative |= ui.input(|i| i.key_pressed(Key::ArrowDown));

        let query = self.query.to_lowercase();

        let item_height = 16.0;
        let font_id = egui::TextStyle::Button.resolve(ui.style());

        let mut num_alternatives: usize = 0;
        let mut selected_command = None;

        let matches = commands_that_match(&query, commands);

        // Align descriptions into a single column just past the widest name (so
        // names align at the left and descriptions align at `desc_x`), capped so
        // the description always keeps at least ~half the row.
        let row_width = ui.available_width();
        let name_col_w = matches
            .iter()
            .map(|m| {
                ui.painter()
                    .layout_no_wrap(
                        m.command.text().to_owned(),
                        font_id.clone(),
                        egui::Color32::WHITE,
                    )
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max);
        let desc_x = (name_col_w + 12.0).at_most(row_width * 0.55);

        for (i, fuzzy_match) in matches.iter().enumerate() {
            let command = fuzzy_match.command;
            let kb_shortcut_text = command.formatted_kb_shortcut(ui.ctx()).unwrap_or_default();

            let (rect, response) = ui.allocate_at_least(
                egui::vec2(ui.available_width(), item_height),
                egui::Sense::click(),
            );

            let response = response.on_hover_ui(|ui| {
                // Re-assert the wrap width every frame so the tooltip widens as
                // content grows (see the note in `graph_scene::socket_hover`).
                let max_width = ui.spacing().tooltip_width;
                ui.set_max_width(max_width);
                command.info_ui(ui);
            });

            if response.clicked() {
                selected_command = Some(command);
            }

            let selected = self.selected_alternative == Some(i);
            let style = ui.style().interact_selectable(&response, selected);

            if selected {
                ui.painter()
                    .rect_filled(rect, style.corner_radius, ui.visuals().selection.bg_fill);

                if enter_pressed {
                    selected_command = Some(command);
                }

                if scroll_to_selected_alternative {
                    ui.scroll_to_rect(rect, None);
                }
            }

            let text = format_match(fuzzy_match, ui, &font_id, style.text_color());

            let galley = text.into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::FontSelection::default(),
            );
            let text_rect = Align2::LEFT_CENTER
                .anchor_rect(egui::Rect::from_min_size(rect.left_center(), galley.size()));
            ui.painter()
                .galley(text_rect.min, galley, style.text_color());

            ui.painter().text(
                rect.right_center(),
                Align2::RIGHT_CENTER,
                kb_shortcut_text,
                font_id.clone(),
                if selected {
                    style.text_color()
                } else {
                    ui.visuals().weak_text_color()
                },
            );

            // The description, aligned in its own column right of the name and
            // dimmed. Overflow is clipped by the scroll area; the hover tooltip
            // carries the full text.
            if let Some(desc) = command.description() {
                ui.painter().text(
                    egui::pos2(rect.left() + desc_x, rect.center().y),
                    Align2::LEFT_CENTER,
                    desc.as_ref(),
                    font_id.clone(),
                    if selected {
                        style.text_color()
                    } else {
                        ui.visuals().weak_text_color()
                    },
                );
            }

            num_alternatives += 1;
        }

        if num_alternatives == 0 {
            ui.weak("No matching results");
        }

        // Move up/down in the list. From no highlight, the first arrow press
        // starts at the top; otherwise step and keep the highlight in range.
        let up = ui.input_mut(|i| i.count_and_consume_key(Default::default(), Key::ArrowUp));
        let down = ui.input_mut(|i| i.count_and_consume_key(Default::default(), Key::ArrowDown));
        self.selected_alternative = if num_alternatives == 0 {
            None
        } else if up == 0 && down == 0 {
            self.selected_alternative
                .map(|sel| sel.min(num_alternatives - 1))
        } else {
            let next = match self.selected_alternative {
                None => 0,
                Some(cur) => (cur + down).saturating_sub(up),
            };
            Some(next.min(num_alternatives - 1))
        };

        selected_command
    }
}

struct FuzzyMatch<T> {
    command: T,
    score: isize,
    fuzzy_match: Option<sublime_fuzzy::Match>,
}

fn commands_that_match<T: Command>(query: &str, commands: &[T]) -> Vec<FuzzyMatch<T>> {
    if query.is_empty() {
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
    }
}

fn format_match<T: Command>(
    m: &FuzzyMatch<T>,
    ui: &egui::Ui,
    font_id: &egui::FontId,
    default_text_color: egui::Color32,
) -> egui::WidgetText {
    let target_text = m.command.text();

    if let Some(fm) = &m.fuzzy_match {
        let matched_indices: BTreeSet<_> = fm.matched_indices().collect();

        let mut job = egui::text::LayoutJob::default();
        for (i, c) in target_text.chars().enumerate() {
            let color = if matched_indices.contains(&i) {
                ui.visuals().strong_text_color()
            } else {
                default_text_color
            };
            job.append(
                &c.to_string(),
                0.0,
                egui::text::TextFormat::simple(font_id.clone(), color),
            );
        }

        job.into()
    } else {
        egui::RichText::new(target_text)
            .color(default_text_color)
            .into()
    }
}
