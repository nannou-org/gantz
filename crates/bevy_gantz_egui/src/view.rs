//! Persisting each commit's view and committing settled layouts.

use crate::{GraphView, GuiState};
use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head;
use gantz_ca as ca;

/// The system set containing the per-frame view persistence passes
/// [`persist_camera_and_seed`] and [`settle_layout`] in the `Update`
/// schedule.
///
/// Layers that publish commit views beyond the process, for example the
/// collaborative-session announce, should run `.after(ViewPersistSet)` so a
/// commit rendered this frame has its view seeded before it can be served.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub struct ViewPersistSet;

/// Keep each open head's camera current and seed its commit's layout baseline.
///
/// Runs every frame. Each commit's stored `layout` stays frozen as an undo
/// baseline. It is written exactly once, when a commit first has a populated
/// live layout, and is never overwritten in place. Node-position changes only
/// ever produce a new commit, see [`settle_layout`]. The camera is excluded
/// from undo, so it tracks the live view in place every frame.
pub fn persist_camera_and_seed(
    mut registry: ResMut<Registry>,
    heads: Query<(&head::HeadRef, &GraphView), With<head::OpenHead>>,
) {
    for (head_ref, head_view) in heads.iter() {
        let Some(commit_addr) = registry.head_commit_ca(&**head_ref) else {
            continue;
        };
        match gantz_edit::section::view(&registry, &commit_addr) {
            // Layout frozen as the undo baseline. Only the camera tracks live.
            Some(mut view) => {
                if view.camera != head_view.camera {
                    view.camera = head_view.camera;
                    gantz_edit::section::set_view(&mut registry.0, commit_addr, &view);
                }
            }
            // Seed the baseline once the scene has laid the graph out. Guarding
            // on a non-empty layout avoids capturing an empty pre-layout frame.
            None if !head_view.layout.is_empty() => {
                gantz_edit::section::set_view(&mut registry.0, commit_addr, head_view);
            }
            None => {}
        }
    }
}

/// On layout settle, signalled by [`DebouncedInputEvent`], fork a layout-only
/// commit for any open head whose node positions changed since its frozen
/// baseline view.
///
/// Mirrors the graph-commit path's GUI bookkeeping. It migrates per-head GUI
/// state, clears redo and migrates the graph pane. It does not fire
/// [`head::CommittedEvent`]. The graph content is unchanged, so there is
/// nothing to resync, and firing it would churn every sync-enabled referrer's
/// history for a pure layout move. `GraphView` is left untouched. It already
/// holds the settled layout, which matches the new commit's seeded baseline.
///
/// [`DebouncedInputEvent`]: bevy_gantz::debounced_input::DebouncedInputEvent
pub fn settle_layout(
    mut registry: ResMut<Registry>,
    mut gui_state: ResMut<GuiState>,
    mut heads: Query<(Entity, &mut head::HeadRef, &GraphView), With<head::OpenHead>>,
    mut cmds: Commands,
) {
    for (entity, mut head_ref, head_view) in heads.iter_mut() {
        let old_head = (**head_ref).clone();
        let Some(new_commit) = gantz_edit::view::commit_layout(
            &mut registry,
            bevy_gantz::reg::timestamp(),
            &mut head_ref,
            head_view,
        ) else {
            continue;
        };
        // Freeze the new commit's layout baseline this frame, before any
        // debounce-gated save/export reads the registry's view section.
        gantz_edit::section::set_view(&mut registry.0, new_commit, head_view);
        // Clear redo, since a new commit invalidates it, and migrate GUI state.
        let new_head = (**head_ref).clone();
        gui_state.migrate_head(&old_head, &new_head, true);
        // Not a `CommittedEvent`, so no resync cascade. Layers that mirror
        // commits elsewhere, such as collab sessions syncing node positions,
        // still need to hear about it.
        cmds.trigger(LayoutCommittedEvent { entity, new_commit });
    }
}

/// Emitted after a settled node-move produced a layout-only commit, with the
/// same graph and a new commit. Unlike [`head::CommittedEvent`] this triggers
/// no resync or recompile machinery. It exists for layers that follow the
/// commit chain, such as collaborative sessions syncing node positions.
#[derive(Debug, Event)]
pub struct LayoutCommittedEvent {
    pub entity: Entity,
    pub new_commit: ca::CommitAddr,
}
