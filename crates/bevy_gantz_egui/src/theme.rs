//! Follow the system's light or dark preference.
//!
//! `bevy_egui` reports no system theme, so egui's `System` theme preference
//! falls back to [`egui::Options::fallback_theme`]. These systems keep that
//! fallback at the system's preference. On Linux the preference comes from
//! the desktop's settings portal, as winit does not read it there. Elsewhere
//! it comes from the primary window's theme.

use bevy_ecs::prelude::*;
use bevy_egui::{EguiContext, egui};

/// The system's light or dark preference, once known.
#[derive(Default, Resource)]
pub struct SystemTheme(pub Option<egui::Theme>);

/// The receiving end of the settings portal's color scheme updates.
#[cfg(target_os = "linux")]
#[derive(Resource)]
struct PortalTheme(std::sync::Mutex<std::sync::mpsc::Receiver<egui::Theme>>);

/// Add the systems that keep [`SystemTheme`] and the egui contexts' fallback
/// theme at the system's preference.
pub(crate) fn build(app: &mut bevy_app::App) {
    app.init_resource::<SystemTheme>()
        .add_systems(bevy_app::PostUpdate, apply_system_theme);
    #[cfg(target_os = "linux")]
    app.add_systems(bevy_app::Startup, watch_portal)
        .add_systems(bevy_app::PreUpdate, read_portal);
    #[cfg(not(target_os = "linux"))]
    app.add_systems(bevy_app::PreUpdate, read_window_theme);
}

/// Read the desktop's color scheme, then follow its changes, on the IO task
/// pool. A desktop without the portal leaves the theme unknown.
#[cfg(target_os = "linux")]
fn watch_portal(mut cmds: Commands) {
    use ashpd::desktop::settings::{ColorScheme, Settings};
    use bevy_tasks::futures_lite::StreamExt;
    // A desktop with no preference shows its apps light.
    let theme = |scheme| match scheme {
        ColorScheme::PreferDark => egui::Theme::Dark,
        ColorScheme::PreferLight | ColorScheme::NoPreference => egui::Theme::Light,
    };
    let (tx, rx) = std::sync::mpsc::channel();
    bevy_tasks::IoTaskPool::get()
        .spawn(async move {
            let Ok(settings) = Settings::new().await else {
                return;
            };
            if let Ok(scheme) = settings.color_scheme().await {
                if tx.send(theme(scheme)).is_err() {
                    return;
                }
            }
            let Ok(mut changes) = settings.receive_color_scheme_changed().await else {
                return;
            };
            while let Some(scheme) = changes.next().await {
                if tx.send(theme(scheme)).is_err() {
                    return;
                }
            }
        })
        .detach();
    cmds.insert_resource(PortalTheme(std::sync::Mutex::new(rx)));
}

#[cfg(target_os = "linux")]
fn read_portal(portal: Res<PortalTheme>, mut system: ResMut<SystemTheme>) {
    let Ok(rx) = portal.0.lock() else {
        return;
    };
    if let Some(theme) = rx.try_iter().last() {
        system.0 = Some(theme);
    }
}

#[cfg(not(target_os = "linux"))]
fn read_window_theme(
    windows: Query<&bevy_window::Window, With<bevy_window::PrimaryWindow>>,
    mut system: ResMut<SystemTheme>,
) {
    let theme = windows
        .single()
        .ok()
        .and_then(|w| w.window_theme)
        .map(|t| match t {
            bevy_window::WindowTheme::Dark => egui::Theme::Dark,
            bevy_window::WindowTheme::Light => egui::Theme::Light,
        });
    if theme.is_some() && system.0 != theme {
        system.0 = theme;
    }
}

/// Give every egui context the system's theme as its fallback, including the
/// contexts of windows opened since.
fn apply_system_theme(system: Res<SystemTheme>, mut contexts: Query<&mut EguiContext>) {
    let Some(theme) = system.0 else {
        return;
    };
    for mut context in &mut contexts {
        let ctx = context.get_mut();
        if ctx.options(|o| o.fallback_theme) != theme {
            ctx.options_mut(|o| o.fallback_theme = theme);
            ctx.request_repaint();
        }
    }
}
