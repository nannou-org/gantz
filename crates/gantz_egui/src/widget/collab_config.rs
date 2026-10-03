//! The Collab settings subtab. Identity, the vault link, username, action
//! rate and relay configuration. Joining a session lives with the graphs it
//! creates, in the Graphs pane's join button.

use crate::collab::{CollabConfig, SessionConn, VaultDisplay, VaultState};
use crate::{CheckVault, Responses};

/// The inputs for [`collab_config`].
pub struct CollabSettings<'a> {
    /// The user-editable, persisted configuration.
    pub config: &'a mut CollabConfig,
    /// This user's public identity, once generated.
    pub peer_id: Option<&'a str>,
    /// The endpoint's home relays and their connection state. Empty until the
    /// collab runtime starts.
    pub relays: &'a [(String, bool)],
    /// The vault link's state, while linked.
    pub vault: Option<&'a VaultDisplay>,
}

/// The Collab settings subtab. Identity, username, action rate and relay
/// configuration.
///
/// Holds a per-frame snapshot of the persisted [`CollabConfig`] plus the
/// user's displayable identity and relay status. Edits apply to the snapshot
/// in place. The full updated [`CollabConfig`] is emitted as a payload for
/// the collab layer to apply.
#[derive(Clone, Debug, Default)]
pub struct CollabSettingsTab {
    /// The editable configuration snapshot.
    pub config: CollabConfig,
    /// This user's public identity as a displayable string, once minted.
    pub peer_id: Option<String>,
    /// The endpoint's home relays and their connection state.
    pub relays: Vec<(String, bool)>,
    /// The vault link's state, while linked.
    pub vault: Option<VaultDisplay>,
}

impl crate::widget::SettingsTab for CollabSettingsTab {
    fn title(&self) -> &str {
        "Collab"
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> Responses {
        let before = self.config.clone();
        let mut responses = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let settings = CollabSettings {
                    config: &mut self.config,
                    peer_id: self.peer_id.as_deref(),
                    relays: &self.relays,
                    vault: self.vault.as_ref(),
                };
                collab_config(settings, ui)
            })
            .inner;
        if self.config != before {
            responses.push(None, self.config.clone());
        }
        responses
    }
}

/// Render the collab configuration. The user's identity, the vault link,
/// their shared username, the live-action send rate and the relay
/// configuration and status. Returns the actions the user took, such as
/// [`CheckVault`].
pub fn collab_config(settings: CollabSettings, ui: &mut egui::Ui) -> Responses {
    let mut responses = Responses::default();
    let CollabSettings {
        config,
        peer_id,
        relays,
        vault,
    } = settings;
    let control_w = (ui.available_width() - 64.0).max(64.0);
    egui::Grid::new("collab_config_grid")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            // The public identity that peers see and can allowlist.
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

            // The vault this device syncs all its named graphs with.
            wrapped_row(ui, "vault", control_w, |ui| match &config.vault {
                Some(_) => {
                    ui.horizontal_wrapped(|ui| {
                        let state = vault.map(|v| v.state.clone()).unwrap_or_default();
                        let hover = vault
                            .map(|v| v.hover_text())
                            .unwrap_or_else(|| state.guidance().to_string());
                        super::status_dot(ui, state.color()).on_hover_text(hover);
                        let id = vault.map(|v| v.vault.as_str()).unwrap_or_default();
                        ui.label(egui::RichText::new(id).weak());
                        let settled = matches!(state, VaultState::Connecting | VaultState::Live);
                        if !settled
                            && ui
                                .button("check again")
                                .on_hover_text("link to the vault again now")
                                .clicked()
                        {
                            responses.push(None, CheckVault);
                        }
                        if ui
                            .button("unlink")
                            .on_hover_text("stop syncing with the vault. Local graphs stay")
                            .clicked()
                        {
                            config.vault = None;
                        }
                    });
                }
                None => {
                    let ticket_id = ui.id().with("collab_vault_ticket");
                    let mut ticket = ui
                        .data(|d| d.get_temp::<String>(ticket_id))
                        .unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut ticket)
                                .hint_text("paste a vault ticket")
                                .desired_width((control_w - 48.0).max(48.0)),
                        )
                        .on_hover_text(
                            "sync all your named graphs with a vault. \
                             `gantz vault serve` prints its ticket",
                        );
                        let ready = ticket.trim().starts_with("gantzvault");
                        if ui.add_enabled(ready, egui::Button::new("link")).clicked() {
                            config.vault = Some(ticket.trim().to_string());
                            ticket.clear();
                        }
                    });
                    ui.data_mut(|d| d.insert_temp(ticket_id, ticket));
                }
            });
            if let Some(vault) = vault.filter(|_| config.vault.is_some()) {
                wrapped_row(ui, "", control_w, |ui| {
                    ui.label(vault.state.guidance());
                    if let Some(reason) = vault.state.reason() {
                        ui.label(egui::RichText::new(reason).weak());
                    }
                });
                wrapped_row(ui, "versions", control_w, |ui| {
                    ui.label(egui::RichText::new(vault.versions()).weak());
                });
                if !vault.failures.is_empty() {
                    wrapped_row(ui, "not synced", control_w, |ui| {
                        for (name, reason) in &vault.failures {
                            ui.label(egui::RichText::new(format!("{name}: {reason}")).weak());
                        }
                    });
                }
                if !vault.newer.is_empty() {
                    wrapped_row(ui, "locked", control_w, |ui| {
                        ui.label(
                            "A newer gantz synced settings that this gantz does not \
                             recognise. Update gantz to edit these graphs.",
                        );
                        ui.label(egui::RichText::new(vault.newer.join(", ")).weak());
                    });
                }
            }

            // The username shared with session peers.
            ui.label("username");
            ui.add(
                egui::TextEdit::singleline(&mut config.username)
                    .hint_text("anonymous")
                    .desired_width(control_w),
            );
            ui.end_row();

            // The per-node-path send window for live actions.
            ui.label("action rate");
            ui.add(
                egui::DragValue::new(&mut config.action_rate_ms)
                    .speed(1)
                    .range(0..=1000)
                    .suffix(" ms"),
            )
            .on_hover_text(
                "minimum interval between live-action sends per node \
                 (drags, bangs); values written faster batch into one \
                 message and replay in order on peers. 0 sends every frame",
            );
            ui.end_row();

            // Presence cursors.
            ui.label("pointers");
            ui.checkbox(&mut config.show_pointers, "show peer pointers")
                .on_hover_text(
                    "show session peers' live pointers over shared graphs; \
                     your own pointer is shared regardless",
                );
            ui.end_row();

            // The relay server that assists connections and carries them for
            // browser peers. Empty means iroh's default n0 public relays.
            ui.label("relay");
            let relay_id = ui.id().with("collab_relay");
            let mut relay = ui
                .data(|d| d.get_temp::<String>(relay_id))
                .unwrap_or_else(|| config.custom_relay.clone().unwrap_or_default());
            ui.horizontal(|ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut relay)
                        .hint_text("n0 public relays (default)")
                        .desired_width((control_w - 48.0).max(48.0)),
                );
                resp.on_hover_text(
                    "a custom relay server URL (e.g. a self-hosted iroh-relay). \
                     Replaces n0's public infrastructure entirely: peers \
                     connect via invite tickets and the relay, with no \
                     third-party address lookup. Applies when the app \
                     restarts",
                );
                if ui
                    .button("reset")
                    .on_hover_text("use the default (n0 public) relays")
                    .clicked()
                {
                    relay.clear();
                }
                let trimmed = relay.trim();
                config.custom_relay = (!trimmed.is_empty()).then(|| trimmed.to_string());
            });
            ui.data_mut(|d| d.insert_temp(relay_id, relay));
            ui.end_row();

            // Live relay status once the collab runtime is up. It shows who
            // this peer is routed through.
            if !relays.is_empty() {
                ui.label("");
                ui.vertical(|ui| {
                    for (url, connected) in relays {
                        ui.horizontal(|ui| {
                            let (color, label) = if *connected {
                                (SessionConn::Live.color(), "connected")
                            } else {
                                (SessionConn::Degraded.color(), "disconnected")
                            };
                            super::status_dot(ui, color).on_hover_text(label);
                            ui.label(egui::RichText::new(url).weak());
                        });
                    }
                });
                ui.end_row();
            }
        });
    responses
}

/// One grid row of `label` and `body`, with the body wrapped to `width`.
fn wrapped_row(ui: &mut egui::Ui, label: &str, width: f32, body: impl FnOnce(&mut egui::Ui)) {
    ui.label(label);
    ui.vertical(|ui| {
        ui.set_max_width(width);
        body(ui);
    });
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The width the settings use in a narrow pane, linked to `vault` if any.
    fn used_width(vault: Option<VaultDisplay>) -> f32 {
        let ctx = egui::Context::default();
        let mut config = CollabConfig {
            vault: vault.as_ref().map(|_| "gantzvault".to_string()),
            ..Default::default()
        };
        let mut used = 0.0;
        // The grid sizes its columns in the first pass.
        for _ in 0..2 {
            let _ = ctx.run_ui(Default::default(), |ui| {
                let settings = CollabSettings {
                    config: &mut config,
                    peer_id: Some("22846021"),
                    relays: &[],
                    vault: vault.as_ref(),
                };
                let scope = ui.scope(|ui| {
                    ui.set_max_width(220.0);
                    collab_config(settings, ui)
                });
                used = scope.response.rect.width();
            });
        }
        used
    }

    // A refused device must reach "unlink" to paste a new ticket. So the
    // vault row fits wherever the other rows fit.
    #[test]
    fn a_refused_vault_link_fits_the_pane() {
        let refused = VaultDisplay {
            vault: "8b51cfb9".to_string(),
            state: VaultState::Denied("access denied".to_string()),
            ..Default::default()
        };
        let unlinked = used_width(None);
        let linked = used_width(Some(refused));
        assert!(
            linked <= unlinked,
            "linked uses {linked}, unlinked {unlinked}"
        );
    }
}
