//! A generic command palette widget adapted from rerun's command palette.

use egui::{Align2, Key, NumExt as _};
use std::collections::BTreeSet;

/// Trait that must be implemented by command types
pub trait Command: Copy + Sized {
    /// The text used for display and fuzzy matching
    fn text(&self) -> &str;
    /// A more detailed description shown in tooltips
    fn tooltip(&self) -> &str {
        ""
    }
    /// Optional keyboard shortcut for the command
    fn formatted_kb_shortcut(&self, _ctx: &egui::Context) -> Option<String> {
        None
    }
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct CommandPalette {
    visible: bool,
    query: String,
    selected_alternative: usize,
}

impl CommandPalette {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle(&mut self) {
        self.visible ^= true;
    }

    /// Show the command palette, if it is visible.
    #[must_use = "Returns the command that was selected"]
    pub fn show<T, I>(&mut self, egui_ctx: &egui::Context, commands: I) -> Option<T>
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

        let screen_rect = egui_ctx.screen_rect();
        let width = 300.0;
        let max_height = 320.0.at_most(screen_rect.height());

        egui::Window::new("Command Palette")
            .fixed_pos(screen_rect.center() - 0.5 * max_height * egui::Vec2::Y)
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
            })?
            .inner?
    }

    #[must_use = "Returns the command that was selected"]
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
            self.selected_alternative = 0;
            scroll_to_selected_alternative = true;
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

    #[must_use = "Returns the command that was selected"]
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

        for (i, fuzzy_match) in matches.iter().enumerate() {
            let command = fuzzy_match.command;
            let kb_shortcut_text = command.formatted_kb_shortcut(ui.ctx()).unwrap_or_default();

            let (rect, response) = ui.allocate_at_least(
                egui::vec2(ui.available_width(), item_height),
                egui::Sense::click(),
            );

            let response = response.on_hover_text(command.tooltip());

            if response.clicked() {
                selected_command = Some(command);
            }

            let selected = i == self.selected_alternative;
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

            num_alternatives += 1;
        }

        if num_alternatives == 0 {
            ui.weak("No matching results");
        }

        // Move up/down in the list:
        self.selected_alternative = self.selected_alternative.saturating_sub(
            ui.input_mut(|i| i.count_and_consume_key(Default::default(), Key::ArrowUp)),
        );
        self.selected_alternative = self.selected_alternative.saturating_add(
            ui.input_mut(|i| i.count_and_consume_key(Default::default(), Key::ArrowDown)),
        );

        self.selected_alternative = self
            .selected_alternative
            .clamp(0, num_alternatives.saturating_sub(1));

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
