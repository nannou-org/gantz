//! A custom tab widget shared by the inner graph tree and the outer pane tree.
//!
//! It renders a tab as plain text coloured by state, with no background box
//! and a small close button, so all tabs look consistent. An optional
//! speaker leads the title as a mute toggle. An optional badge follows it
//! as a small clickable text, for a per-tab picker.

/// Response from the [`Tab`] widget.
pub struct TabResponse {
    /// The response for the tab area. Use it for click and drag detection.
    pub tab: egui::Response,
    /// The response for the close button, if present.
    pub close: Option<egui::Response>,
    /// The response for the speaker, if present.
    pub audio: Option<egui::Response>,
    /// The response for the badge, if present.
    pub badge: Option<egui::Response>,
}

/// A tab widget displaying a title with an optional close button.
pub struct Tab {
    text: egui::WidgetText,
    active: bool,
    closable: bool,
    id: egui::Id,
    /// Optional hover hint for the tab, for example "double-click to rename".
    hint: Option<egui::WidgetText>,
    /// Optional painted status dot after the title, as colour and hover text.
    status: Option<(egui::Color32, egui::WidgetText)>,
    /// Optional painted speaker before the title, as whether it is muted.
    audio: Option<bool>,
    /// Optional clickable text after the title.
    badge: Option<egui::WidgetText>,
}

impl Tab {
    pub fn new(text: impl Into<egui::WidgetText>, id: egui::Id) -> Self {
        Self {
            text: text.into(),
            active: false,
            closable: false,
            id,
            hint: None,
            status: None,
            audio: None,
            badge: None,
        }
    }

    /// Show a small painted speaker before the title, struck through when
    /// `muted`. Its click is reported via [`TabResponse::audio`].
    pub fn audio(mut self, muted: bool) -> Self {
        self.audio = Some(muted);
        self
    }

    /// Show a small clickable text after the title, coloured like the close
    /// button. Its click is reported via [`TabResponse::badge`]. Use it for
    /// a per-tab picker that opens a menu on the response.
    pub fn badge(mut self, text: impl Into<egui::WidgetText>) -> Self {
        self.badge = Some(text.into());
        self
    }

    /// Show a small painted status dot after the title with the given hover
    /// text, for example a collab session's connection state. The dot is
    /// painted rather than a glyph because circle glyphs are missing from
    /// egui's default fonts on some platforms.
    pub fn status_dot(mut self, color: egui::Color32, hover: impl Into<egui::WidgetText>) -> Self {
        self.status = Some((color, hover.into()));
        self
    }

    /// Set whether this tab is currently active.
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
            status,
            audio,
            badge,
        } = self;

        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let wrap = Some(egui::TextWrapMode::Extend);
        let galley = text.into_galley(ui, wrap, f32::INFINITY, font_id.clone());
        let badge_galley = badge.map(|b| b.into_galley(ui, wrap, f32::INFINITY, font_id));

        let x_margin = ui.spacing().button_padding.x;
        let close_btn_width = if closable {
            ui.spacing().icon_width
        } else {
            0.0
        };
        let dot_width = if status.is_some() {
            ui.spacing().icon_width * 0.75
        } else {
            0.0
        };
        let audio_width = if audio.is_some() {
            ui.spacing().icon_width
        } else {
            0.0
        };
        let badge_width = badge_galley.as_ref().map_or(0.0, |g| g.size().x + x_margin);

        let desired_size = egui::vec2(
            galley.size().x
                + 2.0 * x_margin
                + audio_width
                + badge_width
                + dot_width
                + close_btn_width,
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
        let mut audio_response = None;
        let mut badge_response = None;

        if ui.is_rect_visible(rect) {
            // Only the text colour responds to state. There is no background.
            let text_color = if active {
                ui.visuals().strong_text_color()
            } else if tab_response.hovered() {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            };

            if let Some(muted) = audio {
                let audio_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + x_margin, rect.top()),
                    egui::pos2(rect.left() + x_margin + audio_width, rect.bottom()),
                );
                let hover = if muted {
                    "click to unmute"
                } else {
                    "click to mute"
                };
                let audio_res = ui
                    .interact(audio_rect, id.with("audio"), egui::Sense::click())
                    .on_hover_cursor(egui::CursorIcon::Default)
                    .on_hover_text(hover);
                let color = if audio_res.hovered() {
                    ui.visuals().strong_text_color()
                } else {
                    ui.visuals().weak_text_color()
                };
                paint_speaker(
                    ui.painter(),
                    audio_rect,
                    color,
                    muted,
                    ui.visuals().panel_fill,
                );
                audio_response = Some(audio_res);
            }

            // Draw the title, leaving space for the speaker, badge, dot and
            // close areas.
            let text_rect = rect
                .shrink2(egui::vec2(x_margin, 0.0))
                .with_min_x(rect.left() + x_margin + audio_width)
                .with_max_x(rect.right() - close_btn_width - dot_width - badge_width);
            let text_pos = egui::Align2::LEFT_CENTER
                .align_size_within_rect(galley.size(), text_rect)
                .min;
            ui.painter().galley(text_pos, galley, text_color);

            // Draw the badge between the title and the status dot.
            if let Some(badge_galley) = badge_galley {
                let badge_rect = egui::Rect::from_min_max(
                    egui::pos2(
                        rect.right() - close_btn_width - dot_width - badge_width,
                        rect.top(),
                    ),
                    egui::pos2(rect.right() - close_btn_width - dot_width, rect.bottom()),
                );
                let badge_res = ui
                    .interact(badge_rect, id.with("badge"), egui::Sense::click())
                    .on_hover_cursor(egui::CursorIcon::Default);
                let color = if badge_res.hovered() {
                    ui.visuals().strong_text_color()
                } else {
                    ui.visuals().weak_text_color()
                };
                let pos = egui::Align2::RIGHT_CENTER
                    .align_size_within_rect(badge_galley.size(), badge_rect)
                    .min;
                ui.painter().galley(pos, badge_galley, color);
                badge_response = Some(badge_res);
            }

            // Draw the status dot between the badge and the close button.
            if let Some((color, hover)) = status {
                let dot_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.right() - close_btn_width - dot_width, rect.top()),
                    egui::pos2(rect.right() - close_btn_width, rect.bottom()),
                );
                ui.interact(dot_rect, id.with("status_dot"), egui::Sense::hover())
                    .on_hover_text(hover);
                let radius = (dot_rect.width() * 0.3).min(dot_rect.height() * 0.5);
                ui.painter().circle_filled(dot_rect.center(), radius, color);
            }

            if closable {
                let close_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.right() - close_btn_width, rect.top()),
                    rect.right_bottom(),
                );
                let close_id = id.with("close");
                let close_res = ui
                    .interact(close_rect, close_id, egui::Sense::click())
                    .on_hover_cursor(egui::CursorIcon::Default);

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
            audio: audio_response,
            badge: badge_response,
        }
    }
}

/// Paint a speaker centred in `rect`, struck through when `muted`. The strike
/// is cut in `gap`, the tab bar fill, so it reads at any size.
fn paint_speaker(
    painter: &egui::Painter,
    rect: egui::Rect,
    color: egui::Color32,
    muted: bool,
    gap: egui::Color32,
) {
    let s = rect.width().min(rect.height()) * 0.6;
    let c = rect.center();
    let body = egui::Rect::from_min_max(
        egui::pos2(c.x - s * 0.5, c.y - s * 0.2),
        egui::pos2(c.x - s * 0.15, c.y + s * 0.2),
    );
    painter.rect_filled(body, 0.0, color);
    let cone = vec![
        egui::pos2(c.x - s * 0.15, c.y - s * 0.2),
        egui::pos2(c.x + s * 0.35, c.y - s * 0.5),
        egui::pos2(c.x + s * 0.35, c.y + s * 0.5),
        egui::pos2(c.x - s * 0.15, c.y + s * 0.2),
    ];
    painter.add(egui::Shape::convex_polygon(cone, color, egui::Stroke::NONE));
    if muted {
        let a = egui::pos2(c.x - s * 0.5, c.y + s * 0.5);
        let b = egui::pos2(c.x + s * 0.5, c.y - s * 0.5);
        painter.line_segment([a, b], egui::Stroke::new(s * 0.3, gap));
        painter.line_segment([a, b], egui::Stroke::new(s * 0.12, color));
    }
}
