//! Provides a Bevy plugin for debounced event emission based on input activity.
//!
//! The plugin monitors input events and emits a `DebouncedInputEvent` after a
//! configurable delay from the last input event. This can be used for
//! auto-saving, or any other behavior that should occur after a period of user
//! inactivity.

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_input::prelude::*;
use bevy_time::prelude::*;
use bevy_window::WindowFocused;

/// A plugin that emits debounced events based on input activity.
///
/// The plugin monitors keyboard, mouse, and touch input events. When any input
/// is detected, it starts a timer. Once the timer expires without new input, a
/// `DebouncedInputEvent` is emitted. The event is also emitted immediately if
/// the window loses focus with pending input.
///
/// # Example
///
/// ```ignore
/// use bevy::prelude::*;
///
/// fn main() {
///     App::new()
///         .add_plugins(DebouncedInputPlugin::new(2.0))
///         .add_systems(Update, handle_autosave)
///         .run();
/// }
///
/// fn handle_autosave(mut msgs: MessageReader<DebouncedInputEvent>) {
///     for msg in msgs.read() {
///         if msg.triggered_by_focus_loss {
///             println!("Auto-saving due to window focus loss");
///         } else {
///             println!("Auto-saving after inactivity");
///         }
///         // Perform your save logic here
///     }
/// }
/// ```
pub struct DebouncedInputPlugin {
    /// Duration in secs to wait after the last input before emitting the event
    debounce_secs: f32,
}

/// Event emitted when the debounce timer expires after input activity.
///
/// This event is triggered in two scenarios:
///
/// 1. After the debounce delay has passed since the last input event
/// 2. Immediately when the window loses focus (if there was pending input)
#[derive(Message)]
pub struct DebouncedInputEvent {
    /// Indicates whether this event was triggered by window focus loss
    pub triggered_by_focus_loss: bool,
}

#[derive(Resource)]
struct DebouncedInputTimer {
    timer: Timer,
    has_pending_input: bool,
}

impl DebouncedInputPlugin {
    /// Creates a new plugin instance with the specified debounce duration.
    ///
    /// The `debounce_secs` parameter determines how long to wait after the last
    /// input event before emitting a `DebouncedInputEvent`.
    pub fn new(debounce_secs: f32) -> Self {
        Self { debounce_secs }
    }
}

impl DebouncedInputEvent {
    fn from_timer() -> Self {
        Self {
            triggered_by_focus_loss: false,
        }
    }

    fn from_focus_loss() -> Self {
        Self {
            triggered_by_focus_loss: true,
        }
    }
}

impl DebouncedInputTimer {
    fn new(secs: f32) -> Self {
        Self {
            timer: Timer::from_seconds(secs, TimerMode::Once),
            has_pending_input: false,
        }
    }
}

impl Default for DebouncedInputEvent {
    fn default() -> Self {
        Self::from_timer()
    }
}

impl Plugin for DebouncedInputPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DebouncedInputEvent>()
            .insert_resource(DebouncedInputTimer::new(self.debounce_secs))
            .add_systems(
                Update,
                (
                    detect_input_changes,
                    handle_debounce_timer,
                    handle_window_focus,
                )
                    .chain(),
            );
    }
}

fn detect_input_changes(
    mut timer: ResMut<DebouncedInputTimer>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut touch: MessageReader<TouchInput>,
) {
    let has_input =
        keyboard.get_pressed().len() > 0 || mouse.get_pressed().len() > 0 || touch.read().len() > 0;

    if has_input {
        timer.timer.reset();
        timer.has_pending_input = true;
    }
}

fn handle_debounce_timer(
    time: Res<Time>,
    mut timer: ResMut<DebouncedInputTimer>,
    mut msgs: MessageWriter<DebouncedInputEvent>,
) {
    if timer.has_pending_input {
        timer.timer.tick(time.delta());

        if timer.timer.just_finished() {
            msgs.write(DebouncedInputEvent::from_timer());
            timer.has_pending_input = false;
        }
    }
}

fn handle_window_focus(
    mut window_focused: MessageReader<WindowFocused>,
    mut timer: ResMut<DebouncedInputTimer>,
    mut msgs: MessageWriter<DebouncedInputEvent>,
) {
    // If we lose window focus and have pending input, emit event immediately
    for msg in window_focused.read() {
        if !msg.focused && timer.has_pending_input {
            msgs.write(DebouncedInputEvent::from_focus_loss());
            timer.has_pending_input = false;
            timer.timer.reset();
        }
    }
}
