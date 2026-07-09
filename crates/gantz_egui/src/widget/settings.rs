//! The "Settings" sidebar tab: globally-relevant configuration grouped into
//! Global / Style / Keybinds / Panes subtabs, plus any application-supplied
//! extension subtabs (see [`SettingsTab`]).

use super::gantz::{LayoutConfig, SceneConfig, ViewToggles};
use crate::{Keymap, Responses};

/// An application-supplied settings subtab.
///
/// Domains contribute their own settings UI by supplying implementations to
/// the [`Gantz`][super::Gantz] widget via
/// [`Gantz::settings_tabs`][super::Gantz::settings_tabs]. A tab typically
/// holds a per-frame snapshot of the domain's config and status, edits the
/// snapshot in place, and reports changes by pushing a typed payload into the
/// returned [`Responses`] for the host to apply.
pub trait SettingsTab {
    /// The subtab's label. Also identifies the selected subtab, so it should
    /// be unique among the supplied tabs and stable across frames.
    fn title(&self) -> &str;

    /// Render the subtab's contents, returning any change payloads.
    fn ui(&mut self, ui: &mut egui::Ui) -> Responses;
}

/// Which settings subtab is selected. Persisted only within a session.
#[derive(Clone, Default, PartialEq, Eq)]
enum SubTab {
    #[default]
    Global,
    Style,
    Keybinds,
    Panes,
    /// An extension subtab, identified by its title.
    Ext(String),
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
    /// Payloads emitted by extension subtabs.
    pub responses: Responses,
}

/// Render the Settings pane: a subtab selector over Global / Style / Keybinds /
/// Panes and any supplied extension subtabs.
pub fn settings(
    view: &mut ViewToggles,
    compile_config: Option<gantz_core::compile::Config>,
    validate_change_tracking: Option<bool>,
    layout_config: &mut LayoutConfig,
    scene_config: &mut SceneConfig,
    keymap: &mut Keymap,
    ext_tabs: &mut [&mut dyn SettingsTab],
    ui: &mut egui::Ui,
) -> SettingsResponse {
    let id = ui.id().with("settings_subtab");
    let mut tab = ui.data(|d| d.get_temp::<SubTab>(id)).unwrap_or_default();

    // A previously selected extension subtab may no longer be supplied.
    if let SubTab::Ext(ref name) = tab {
        if !ext_tabs.iter().any(|t| t.title() == name) {
            tab = SubTab::Global;
        }
    }

    // Subtab selector rendered like the shared tab widget: plain labels (no
    // box), the active tab in the strong text colour and the rest dim.
    ui.horizontal(|ui| {
        let mut tab_label = |ui: &mut egui::Ui, this: SubTab, label: &str| {
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
        tab_label(ui, SubTab::Global, "Global");
        tab_label(ui, SubTab::Style, "Style");
        tab_label(ui, SubTab::Keybinds, "Keybinds");
        tab_label(ui, SubTab::Panes, "Panes");
        // Extension subtabs only exist while the app supplies them.
        for t in ext_tabs.iter() {
            let title = t.title();
            tab_label(ui, SubTab::Ext(title.to_string()), title);
        }
    });
    ui.separator();

    let mut res = SettingsResponse::default();
    match tab {
        SubTab::Panes => {
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
        SubTab::Style => {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| super::style_config(&mut scene_config.grid, ui));
        }
        SubTab::Keybinds => {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| super::keybinds_config(keymap, ui));
        }
        SubTab::Global => {
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
        SubTab::Ext(ref name) => {
            if let Some(t) = ext_tabs.iter_mut().find(|t| t.title() == name) {
                res.responses = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| t.ui(ui))
                    .inner;
            }
        }
    }

    ui.data_mut(|d| d.insert_temp(id, tab));
    res
}
