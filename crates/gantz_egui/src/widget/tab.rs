//! A custom tab widget shared by the inner graph tree and the outer pane tree.
//!
//! It renders a tab as plain text (no background box) coloured by state, with a
//! small close button, so all tabs look consistent.

/// Response from the [`Tab`] widget.
pub struct TabResponse {
    /// The response for the tab area (for click/drag detection).
    pub tab: egui::Response,
    /// The response for the close button, if present.
    pub close: Option<egui::Response>,
}

/// A tab widget displaying a title with an optional close button.
pub struct Tab {
    text: egui::WidgetText,
    active: bool,
    closable: bool,
    id: egui::Id,
    /// Optional hover hint for the tab (e.g. "double-click to rename").
    hint: Option<egui::WidgetText>,
}

impl Tab {
    pub fn new(text: impl Into<egui::WidgetText>, id: egui::Id) -> Self {
        Self {
            text: text.into(),
            active: false,
            closable: false,
            id,
            hint: None,
        }
    }

    /// Set whether this tab is currently active (selected).
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Set whether this tab has a close button.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Set a hover hint shown over the tab.
    pub fn hint(mut self, hint: impl Into<egui::WidgetText>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Show the widget.
    pub fn show(self, ui: &mut egui::Ui) -> TabResponse {
        let Self {
            text,
            active,
            closable,
            id,
            hint,
        } = self;

        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, font_id);

        let x_margin = ui.spacing().button_padding.x;
        let close_btn_width = if closable {
            // Width for the close button area.
            ui.spacing().icon_width
        } else {
            0.0
        };

        let desired_size = egui::vec2(
            galley.size().x + 2.0 * x_margin + close_btn_width,
            ui.available_height(),
        );

        let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
        // Use ui.interact for proper drag support, like egui_tiles does.
        let mut tab_response = ui
            .interact(rect, id, egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Grab);
        if let Some(hint) = hint {
            tab_response = tab_response.on_hover_text(hint);
        }

        let mut close_response = None;

        if ui.is_rect_visible(rect) {
            // Text color based on state - no background, only text responds.
            let text_color = if active {
                ui.visuals().strong_text_color()
            } else if tab_response.hovered() {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            };

            // Draw title text (leaving space for close button if closable).
            let text_rect = if closable {
                rect.shrink2(egui::vec2(x_margin, 0.0))
                    .with_max_x(rect.right() - close_btn_width)
            } else {
                rect.shrink2(egui::vec2(x_margin, 0.0))
            };
            let text_pos = egui::Align2::LEFT_CENTER
                .align_size_within_rect(galley.size(), text_rect)
                .min;
            ui.painter().galley(text_pos, galley, text_color);

            // Draw close button if closable.
            if closable {
                let close_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.right() - close_btn_width, rect.top()),
                    rect.right_bottom(),
                );
                let close_id = id.with("close");
                let close_res = ui
                    .interact(close_rect, close_id, egui::Sense::click())
                    .on_hover_cursor(egui::CursorIcon::Default);

                // Draw the × character.
                let close_color = if close_res.hovered() {
                    ui.visuals().strong_text_color()
                } else {
                    ui.visuals().weak_text_color()
                };
                let close_font = egui::TextStyle::Body.resolve(ui.style());
                ui.painter().text(
                    close_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "×",
                    close_font,
                    close_color,
                );

                close_response = Some(close_res);
            }
        }

        TabResponse {
            tab: tab_response,
            close: close_response,
        }
    }
}
