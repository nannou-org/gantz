//! The "Settings" sidebar tab: globally-relevant configuration grouped into
//! Panes / Style / Global subtabs.

use super::gantz::{LayoutConfig, SceneConfig, ViewToggles};

/// Which settings subtab is selected. Persisted only within a session.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SettingsTab {
    #[default]
    Global,
    Style,
    Panes,
}

/// Response from [`settings`].
#[derive(Default)]
pub struct SettingsResponse {
    /// The global compile config was changed (Global subtab).
    pub compile_config: Option<gantz_core::compile::Config>,
    /// The change-tracking validation toggle was changed (Global subtab).
    pub validate_change_tracking: Option<bool>,
    /// The "Reset all demos" button was clicked (Global subtab).
    pub reset_all_demos: bool,
    /// The "reset all" layout button was clicked (Panes subtab).
    pub reset_layout: bool,
}

/// Render the Settings pane: a subtab selector over Panes / Style / Global.
pub fn settings(
    view: &mut ViewToggles,
    compile_config: Option<gantz_core::compile::Config>,
    validate_change_tracking: Option<bool>,
    layout_config: &mut LayoutConfig,
    scene_config: &mut SceneConfig,
    ui: &mut egui::Ui,
) -> SettingsResponse {
    let id = ui.id().with("settings_subtab");
    let mut tab = ui
        .data(|d| d.get_temp::<SettingsTab>(id))
        .unwrap_or_default();

    // Subtab selector rendered like the shared tab widget: plain labels (no
    // box), the active tab in the strong text colour and the rest dim.
    ui.horizontal(|ui| {
        let mut tab_label = |ui: &mut egui::Ui, this: SettingsTab, label: &str| {
            let color = if tab == this {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            let resp = ui
                .add(
                    egui::Label::new(egui::RichText::new(label).color(color))
                        .sense(egui::Sense::click())
                        .selectable(false),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                tab = this;
            }
        };
        tab_label(ui, SettingsTab::Global, "Global");
        tab_label(ui, SettingsTab::Style, "Style");
        tab_label(ui, SettingsTab::Panes, "Panes");
    });
    ui.separator();

    let mut res = SettingsResponse::default();
    match tab {
        SettingsTab::Panes => {
            // Pin "reset all" to the bottom; the toggles scroll above it.
            // `Frame::NONE` keeps the inner margin matching the other subtabs,
            // which render directly in the pane's central panel.
            egui::Panel::bottom(id.with("reset"))
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
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| super::style_config(&mut scene_config.grid, ui));
        }
        SettingsTab::Global => {
            let g = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    super::global_config(
                        compile_config,
                        validate_change_tracking,
                        layout_config,
                        &mut scene_config.snap,
                        &mut scene_config.align,
                        ui,
                    )
                })
                .inner;
            res.compile_config = g.compile_config;
            res.validate_change_tracking = g.validate_change_tracking;
            res.reset_all_demos = g.reset_all_demos;
        }
    }

    ui.data_mut(|d| d.insert_temp(id, tab));
    res
}
