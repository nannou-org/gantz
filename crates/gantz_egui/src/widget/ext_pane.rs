//! Application-supplied top-level panes. See [`ExtPane`].

use crate::Responses;
use gantz_core::node;

/// An application-supplied top-level pane. It is the pane analogue of
/// [`SettingsTab`][super::SettingsTab].
///
/// Domains contribute their own panes by supplying implementations to the
/// [`Gantz`][super::Gantz] widget via
/// [`Gantz::ext_panes`][super::Gantz::ext_panes]. A pane typically holds a
/// per-frame snapshot of its domain's data. It reports changes by pushing
/// typed payloads into the returned [`Responses`] for the host to apply.
///
/// A supplied pane's tile is inserted into the tray on first sight of its
/// [`key`][Self::key] and persists in the tile tree from then on. While no
/// provider supplies the key, the tile renders a placeholder. Visibility
/// defaults to hidden and is toggled in the Settings Panes subtab like the
/// built-in tray panes.
pub trait ExtPane {
    /// The pane's stable identity. It is persisted in the tile tree and keys
    /// the pop-out window geometry. It must be unique among the supplied panes
    /// and stable across sessions.
    fn key(&self) -> &str;

    /// The tab label. The tab title suffixes it with the focused head, like
    /// the built-in Steel pane.
    fn title(&self) -> &str;

    /// Hover text for the pane's visibility checkbox in the Settings Panes
    /// subtab.
    fn description(&self) -> &str {
        ""
    }

    /// Render the pane's contents, returning any change payloads.
    fn ui(&mut self, cx: ExtPaneCtx, ui: &mut egui::Ui) -> Responses;
}

/// The context handed to [`ExtPane::ui`]. It carries what the widget knows
/// about the focused head, roughly the built-in Steel pane's inputs.
///
/// Only the widget constructs one. It is `#[non_exhaustive]` so new context
/// can reach panes without breaking implementors.
#[non_exhaustive]
pub struct ExtPaneCtx<'a> {
    /// The currently focused head, if any.
    pub focused: Option<&'a gantz_ca::Head>,
    /// The focused head's selected root-level node indices, sorted.
    pub selection: &'a [node::Id],
}

/// One supplied extension pane's identity and labels for the pane-visibility
/// checkboxes. See [`panes_config`][super::panes_config()].
#[derive(Clone, Debug)]
pub struct ExtPaneEntry {
    /// The pane's stable identity. See [`ExtPane::key`].
    pub key: String,
    /// The checkbox label. See [`ExtPane::title`].
    pub title: String,
    /// The checkbox hover text. See [`ExtPane::description`].
    pub description: String,
}
