//! The DSP settings subtab: the dsp runtime's status plus a couple of live
//! knobs (scheduling lead and mute).

use crate::{Config, Status};
use gantz_egui::Responses;

/// The DSP settings subtab: a read-only status readout plus a live
/// scheduling-lead control and an enable/mute toggle.
///
/// Holds a per-frame snapshot of the domain's [`Config`] and [`Status`].
/// Edits apply to the snapshot in place, and the full updated [`Config`] is
/// emitted as a payload for the host to apply.
#[derive(Clone, Debug, Default)]
pub struct DspSettingsTab {
    /// The editable settings snapshot.
    pub config: Config,
    /// The read-only runtime status snapshot.
    pub status: Status,
}

impl gantz_egui::widget::SettingsTab for DspSettingsTab {
    fn title(&self) -> &str {
        "DSP"
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> Responses {
        let mut responses = Responses::default();

        ui.label("Status:");
        if self.status.present {
            egui::Grid::new("dsp_status_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Device");
                    ui.label(self.status.device.as_deref().unwrap_or("-"));
                    ui.end_row();
                    ui.label("Sample rate");
                    ui.label(format!("{:.0} Hz", self.status.sample_rate));
                    ui.end_row();
                    ui.label("Channels");
                    ui.label(self.status.channels.to_string());
                    ui.end_row();
                });
        } else {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "No DSP output device — running silent.",
            );
        }

        ui.separator();

        // The live controls only do anything when a device is present.
        ui.add_enabled_ui(self.status.present, |ui| {
            let mut changed = false;

            if ui
                .checkbox(&mut self.config.enabled, "DSP enabled")
                .on_hover_text("Mute/unmute DSP output (pauses the output stream).")
                .changed()
            {
                changed = true;
            }

            let mut lead_ms = self.config.sched_lead.as_secs_f32() * 1000.0;
            ui.horizontal(|ui| {
                let dv = egui::DragValue::new(&mut lead_ms)
                    .range(0.0..=500.0)
                    .speed(1.0)
                    .suffix(" ms");
                if ui
                    .add(dv)
                    .on_hover_text(
                        "Scheduling lead: how far ahead control automation is scheduled. \
                         Higher = safer timing but more latency; must exceed output latency \
                         or timing degrades to block granularity.",
                    )
                    .changed()
                {
                    self.config.sched_lead = std::time::Duration::from_secs_f32(lead_ms / 1000.0);
                    changed = true;
                }
                ui.label("Scheduling lead");
            });

            if changed {
                responses.push(None, self.config.clone());
            }
        });

        responses
    }
}
