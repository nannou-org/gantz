//! Committed-size synchronisation for resizable node bodies such as comment
//! and plot.
//!
//! A node's committed size is part of its content address. The displayed size
//! lives in egui's persisted `Resize` state. Left unreconciled, the two feed
//! back. External changes from undo, collab sync or merge neither display nor
//! survive. The next frame overwrites them from the local layout and mints a
//! spurious commit. Collaborating peers whose rendered sizes round differently
//! correct each other in an endless commit ping-pong.
//!
//! [`size_sync_frame`] is the per-frame state machine both nodes share.
//! External changes push into the display and never commit. The push is a
//! one-frame `Resize::fixed_size` container. egui clamps persisted state
//! through min and max and stores it back, which overrides stale state. Only
//! genuine local interaction writes the committed size. That is a settled
//! resize-corner release, or a node-specific edit such as a comment text
//! flush.

/// Per-frame size bookkeeping stored in egui temp memory, keyed by the
/// node's egui id `.with("size_sync")`.
///
/// `last_seen` is the node's committed size at the end of the last UI pass.
/// A mismatch at frame start means the value was replaced externally by
/// undo, collab sync or merge. That must update the display rather than be
/// clobbered by it. The key is the node index, so index reuse after a
/// deletion can leave a stale entry. The mismatch then reads as an external
/// change, which self-heals with a push and no commit.
#[derive(Clone, Copy)]
struct SizeSync {
    last_seen: [u16; 2],
    was_resizing: bool,
}

/// The per-frame size-sync decisions. See [`begin`].
#[derive(Clone, Copy)]
pub(crate) struct Decisions {
    /// The resize corner is actively dragged this frame.
    pub resizing: bool,
    /// The committed size must be pushed into the displayed resize state.
    /// This is the first frame under this id, or the node value changed
    /// externally. An external change also wins over an in-flight drag.
    pub push_external: bool,
    /// The resize corner was released since the last frame. This is the
    /// interaction allowed to write the committed size.
    pub drag_released: bool,
}

/// Begin a frame. Read whether the resize corner is dragged, load the stored
/// sync state and derive this frame's [`Decisions`] against the committed
/// `size`. Repaints are requested while the corner is dragged so the
/// post-release snap frame draws.
///
/// `resize_id` is the id of the node's `egui::containers::Resize`. The
/// corner interaction registers under its `"__resize_corner"` salt, an egui
/// 0.34 internal in `containers/resize.rs`.
pub(crate) fn begin(
    ui: &egui::Ui,
    sync_id: egui::Id,
    resize_id: egui::Id,
    size: [u16; 2],
) -> Decisions {
    let corner_id = resize_id.with("__resize_corner");
    let resizing = ui
        .ctx()
        .read_response(corner_id)
        .is_some_and(|r| r.dragged());
    if resizing {
        ui.ctx().request_repaint();
    }
    let prev: Option<SizeSync> = ui.memory_mut(|m| m.data.get_temp(sync_id));
    let (push_external, drag_released) = size_sync_frame(prev, size, resizing);
    Decisions {
        resizing,
        push_external,
        drag_released,
    }
}

/// Persist the sync state at the end of the frame, after any local write,
/// so a local commit never masquerades as an external change next frame.
pub(crate) fn store(
    ui: &egui::Ui,
    sync_id: egui::Id,
    size: [u16; 2],
    push_external: bool,
    resizing: bool,
) {
    ui.memory_mut(|m| {
        m.data.insert_temp(
            sync_id,
            SizeSync {
                last_seen: size,
                was_resizing: if push_external { false } else { resizing },
            },
        )
    });
}

/// The pure size-sync decisions for one frame as `(push_external,
/// drag_released)`. See [`Decisions`].
fn size_sync_frame(prev: Option<SizeSync>, size: [u16; 2], resizing: bool) -> (bool, bool) {
    match prev {
        None => (true, false),
        Some(prev) => {
            let push = prev.last_seen != size;
            let released = !push && prev.was_resizing && !resizing;
            (push, released)
        }
    }
}

/// The committed form of a rendered size. It is rounded rather than
/// truncated, so two machines whose rendered sizes straddle an integer do not
/// disagree by a whole unit.
pub(crate) fn fitted_size(width: f32, height: f32) -> [u16; 2] {
    [width.round() as u16, height.round() as u16]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// External changes always push into the display and never commit. A
    /// commit-worthy release fires only on a clean corner-drag transition
    /// from `true` to `false`.
    #[test]
    fn size_sync_frame_decisions() {
        let sync = |last_seen, was_resizing| {
            Some(SizeSync {
                last_seen,
                was_resizing,
            })
        };
        // The first frame under this id pushes and never commits.
        assert_eq!(size_sync_frame(None, [100, 40], false), (true, false));
        // Steady state has nothing to do.
        assert_eq!(
            size_sync_frame(sync([100, 40], false), [100, 40], false),
            (false, false)
        );
        // An external change pushes even mid-drag. External wins, no release.
        assert_eq!(
            size_sync_frame(sync([100, 40], true), [120, 40], true),
            (true, false)
        );
        // A drag in progress, starting or continuing, gives no push and no
        // release.
        assert_eq!(
            size_sync_frame(sync([100, 40], false), [100, 40], true),
            (false, false)
        );
        assert_eq!(
            size_sync_frame(sync([100, 40], true), [100, 40], true),
            (false, false)
        );
        // The release transition is commit-worthy.
        assert_eq!(
            size_sync_frame(sync([100, 40], true), [100, 40], false),
            (false, true)
        );
    }

    #[test]
    fn fitted_size_rounds() {
        assert_eq!(fitted_size(100.6, 40.4), [101, 40]);
        assert_eq!(fitted_size(100.4, 40.5), [100, 41]);
    }
}
