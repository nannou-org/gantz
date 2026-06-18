//! The "Settings" sidebar tab: globally-relevant configuration grouped into
//! Panes / Style / Global subtabs.

use super::gantz::ViewToggles;

/// Which settings subtab is selected. Persisted only within a session.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SettingsTab {
    #[default]
    Panes,
    Style,
    Global,
}

/// Response from [`settings`].
#[derive(Default)]
pub struct SettingsResponse {
    /// The global compile config was changed (Global subtab).
    pub compile_config: Option<gantz_core::compile::Config>,
    /// The "Reset all demos" button was clicked (Global subtab).
    pub reset_all_demos: bool,
    /// The "reset all" layout button was clicked (Panes subtab).
    pub reset_layout: bool,
}

/// Render the Settings pane: a subtab selector over Panes / Style / Global.
pub fn settings(
    view: &mut ViewToggles,
    compile_config: Option<gantz_core::compile::Config>,
    ui: &mut egui::Ui,
) -> SettingsResponse {
    let id = ui.id().with("settings_subtab");
    let mut tab = ui
        .data(|d| d.get_temp::<SettingsTab>(id))
        .unwrap_or_default();

    ui.horizontal(|ui| {
        ui.selectable_value(&mut tab, SettingsTab::Global, "Global");
        ui.selectable_value(&mut tab, SettingsTab::Style, "Style");
        ui.selectable_value(&mut tab, SettingsTab::Panes, "Panes");
    });
    ui.separator();

    let mut res = SettingsResponse::default();
    match tab {
        SettingsTab::Panes => {
            // Pin "reset all" to the bottom; the toggles scroll above it.
            // `Frame::NONE` keeps the inner margin matching the other subtabs,
            // which render directly in the pane's central panel.
            egui::TopBottomPanel::bottom(id.with("reset"))
                .show_separator_line(false)
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    res.reset_layout = super::reset_layout_button(ui);
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| super::panes_config(view, ui));
                });
        }
        SettingsTab::Style => {
            ui.weak("Style configuration coming soon.");
        }
        SettingsTab::Global => {
            let g = super::global_config(compile_config, ui);
            res.compile_config = g.compile_config;
            res.reset_all_demos = g.reset_all_demos;
        }
    }

    ui.data_mut(|d| d.insert_temp(id, tab));
    res
}
