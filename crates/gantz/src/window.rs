//! Primary window setup, native window-size persistence and decorations.
//!
//! On native platforms the window's size is restored on launch and persisted
//! via the same [`bevy_pkv`]-backed store as the rest of the app state. The GUI
//! draws the window's decorations in place of the system's, unless the user
//! picks the system's. On web `fit_canvas_to_parent` governs the window, so the
//! canvas fills its HTML container and a fixed size is not persisted.

use bevy::{
    prelude::*,
    window::{Window, WindowPlugin},
};
use gantz_store::{Load, Save};
#[cfg(not(target_arch = "wasm32"))]
use serde::{Deserialize, Serialize};

/// The persisted size of the primary window, in logical pixels.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

/// The storage key under which the window size is persisted.
#[cfg(not(target_arch = "wasm32"))]
const KEY: &str = "window-size";

/// Build the [`WindowPlugin`] for the primary window.
pub fn plugin() -> WindowPlugin {
    WindowPlugin {
        primary_window: Some(Window {
            title: "gantz".into(),
            name: Some("gantz".into()),
            fit_canvas_to_parent: true,
            // This vastly improves input latency on wayland. If you notice
            // tearing or similar issues, open an issue so the right
            // `PresentMode` can be selected for each system.
            present_mode: bevy::window::PresentMode::AutoNoVsync,
            // On web, let the browser's default keydown handling proceed.
            // winit otherwise calls `preventDefault()` on every keydown, which
            // cancels the browser's paste event. That event is the only source
            // of paste input on wasm, because bevy_egui's Ctrl+V clipboard
            // fallback is compiled out there. eframe ships the same policy.
            // Small JS listeners in `web/index.html` guard the unwanted
            // side-effects such as the native context menu.
            #[cfg(target_arch = "wasm32")]
            prevent_default_event_handling: false,
            // On native the GUI decorates the window itself, so the window
            // starts without the system's decorations and is transparent
            // beyond its rounded corners. See [`DecorationsPlugin`].
            #[cfg(not(target_arch = "wasm32"))]
            decorations: false,
            #[cfg(not(target_arch = "wasm32"))]
            transparent: true,
            #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
            composite_alpha_mode: bevy::window::CompositeAlphaMode::PostMultiplied,
            #[cfg(all(not(target_arch = "wasm32"), not(target_os = "macos")))]
            composite_alpha_mode: bevy::window::CompositeAlphaMode::PreMultiplied,
            ..default()
        }),
        ..default()
    }
}

/// Apply the persisted window size to `window`, if one was saved.
///
/// Called from a `Startup` system. No-op on web, where the window size is not
/// persisted.
pub fn apply_saved_size(storage: &impl Load, window: &mut Window) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(size) = gantz_store::load::<WindowSize>(storage, KEY) {
        window.resolution.set(size.width, size.height);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (storage, window);
}

/// Persist the primary window's logical size to storage.
///
/// No-op on web. Also skips the write before the window surface is realized,
/// when the reported size is still effectively zero.
pub fn save(storage: &mut impl Save, window: &Window) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let width = window.resolution.width();
        let height = window.resolution.height();
        if width >= 1.0 && height >= 1.0 {
            gantz_store::save(storage, KEY, &WindowSize { width, height });
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (storage, window);
}

/// Lets the GUI decorate the primary window in place of the system, on
/// native platforms. The GUI's window style picks the system's decorations or
/// its own. See [`bevy_gantz_egui::DecoratedWindow`].
#[cfg(not(target_arch = "wasm32"))]
pub struct DecorationsPlugin;

#[cfg(not(target_arch = "wasm32"))]
impl Plugin for DecorationsPlugin {
    fn build(&self, app: &mut App) {
        // The GUI paints the window's ground, so the window clears to transparent.
        app.insert_resource(ClearColor(Color::NONE))
            .init_resource::<bevy_gantz_egui::DecoratedWindow>()
            .add_observer(on_window_action)
            .add_systems(Update, (track_maximized, sync_decorations));
    }
}

/// Keep [`bevy_gantz_egui::DecoratedWindow`] up to date with whether the
/// primary window is maximised or fullscreen. Bevy's `Window` does not report
/// it, so it is read from winit, on the main thread.
#[cfg(not(target_arch = "wasm32"))]
fn track_maximized(
    _main_thread: bevy::ecs::system::NonSendMarker,
    windows: Query<Entity, With<bevy::window::PrimaryWindow>>,
    mut decorated: ResMut<bevy_gantz_egui::DecoratedWindow>,
) {
    let Ok(entity) = windows.single() else {
        return;
    };
    let maximized = bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        windows
            .get_window(entity)
            .is_some_and(|w| w.is_maximized() || w.fullscreen().is_some())
    });
    if decorated.0.maximized != maximized {
        decorated.0.maximized = maximized;
    }
}

/// Give the primary window the system's decorations only while the GUI's
/// window style asks for them.
#[cfg(not(target_arch = "wasm32"))]
fn sync_decorations(
    gui: Res<bevy_gantz_egui::GuiState>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
) {
    let system = gui.view_toggles.window == gantz_egui::widget::WindowStyle::Native;
    for mut window in &mut windows {
        if window.decorations != system {
            window.decorations = system;
        }
    }
}

/// Apply a window action from the GUI to the primary window.
#[cfg(not(target_arch = "wasm32"))]
fn on_window_action(
    trigger: On<bevy_gantz_egui::ForApp<gantz_egui::widget::WindowAction>>,
    decorated: Res<bevy_gantz_egui::DecoratedWindow>,
    mut windows: Query<(Entity, &mut Window), With<bevy::window::PrimaryWindow>>,
    mut close: MessageWriter<bevy::window::WindowCloseRequested>,
) {
    use gantz_egui::widget::WindowAction;
    let Ok((entity, mut window)) = windows.single_mut() else {
        return;
    };
    match trigger.event().0 {
        WindowAction::Minimize => window.set_minimized(true),
        WindowAction::ToggleMaximize => window.set_maximized(!decorated.0.maximized),
        WindowAction::Close => {
            close.write(bevy::window::WindowCloseRequested { window: entity });
        }
        WindowAction::Drag => window.start_drag_move(),
        WindowAction::Resize(edge) => window.start_drag_resize(octant(edge)),
    }
}

/// The compass octant of a window edge.
#[cfg(not(target_arch = "wasm32"))]
fn octant(edge: gantz_egui::widget::WindowEdge) -> bevy::math::CompassOctant {
    use bevy::math::CompassOctant;
    use gantz_egui::widget::WindowEdge;
    match edge {
        WindowEdge::North => CompassOctant::North,
        WindowEdge::NorthEast => CompassOctant::NorthEast,
        WindowEdge::East => CompassOctant::East,
        WindowEdge::SouthEast => CompassOctant::SouthEast,
        WindowEdge::South => CompassOctant::South,
        WindowEdge::SouthWest => CompassOctant::SouthWest,
        WindowEdge::West => CompassOctant::West,
        WindowEdge::NorthWest => CompassOctant::NorthWest,
    }
}
