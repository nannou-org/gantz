//! The "Keybinds" settings subtab: view and rebind the editor's command
//! keyboard shortcuts (the [`Keymap`]).
//!
//! Edits mutate the [`Keymap`] in place; it is persisted as part of
//! [`crate::widget::GantzState`].
//!
//! Note: capturing a combo that is *already* bound to another command may be
//! intercepted by that command's dispatch (panes share egui's input), so the
//! combo won't register here. That case is a conflict anyway - free the combo
//! from the other command first. Capturing any unbound combo works normally.

use crate::{Action, Keymap};

/// The outcome of polling input while capturing a new binding.
enum Capture {
    /// No key pressed yet; keep waiting.
    Waiting,
    /// The user pressed Escape; cancel the capture.
    Cancelled,
    /// A combo was captured.
    Got(egui::KeyboardShortcut),
}

/// Render the keybinds editor. Mutates `keymap` in place.
pub fn keybinds_config(keymap: &mut Keymap, ui: &mut egui::Ui) {
    let capture_id = ui.id().with("capturing_action");
    let mut capturing: Option<Action> = ui.data(|d| d.get_temp(capture_id)).unwrap_or(None);

    // Advance an in-progress capture.
    if let Some(action) = capturing {
        match poll_capture(ui) {
            Capture::Got(shortcut) => {
                keymap.add(action, shortcut);
                capturing = None;
            }
            Capture::Cancelled => capturing = None,
            // Keep redrawing so the next key press is polled promptly.
            Capture::Waiting => ui.ctx().request_repaint(),
        }
    }

    let conflicts = keymap.conflicts();

    ui.label("Command shortcuts:");
    ui.add_space(4.0);

    for &action in Action::ALL {
        ui.horizontal(|ui| {
            ui.label(action.label()).on_hover_text(action.description());

            // Existing bindings as chips; click one to remove it. Cloned so the
            // map can be mutated while iterating.
            let bindings = keymap.bindings(action).to_vec();
            if bindings.is_empty() && capturing != Some(action) {
                ui.weak("(unbound)");
            }
            for shortcut in bindings {
                let mut text = egui::RichText::new(ui.ctx().format_shortcut(&shortcut));
                if conflicts.contains_key(&shortcut) {
                    text = text.color(ui.visuals().error_fg_color);
                }
                if ui
                    .button(text)
                    .on_hover_text("Remove this binding")
                    .clicked()
                {
                    keymap.remove(action, shortcut);
                }
            }

            // Capture a new binding for this action.
            let capturing_this = capturing == Some(action);
            let label = if capturing_this { "press keys…" } else { "+" };
            if ui
                .selectable_label(capturing_this, label)
                .on_hover_text("Add a binding (Esc to cancel)")
                .clicked()
            {
                capturing = if capturing_this { None } else { Some(action) };
            }

            // Reset this action to its default, enabled only when customised.
            if ui
                .add_enabled(keymap.is_overridden(action), egui::Button::new("default"))
                .on_hover_text("Reset to the default binding")
                .clicked()
            {
                keymap.reset(action);
                if capturing == Some(action) {
                    capturing = None;
                }
            }
        });
    }

    ui.add_space(8.0);
    ui.separator();
    if !conflicts.is_empty() {
        ui.colored_label(
            ui.visuals().error_fg_color,
            "Some shortcuts are bound to more than one command (shown in red).",
        );
    }
    if ui
        .button("Reset all keybinds")
        .on_hover_text("Reset every command shortcut to its default")
        .clicked()
    {
        keymap.reset_all();
        capturing = None;
    }

    ui.data_mut(|d| d.insert_temp(capture_id, capturing));
}

/// Poll input for the next key combo while capturing, consuming the matched
/// event so it does not also trigger something else.
fn poll_capture(ui: &egui::Ui) -> Capture {
    ui.input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            return Capture::Cancelled;
        }
        let found = i.events.iter().find_map(|e| match e {
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if *key != egui::Key::Escape => Some(egui::KeyboardShortcut::new(*modifiers, *key)),
            _ => None,
        });
        match found {
            Some(shortcut) => {
                i.consume_shortcut(&shortcut);
                Capture::Got(shortcut)
            }
            None => Capture::Waiting,
        }
    })
}
