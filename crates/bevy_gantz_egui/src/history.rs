//! Undo and redo.

use crate::{ForHead, GraphView, GuiState, ResyncRefsEvent, SessionHead};
use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head;
use bevy_log as log;

/// Handle undo payloads. Move the head back to its parent commit.
///
/// A session head, see [`SessionHead`], instead mints a forward revert
/// commit so the undo propagates to peers like any other edit.
pub fn on_undo(
    trigger: On<ForHead<gantz_egui::Undo>>,
    mut registry: ResMut<Registry>,
    mut gui_state: ResMut<GuiState>,
    heads: Query<(&head::HeadRef, Has<SessionHead>), With<head::OpenHead>>,
    graph_views: Query<&GraphView>,
    mut cmds: Commands,
) {
    let entity = trigger.event().head;
    let Ok((head_ref, in_session)) = heads.get(entity) else {
        log::error!("Undo: head not found for entity {entity:?}");
        return;
    };
    let head = (**head_ref).clone();
    let gui = &mut gui_state.0;
    let target = if in_session {
        let live_camera = graph_views.get(entity).ok().map(|gv| gv.0.camera);
        gantz_egui::ops::session_undo(
            &mut registry.0,
            &mut gui.redo_stacks,
            &mut gui.undo_cursors,
            bevy_gantz::reg::timestamp(),
            &head,
            live_camera,
        )
    } else {
        gantz_egui::ops::undo(&registry.0, &mut gui.redo_stacks, &head)
    };
    if let Some(target) = target {
        cmds.trigger(head::MoveHeadEvent { entity, target });
        cmds.trigger(ResyncRefsEvent::default());
    }
}

/// Handle redo payloads. Move the head forward to an undone commit.
///
/// A session head, see [`SessionHead`], instead mints a forward revert
/// commit restoring the undone position, mirroring [`on_undo`].
pub fn on_redo(
    trigger: On<ForHead<gantz_egui::Redo>>,
    mut registry: ResMut<Registry>,
    mut gui_state: ResMut<GuiState>,
    heads: Query<(&head::HeadRef, Has<SessionHead>), With<head::OpenHead>>,
    graph_views: Query<&GraphView>,
    mut cmds: Commands,
) {
    let entity = trigger.event().head;
    let Ok((head_ref, in_session)) = heads.get(entity) else {
        log::error!("Redo: head not found for entity {entity:?}");
        return;
    };
    let head = (**head_ref).clone();
    let gui = &mut gui_state.0;
    let target = if in_session {
        let live_camera = graph_views.get(entity).ok().map(|gv| gv.0.camera);
        gantz_egui::ops::session_redo(
            &mut registry.0,
            &mut gui.redo_stacks,
            &mut gui.undo_cursors,
            bevy_gantz::reg::timestamp(),
            &head,
            live_camera,
        )
    } else {
        gantz_egui::ops::redo(&mut gui.redo_stacks, &head)
    };
    if let Some(target) = target {
        cmds.trigger(head::MoveHeadEvent { entity, target });
        cmds.trigger(ResyncRefsEvent::default());
    }
}
