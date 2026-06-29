//! The keymap: a single source of truth for the editor's command keyboard
//! shortcuts.
//!
//! [`Action`] enumerates every user-bindable command. [`Keymap`] maps actions
//! to their [`egui::KeyboardShortcut`]s and is the one place dispatch sites read
//! their bindings from (via [`Keymap::consume`]) and that the
//! `Settings -> Keybinds` panel edits.
//!
//! The map is *sparse*: it stores only user overrides. An action absent from the
//! map uses [`Action::default_bindings`]. This gives forward-compatibility for
//! free (a newly-added action just works with its defaults) and makes "reset"
//! simply forgetting the override.
//!
//! To add a command shortcut: add an [`Action`] variant, give it a `label`,
//! `description` and `default_bindings`, list it in [`Action::ALL`], then call
//! `keymap.consume(ui, Action::Foo)` at the one site that has the context to act.
//! It then appears in the panel, persists, and participates in conflict
//! detection automatically. Dispatch more-specific bindings first (see
//! [`Keymap::consume`]).

use egui::{Key, KeyboardShortcut, Modifiers};
use std::collections::{BTreeMap, HashMap};

/// Every user-bindable editor command. One variant per command shortcut.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Deserialize, serde::Serialize,
)]
pub enum Action {
    /// Copy the selected nodes to the clipboard.
    Copy,
    /// Paste nodes from the clipboard into the focused graph.
    Paste,
    /// Open the dialog to create a new graph.
    NewGraph,
    /// Undo the last change to the focused graph.
    Undo,
    /// Redo the last undone change to the focused graph.
    Redo,
    /// Show or hide the command palette.
    ToggleCommandPalette,
    /// Select every node in the focused graph.
    SelectAll,
    /// Copy the selected nodes to the clipboard, then remove them.
    Cut,
    /// Duplicate the selected nodes in place.
    Duplicate,
}

/// The keymap: action -> bindings, holding only user overrides (see module
/// docs). Defaults come from [`Action::default_bindings`].
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Keymap {
    /// User overrides. An action absent here uses its default bindings; an
    /// action present with an empty `Vec` is explicitly unbound.
    #[serde(default)]
    overrides: BTreeMap<Action, Vec<KeyboardShortcut>>,
}

/// `Cmd` on macOS, `Ctrl` elsewhere (cross-platform; see [`Modifiers::command`]).
const CMD: Modifiers = Modifiers {
    alt: false,
    ctrl: false,
    shift: false,
    mac_cmd: false,
    command: true,
};

/// `Cmd`/`Ctrl` plus `Shift`.
const CMD_SHIFT: Modifiers = Modifiers {
    alt: false,
    ctrl: false,
    shift: true,
    mac_cmd: false,
    command: true,
};

// Default bindings, as named consts so the slices are `'static` (a `&[..]` built
// inline in a `match` arm is not const-promoted when returned).
const COPY: &[KeyboardShortcut] = &[KeyboardShortcut::new(CMD, Key::C)];
const PASTE: &[KeyboardShortcut] = &[KeyboardShortcut::new(CMD, Key::V)];
const NEW_GRAPH: &[KeyboardShortcut] = &[KeyboardShortcut::new(CMD, Key::N)];
const UNDO: &[KeyboardShortcut] = &[KeyboardShortcut::new(CMD, Key::Z)];
const REDO: &[KeyboardShortcut] = &[
    KeyboardShortcut::new(CMD_SHIFT, Key::Z),
    KeyboardShortcut::new(CMD, Key::Y),
];
const TOGGLE_COMMAND_PALETTE: &[KeyboardShortcut] =
    &[KeyboardShortcut::new(Modifiers::NONE, Key::Space)];
const SELECT_ALL: &[KeyboardShortcut] = &[KeyboardShortcut::new(CMD, Key::A)];
const CUT: &[KeyboardShortcut] = &[KeyboardShortcut::new(CMD, Key::X)];
const DUPLICATE: &[KeyboardShortcut] = &[KeyboardShortcut::new(CMD, Key::D)];

impl Action {
    /// Every action, in the order the keybinds panel lists them.
    pub const ALL: &'static [Action] = &[
        Action::Copy,
        Action::Paste,
        Action::NewGraph,
        Action::Undo,
        Action::Redo,
        Action::ToggleCommandPalette,
        Action::SelectAll,
        Action::Cut,
        Action::Duplicate,
    ];

    /// A short human-readable name for the panel.
    pub fn label(self) -> &'static str {
        match self {
            Action::Copy => "Copy",
            Action::Paste => "Paste",
            Action::NewGraph => "New graph",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::ToggleCommandPalette => "Node palette",
            Action::SelectAll => "Select all",
            Action::Cut => "Cut",
            Action::Duplicate => "Duplicate",
        }
    }

    /// A one-line description, shown as hover text in the panel.
    pub fn description(self) -> &'static str {
        match self {
            Action::Copy => "Copy the selected nodes to the clipboard.",
            Action::Paste => "Paste nodes from the clipboard into the focused graph.",
            Action::NewGraph => "Open the dialog to create a new graph.",
            Action::Undo => "Undo the last change to the focused graph.",
            Action::Redo => "Redo the last undone change to the focused graph.",
            Action::ToggleCommandPalette => "Show or hide the command palette for creating nodes.",
            Action::SelectAll => "Select every node in the focused graph.",
            Action::Cut => "Copy the selected nodes to the clipboard, then remove them.",
            Action::Duplicate => "Duplicate the selected nodes in place.",
        }
    }

    /// The default binding(s) for this action.
    ///
    /// Returns a `'static` slice (const-promoted) so the common no-override path
    /// allocates nothing.
    pub fn default_bindings(self) -> &'static [KeyboardShortcut] {
        match self {
            Action::Copy => COPY,
            Action::Paste => PASTE,
            Action::NewGraph => NEW_GRAPH,
            Action::Undo => UNDO,
            // Two defaults; `Cmd+Shift+Z` is the more specific, so dispatch Redo
            // before Undo (see [`Keymap::consume`]).
            Action::Redo => REDO,
            Action::ToggleCommandPalette => TOGGLE_COMMAND_PALETTE,
            Action::SelectAll => SELECT_ALL,
            Action::Cut => CUT,
            Action::Duplicate => DUPLICATE,
        }
    }
}

impl Keymap {
    /// The effective bindings for `action`: the user override if set, otherwise
    /// the default. Borrows, so the no-override path allocates nothing.
    pub fn bindings(&self, action: Action) -> &[KeyboardShortcut] {
        match self.overrides.get(&action) {
            Some(bindings) => bindings,
            None => action.default_bindings(),
        }
    }

    /// Whether `action`'s bindings have been customised away from the default.
    pub fn is_overridden(&self, action: Action) -> bool {
        self.overrides.contains_key(&action)
    }

    /// Consume any input event matching one of `action`'s bindings, returning
    /// whether one fired this frame.
    ///
    /// `egui`'s `consume_shortcut` matches modifiers *logically* (extra
    /// Shift/Alt are ignored), so a less-specific binding like `Cmd+Z` also
    /// matches a `Cmd+Shift+Z` event. Dispatch more-specific bindings first
    /// (e.g. [`Action::Redo`] before [`Action::Undo`]); a consumed event will
    /// not fire again.
    pub fn consume(&self, ui: &egui::Ui, action: Action) -> bool {
        let bindings = self.bindings(action);
        ui.input_mut(|i| {
            // Consume every matching binding (don't short-circuit) so none leak
            // to another handler.
            bindings
                .iter()
                .fold(false, |fired, s| i.consume_shortcut(s) | fired)
        })
    }

    /// Set `action`'s bindings. Setting them back to the default forgets the
    /// override, keeping the map sparse.
    pub fn set(&mut self, action: Action, bindings: Vec<KeyboardShortcut>) {
        if bindings.as_slice() == action.default_bindings() {
            self.overrides.remove(&action);
        } else {
            self.overrides.insert(action, bindings);
        }
    }

    /// Add a single binding to `action` (no-op if it already has it).
    pub fn add(&mut self, action: Action, shortcut: KeyboardShortcut) {
        let mut bindings = self.bindings(action).to_vec();
        if !bindings.contains(&shortcut) {
            bindings.push(shortcut);
            self.set(action, bindings);
        }
    }

    /// Remove a single binding from `action`.
    pub fn remove(&mut self, action: Action, shortcut: KeyboardShortcut) {
        let mut bindings = self.bindings(action).to_vec();
        bindings.retain(|&s| s != shortcut);
        self.set(action, bindings);
    }

    /// Reset `action` to its default binding(s).
    pub fn reset(&mut self, action: Action) {
        self.overrides.remove(&action);
    }

    /// Reset every action to its default binding(s).
    pub fn reset_all(&mut self) {
        self.overrides.clear();
    }

    /// Shortcuts bound to more than one action, mapped to the conflicting
    /// actions (in [`Action::ALL`] order). Empty when there are no conflicts.
    pub fn conflicts(&self) -> HashMap<KeyboardShortcut, Vec<Action>> {
        let mut by_shortcut: HashMap<KeyboardShortcut, Vec<Action>> = HashMap::new();
        for &action in Action::ALL {
            for &shortcut in self.bindings(action) {
                let actions = by_shortcut.entry(shortcut).or_default();
                if !actions.contains(&action) {
                    actions.push(action);
                }
            }
        }
        by_shortcut.retain(|_, actions| actions.len() > 1);
        by_shortcut
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_for_absent_actions() {
        let km = Keymap::default();
        assert_eq!(km.bindings(Action::Copy), Action::Copy.default_bindings());
        assert_eq!(km.bindings(Action::Copy).len(), 1);
        // Redo keeps both default bindings.
        assert_eq!(km.bindings(Action::Redo).len(), 2);
        assert!(!km.is_overridden(Action::Copy));
    }

    #[test]
    fn set_reset_and_sparseness() {
        let mut km = Keymap::default();
        let new = vec![KeyboardShortcut::new(CMD_SHIFT, Key::C)];
        km.set(Action::Copy, new.clone());
        assert!(km.is_overridden(Action::Copy));
        assert_eq!(km.bindings(Action::Copy), new.as_slice());

        // Setting back to the default forgets the override (stays sparse).
        km.set(Action::Copy, Action::Copy.default_bindings().to_vec());
        assert!(!km.is_overridden(Action::Copy));

        km.set(Action::Copy, new);
        km.reset(Action::Copy);
        assert!(!km.is_overridden(Action::Copy));
    }

    #[test]
    fn add_and_remove_bindings() {
        let mut km = Keymap::default();
        let extra = KeyboardShortcut::new(CMD, Key::Insert);
        km.add(Action::Copy, extra);
        assert!(km.bindings(Action::Copy).contains(&extra));
        // Adding again is a no-op.
        km.add(Action::Copy, extra);
        assert_eq!(km.bindings(Action::Copy).len(), 2);
        km.remove(Action::Copy, extra);
        assert!(!km.is_overridden(Action::Copy));
    }

    #[test]
    fn conflicts_detects_shared_binding() {
        let mut km = Keymap::default();
        assert!(km.conflicts().is_empty());
        // Bind Paste to Copy's shortcut.
        km.set(Action::Paste, vec![KeyboardShortcut::new(CMD, Key::C)]);
        let conflicts = km.conflicts();
        let shortcut = KeyboardShortcut::new(CMD, Key::C);
        let actions = conflicts.get(&shortcut).expect("expected a conflict");
        assert!(actions.contains(&Action::Copy));
        assert!(actions.contains(&Action::Paste));
    }

    #[test]
    fn serde_round_trip_is_sparse() {
        let mut km = Keymap::default();
        km.set(Action::Undo, vec![KeyboardShortcut::new(CMD, Key::U)]);
        let encoded = ron::to_string(&km).unwrap();
        let back: Keymap = ron::from_str(&encoded).unwrap();
        assert_eq!(km, back);
        assert!(back.is_overridden(Action::Undo));
        // Untouched actions are not stored.
        assert!(!back.is_overridden(Action::Copy));
    }
}
