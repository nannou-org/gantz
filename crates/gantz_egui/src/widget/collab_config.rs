//! The "Settings > Collab" subtab: identity, username and joining sessions.

use crate::Responses;
use crate::collab::CollabConfig;

/// The Collab settings subtab: identity, username and joining sessions.
///
/// Holds a per-frame snapshot of the persisted [`CollabConfig`] plus the
/// user's displayable identity. Edits apply to the snapshot in place, and
/// the full updated [`CollabConfig`] is emitted as a payload for the collab
/// layer to apply; a submitted invite ticket is emitted as a
/// [`JoinSession`][crate::JoinSession] payload.
#[derive(Clone, Debug, Default)]
pub struct CollabSettingsTab {
    /// The editable configuration snapshot.
    pub config: CollabConfig,
    /// This user's public identity, as a displayable string, once minted.
    pub peer_id: Option<String>,
}

/// Response from [`collab_config`].
#[derive(Default)]
pub struct CollabConfigResponse {
    /// A ticket was submitted via the join field.
    pub join_ticket: Option<String>,
}

impl crate::widget::SettingsTab for CollabSettingsTab {
    fn title(&self) -> &str {
        "Collab"
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> Responses {
        let mut responses = Responses::default();
        let before = self.config.clone();
        let res = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                collab_config(&mut self.config, self.peer_id.as_deref(), ui)
            })
            .inner;
        if let Some(ticket) = res.join_ticket {
            responses.push(None, crate::JoinSession { ticket });
        }
        if self.config != before {
            responses.push(None, self.config.clone());
        }
        responses
    }
}

/// Render the collab configuration: the user's identity, their shared
/// username, and a field for joining a session from an invite ticket.
pub fn collab_config(
    config: &mut CollabConfig,
    peer_id: Option<&str>,
    ui: &mut egui::Ui,
) -> CollabConfigResponse {
    let mut res = CollabConfigResponse::default();
    let control_w = (ui.available_width() - 64.0).max(64.0);
    egui::Grid::new("collab_config_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            // The public identity peers see (and can allowlist).
            ui.label("identity");
            match peer_id {
                Some(id) => {
                    let short: String = id.chars().take(8).collect();
                    if ui
                        .button(format!("{short}…"))
                        .on_hover_text(format!("copy full public key\n{id}"))
                        .clicked()
                    {
                        ui.ctx().copy_text(id.to_string());
                    }
                }
                None => {
                    ui.label(
                        egui::RichText::new("generated when first shared")
                            .italics()
                            .weak(),
                    );
                }
            }
            ui.end_row();

            // The username shared with session peers.
            ui.label("username");
            ui.add(
                egui::TextEdit::singleline(&mut config.username)
                    .hint_text("anonymous")
                    .desired_width(control_w),
            );
            ui.end_row();

            // Join a session from a pasted invite ticket.
            ui.label("join");
            let ticket_id = ui.id().with("collab_join_ticket");
            let mut ticket = ui
                .data(|d| d.get_temp::<String>(ticket_id))
                .unwrap_or_default();
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut ticket)
                        .hint_text("paste an invite ticket")
                        .desired_width((control_w - 48.0).max(48.0)),
                );
                let ready = !ticket.trim().is_empty();
                if ui.add_enabled(ready, egui::Button::new("join")).clicked() {
                    res.join_ticket = Some(ticket.trim().to_string());
                    ticket.clear();
                }
            });
            ui.data_mut(|d| d.insert_temp(ticket_id, ticket));
            ui.end_row();
        });
    res
}
