//! The "DSP" top-level pane. It shows the focused head's derive status and a
//! readable rendering of its derived program. See
//! [`describe_parts`][crate::describe_parts].

use std::collections::HashMap;
use std::sync::Arc;

use gantz_egui::Responses;

use crate::DeriveStatus;

/// The "DSP" pane's stable [`ExtPane::key`][gantz_egui::widget::ExtPane::key].
pub const DSP_PANE_KEY: &str = "dsp";

/// The "DSP" top-level pane, the domain's analogue of the built-in Steel
/// pane. It shows the focused head's derive status and the derived program
/// as text on success.
///
/// Holds a per-frame snapshot of every open head's status so the pane follows
/// whichever head is focused. Supplied to the
/// [`Gantz`][gantz_egui::widget::Gantz] widget via
/// [`ExtPane`][gantz_egui::widget::ExtPane].
pub struct DspPane {
    /// Whether a DSP output device is present. Without one the app runs
    /// silent.
    pub present: bool,
    /// Each open head's snapshot.
    pub heads: HashMap<gantz_ca::Head, DspPaneHead>,
}

/// One head's snapshot within [`DspPane`].
#[derive(Clone, Debug, Default)]
pub struct DspPaneHead {
    /// The most recent derivation's outcome.
    pub status: DeriveStatus,
    /// The derived program rendered as text, or the failure message.
    pub view: Arc<str>,
}

impl gantz_egui::widget::ExtPane for DspPane {
    fn key(&self) -> &str {
        DSP_PANE_KEY
    }

    fn title(&self) -> &str {
        "DSP"
    }

    fn description(&self) -> &str {
        "The focused graph's DSP derive status and derived program."
    }

    fn ui(&mut self, cx: gantz_egui::widget::ExtPaneCtx, ui: &mut egui::Ui) -> Responses {
        if !self.present {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "No DSP output device - running silent.",
            );
        }
        let Some(head) = cx.focused else {
            ui.weak("no focused graph");
            return Responses::default();
        };
        let snapshot = self.heads.get(head).cloned().unwrap_or_default();
        derive_status_label(&snapshot.status, ui);
        if let DeriveStatus::Ok { .. } = snapshot.status {
            ui.separator();
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&*snapshot.view).monospace()).extend(),
                    );
                });
        }
        Responses::default()
    }
}

/// One line summarising a [`DeriveStatus`]. Pending and silent are weak, ok
/// is plain and failures are error-coloured with the full message on hover.
/// Shared by the DSP pane and the settings tab's per-head grid.
pub(crate) fn derive_status_label(status: &DeriveStatus, ui: &mut egui::Ui) {
    match status {
        DeriveStatus::Pending => {
            ui.weak("pending - nothing derived yet");
        }
        DeriveStatus::Silent => {
            ui.weak("silent - no dsp sink (~out / ~scopeout)");
        }
        DeriveStatus::Ok { parts } => {
            ui.label(format!("ok - {parts} part(s)"));
        }
        DeriveStatus::FlattenError(e)
        | DeriveStatus::DeriveError(e)
        | DeriveStatus::SpawnError(e) => {
            // Errors can be long, so truncate to the available width.
            let text = egui::RichText::new(e).color(ui.visuals().error_fg_color);
            ui.add(egui::Label::new(text).truncate()).on_hover_text(e);
        }
    }
}
