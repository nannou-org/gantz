//! Shared row rendering for graph/commit selection widgets.

/// Response returned from a head row.
pub struct HeadRowResponse {
    /// Response for the row.
    pub row: egui::Response,
    /// The response for the delete button (only present for named rows).
    pub delete: Option<egui::Response>,
}

/// The type of row being rendered.
pub enum HeadRowType<'a> {
    /// A named graph (branch).
    Named(&'a str),
    /// An unnamed commit (addressed by timestamp).
    Unnamed(&'a gantz_ca::Timestamp),
}

/// Render a single head/commit row.
///
/// Shows the name/timestamp, CA address, open/focused indicators.
/// Returns responses for click interaction.
pub fn head_row(
    open_heads: &[gantz_ca::Head],
    head: &gantz_ca::Head,
    row_type: HeadRowType,
    row_ca: &gantz_ca::CommitAddr,
    focused_head: Option<usize>,
    ui: &mut egui::Ui,
) -> HeadRowResponse {
    let w = ui.max_rect().width();
    let h = ui.style().interaction.interact_radius;
    let size = egui::Vec2::new(w, h);
    let (rect, mut row) = ui.allocate_at_least(size, egui::Sense::click());

    let builder = egui::UiBuilder::new()
        .sense(egui::Sense::click())
        .max_rect(rect);
    let (res, delete) = ui
        .scope_builder(builder, |ui| {
            let mut res = ui.response();
            let hovered = res.hovered();

            // Create a child UI for the labels positioned over the allocated rect
            ui.horizontal(|ui| {
                let mut name = match row_type {
                    HeadRowType::Named(name) => name.to_string(),
                    HeadRowType::Unnamed(&timestamp) => fmt_commit_timestamp(timestamp),
                };
                // Append focus indicator if this head is focused.
                if let Some(focused) = focused_head {
                    if crate::head_is_focused(open_heads.iter(), focused, head) {
                        name.push_str(" ⚫");
                    }
                }
                let mut text = egui::RichText::new(name.clone());
                let is_open = open_heads.contains(head);
                text = if is_open {
                    text.strong()
                } else if hovered {
                    text
                } else {
                    text.weak()
                };
                let label = egui::Label::new(text).selectable(false);
                res |= ui.add(label);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Show the address.
                    let row_ca_string = format!("{}", row_ca.display_short());
                    let mut text = egui::RichText::new(row_ca_string).monospace();
                    text = if is_open {
                        text.strong()
                    } else if hovered {
                        text
                    } else {
                        text.weak()
                    };
                    let label = egui::Label::new(text).selectable(false);
                    res |= ui.add(label);

                    // Show an x for removing the name mapping.
                    let delete = match row_type {
                        HeadRowType::Named(_) => {
                            Some(ui.add(egui::Button::new("×").frame_when_inactive(false)))
                        }
                        HeadRowType::Unnamed(_) => None,
                    };

                    (res, delete)
                })
                .inner
            })
            .inner
        })
        .inner;

    row |= res;

    HeadRowResponse { row, delete }
}

/// Format the commit as a timestamp for listing unnamed commits.
pub fn fmt_commit_timestamp(timestamp: gantz_ca::Timestamp) -> String {
    std::time::UNIX_EPOCH
        .checked_add(timestamp)
        .map(|system_time| crate::widget::format_local_datetime(system_time))
        .unwrap_or_else(|| "<invalid-timestamp>".to_string())
}
