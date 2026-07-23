//! The "Settings > Collab" subtab: identity, username, action rate and
//! relay configuration. Joining a session lives with the graphs it creates:
//! the Graphs pane's join button.

use crate::Responses;
use crate::collab::{CollabConfig, SessionConn};

/// The inputs for [`collab_config`].
pub struct CollabSettings<'a> {
    /// The user-editable, persisted configuration.
    pub config: &'a mut CollabConfig,
    /// This user's public identity, once generated.
    pub peer_id: Option<&'a str>,
    /// The endpoint's home relay(s) and their connection state (empty until
    /// the collab runtime starts).
    pub relays: &'a [(String, bool)],
}

/// The Collab settings subtab: identity, username, action rate and relay
/// configuration.
///
/// Holds a per-frame snapshot of the persisted [`CollabConfig`] plus the
/// user's displayable identity and relay status. Edits apply to the snapshot
/// in place, and the full updated [`CollabConfig`] is emitted as a payload
/// for the collab layer to apply.
#[derive(Clone, Debug, Default)]
pub struct CollabSettingsTab {
    /// The editable configuration snapshot.
    pub config: CollabConfig,
    /// This user's public identity, as a displayable string, once minted.
    pub peer_id: Option<String>,
    /// The endpoint's home relay(s) and their connection state.
    pub relays: Vec<(String, bool)>,
}

impl crate::widget::SettingsTab for CollabSettingsTab {
    fn title(&self) -> &str {
        "Collab"
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> Responses {
        let mut responses = Responses::default();
        let before = self.config.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let settings = CollabSettings {
                    config: &mut self.config,
                    peer_id: self.peer_id.as_deref(),
                    relays: &self.relays,
                };
                collab_config(settings, ui)
            });
        if self.config != before {
            responses.push(None, self.config.clone());
        }
        responses
    }
}

/// Render the collab configuration: the user's identity, their shared
/// username, the live-action send rate and the relay configuration/status.
pub fn collab_config(settings: CollabSettings, ui: &mut egui::Ui) {
    let CollabSettings {
        config,
        peer_id,
        relays,
    } = settings;
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

            // The relay server assisting (and, for browser peers, carrying)
            // connections. Empty = iroh's default n0 public relays.
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

            // Live relay status, once the collab runtime is up: who this
            // peer is routed through.
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
}
