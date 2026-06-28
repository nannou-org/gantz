//! Provides a Bevy plugin for debounced event emission based on input activity.
//!
//! The plugin monitors input events and emits an event after a configurable
//! delay from the last input event. This can be used for auto-saving, or any
//! other behavior that should occur after a period of user inactivity.
//!
//! The plugin is generic over the event it emits ([`DebouncedEvent`]), so
//! several instances with different delays - and distinct event types - can run
//! side by side to debounce independent behaviours.

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_input::prelude::*;
use bevy_time::prelude::*;
use bevy_window::WindowFocused;
use std::marker::PhantomData;

/// A plugin that emits a debounced event of type `E` based on input activity.
///
/// The plugin monitors keyboard, mouse, and touch input. When any input is
/// detected it (re)starts a timer; once the timer expires without new input, an
/// `E` is emitted. `E` is also emitted immediately if the window loses focus
/// with pending input.
///
/// Add one instance per debounce cadence - each tracks its own timer:
///
/// ```ignore
/// app.add_plugins(DebouncedInputPlugin::<DebouncedInputEvent>::new(0.25))
///    .add_plugins(DebouncedInputPlugin::<SlowSave>::new(0.6));
/// ```
pub struct DebouncedInputPlugin<E = DebouncedInputEvent> {
    /// Seconds to wait after the last input before emitting the event.
    debounce_secs: f32,
    _marker: PhantomData<E>,
}

/// An event emitted by [`DebouncedInputPlugin`].
///
/// Implement this for a custom type to run an additional debounce at a
/// different delay.
pub trait DebouncedEvent: Message {
    /// Construct the event. `triggered_by_focus_loss` is `true` when emitted
    /// early because the window lost focus rather than the timer expiring.
    fn debounced(triggered_by_focus_loss: bool) -> Self;
}

/// The default [`DebouncedEvent`].
///
/// Emitted after the debounce delay, or immediately on window focus loss while
/// input is pending.
#[derive(Message, Default)]
pub struct DebouncedInputEvent {
    /// Whether this event was triggered by window focus loss.
    pub triggered_by_focus_loss: bool,
}

impl DebouncedEvent for DebouncedInputEvent {
    fn debounced(triggered_by_focus_loss: bool) -> Self {
        Self {
            triggered_by_focus_loss,
        }
    }
}

#[derive(Resource)]
struct DebouncedInputTimer<E: Send + Sync + 'static> {
    timer: Timer,
    has_pending_input: bool,
    _marker: PhantomData<E>,
}

impl<E> DebouncedInputPlugin<E> {
    /// Create the plugin, waiting `debounce_secs` after the last input before
    /// emitting `E`.
    pub fn new(debounce_secs: f32) -> Self {
        Self {
            debounce_secs,
            _marker: PhantomData,
        }
    }
}

impl<E: Send + Sync + 'static> DebouncedInputTimer<E> {
    fn new(secs: f32) -> Self {
        Self {
            timer: Timer::from_seconds(secs, TimerMode::Once),
            has_pending_input: false,
            _marker: PhantomData,
        }
    }
}

impl<E: DebouncedEvent> Plugin for DebouncedInputPlugin<E> {
    fn build(&self, app: &mut App) {
        app.add_message::<E>()
            .insert_resource(DebouncedInputTimer::<E>::new(self.debounce_secs))
            .add_systems(
                Update,
                (
                    detect_input_changes::<E>,
                    handle_debounce_timer::<E>,
                    handle_window_focus::<E>,
                )
                    .chain(),
            );
    }
}

fn detect_input_changes<E: Send + Sync + 'static>(
    mut timer: ResMut<DebouncedInputTimer<E>>,
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

fn handle_debounce_timer<E: DebouncedEvent>(
    time: Res<Time>,
    mut timer: ResMut<DebouncedInputTimer<E>>,
    mut msgs: MessageWriter<E>,
) {
    if timer.has_pending_input {
        timer.timer.tick(time.delta());

        if timer.timer.just_finished() {
            msgs.write(E::debounced(false));
            timer.has_pending_input = false;
        }
    }
}

fn handle_window_focus<E: DebouncedEvent>(
    mut window_focused: MessageReader<WindowFocused>,
    mut timer: ResMut<DebouncedInputTimer<E>>,
    mut msgs: MessageWriter<E>,
) {
    // If we lose window focus and have pending input, emit the event immediately.
    for msg in window_focused.read() {
        if !msg.focused && timer.has_pending_input {
            msgs.write(E::debounced(true));
            timer.has_pending_input = false;
            timer.timer.reset();
        }
    }
}
