//! Primary window setup and native window-size persistence.
//!
//! On native platforms the window's size is restored on launch and persisted
//! via the same [`bevy_pkv`]-backed store as the rest of the app state. On web
//! the window is governed by `fit_canvas_to_parent` (the canvas fills its HTML
//! container), so persisting a fixed size is meaningless and is disabled.

use bevy::{
    prelude::*,
    window::{Window, WindowPlugin},
};
use bevy_gantz::storage::{Load, Save};
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
            // NOTE: This vastly improves input-latency on wayland. If you
            // notice tearing or simialr issues, open an issue so we can try and
            // select the right `PresentMode` for each system!
            present_mode: bevy::window::PresentMode::AutoNoVsync,
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
    if let Some(size) = bevy_gantz::storage::load::<WindowSize>(storage, KEY) {
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
            bevy_gantz::storage::save(storage, KEY, &WindowSize { width, height });
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (storage, window);
}
