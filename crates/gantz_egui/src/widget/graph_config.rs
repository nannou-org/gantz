//! A widget for configuring per-head graph layout settings and renaming.

use super::gantz::OpenHeadState;

/// Per-head graph configuration widget.
///
/// Provides a name-editing text field and layout settings
/// (`auto_layout`, `layout_flow`, `center_view`).
pub struct GraphConfig<'a> {
    head: &'a gantz_ca::Head,
    head_state: &'a mut OpenHeadState,
    names: &'a gantz_ca::registry::Names,
}

/// Response from the [`GraphConfig`] widget.
pub struct GraphConfigResponse {
    /// A new branch name was committed via the name editor.
    pub new_branch: Option<(gantz_ca::Head, String)>,
}

impl<'a> GraphConfig<'a> {
    pub fn new(
        head: &'a gantz_ca::Head,
        head_state: &'a mut OpenHeadState,
        names: &'a gantz_ca::registry::Names,
    ) -> Self {
        Self {
            head,
            head_state,
            names,
        }
    }

    pub fn show(self, ui: &mut egui::Ui) -> GraphConfigResponse {
        let mut new_branch = None;

        // Name editing TextEdit.
        let edit_id = egui::Id::new("graph_config_name_edit").with(self.head);
        let mut name = ui
            .memory_mut(|m| m.data.get_temp::<String>(edit_id))
            .unwrap_or_else(|| match self.head {
                gantz_ca::Head::Branch(name) => name.clone(),
                gantz_ca::Head::Commit(_) => String::new(),
            });

        let name_exists = self.names.contains_key(&name);
        let is_current_name = matches!(self.head, gantz_ca::Head::Branch(n) if *n == name);
        let is_empty = name.is_empty();
        let is_invalid = is_empty || (name_exists && !is_current_name);

        let text_color = if is_invalid && !is_current_name {
            egui::Color32::RED
        } else {
            ui.visuals().text_color()
        };

        let text_edit = egui::TextEdit::singleline(&mut name)
            .desired_width(ui.available_width())
            .text_color(text_color)
            .hint_text("name");
        let te_response = ui.add(text_edit);

        // On Enter or focus loss, attempt to commit the name.
        let enter_pressed =
            te_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let focus_lost =
            te_response.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Escape));

        if enter_pressed || focus_lost {
            if !is_empty && !is_invalid {
                new_branch = Some((self.head.clone(), name.clone()));
            }
            // Reset to the current head name.
            let current = head_name(self.head);
            ui.memory_mut(|m| m.data.insert_temp(edit_id, current));
        } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Cancel: reset to current head name.
            let current = head_name(self.head);
            ui.memory_mut(|m| m.data.insert_temp(edit_id, current));
        } else {
            ui.memory_mut(|m| m.data.insert_temp(edit_id, name));
        }

        // Layout config.
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.head_state.auto_layout, "Automatic Layout");
        });
        ui.checkbox(&mut self.head_state.center_view, "Center View");
        ui.horizontal(|ui| {
            ui.label("Flow:");
            ui.radio_value(
                &mut self.head_state.layout_flow,
                egui::Direction::LeftToRight,
                "Right",
            );
            ui.radio_value(
                &mut self.head_state.layout_flow,
                egui::Direction::TopDown,
                "Down",
            );
        });

        GraphConfigResponse { new_branch }
    }
}

fn head_name(head: &gantz_ca::Head) -> String {
    match head {
        gantz_ca::Head::Branch(n) => n.clone(),
        gantz_ca::Head::Commit(_) => String::new(),
    }
}
