//! A widget for configuring per-head graph layout settings and renaming.

use super::gantz::OpenHeadState;
use super::head_name_edit::{head_name, head_name_edit};

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
        // Name editing TextEdit with per-head temp state.
        let edit_id = egui::Id::new("graph_config_name_edit").with(self.head);
        let mut name = ui
            .memory_mut(|m| m.data.get_temp::<String>(edit_id))
            .unwrap_or_else(|| head_name(self.head));
        let name_res = head_name_edit(self.head, &mut name, self.names, ui);
        ui.memory_mut(|m| m.data.insert_temp(edit_id, name));
        let new_branch = name_res.new_branch;

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
