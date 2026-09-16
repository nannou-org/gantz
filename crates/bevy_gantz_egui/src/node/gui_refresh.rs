//! Eager host-side refresh of `gui` marker state.
//!
//! Any push-kind evaluation marks its head dirty. A per-frame system re-pulls
//! every `gui` marker of dirty heads and of heads whose module was just
//! compiled. The trees the GUI reads from marker state lag a push by at most
//! one frame. Marker pulls are Pull-kind, so refresh never feeds back into
//! itself, and idle graphs are never re-evaluated.

use bevy_ecs::prelude::*;
use bevy_gantz::head;
use gantz_core::compile::EvalKind;

/// Marks a head whose `gui` marker state may be stale because a push-kind
/// evaluation ran on it.
#[derive(Component)]
pub struct GuiMarkersDirty;

/// Flags the head dirty on any push-kind evaluation.
///
/// Pull-kind evaluations, including the marker refreshes, never mark. So
/// refresh cannot loop.
pub fn mark_gui_dirty_on_push(trigger: On<bevy_gantz::vm::EvalEntryEvent>, mut cmds: Commands) {
    let event = trigger.event();
    if event.entrypoint.0.iter().any(|s| s.kind == EvalKind::Push) {
        cmds.entity(event.head).insert(GuiMarkersDirty);
    }
}

/// Re-pull every `gui` marker of dirty or freshly compiled heads.
///
/// `Ref<Module>` change detection covers the initial compile and every
/// recompile, since `vm::sync` replaces `head::Module` on every rebuild. The
/// [`GuiMarkersDirty`] flag covers push-kind evaluations. Pushes fired from
/// the egui pass, after `Update`, refresh on the next frame. Each refresh uses
/// the ordinary [`EvalEntryEvent`][bevy_gantz::vm::EvalEntryEvent] flow, so
/// marker pulls show up in the VM perf capture on push frames.
///
/// Marker discovery uses [`gantz_egui::node::gui::marker_paths`], a pure data
/// walk with no codec and no reified cache. Each pull entrypoint is rebuilt
/// from the marker's input count so the entry fn name cannot diverge.
pub fn refresh_gui_markers(
    registry: Res<bevy_gantz::Registry>,
    heads: Query<
        (
            Entity,
            Ref<head::Module>,
            &head::WorkingGraph,
            Has<GuiMarkersDirty>,
        ),
        With<head::OpenHead>,
    >,
    mut cmds: Commands,
) {
    for (entity, module, wg, dirty) in heads.iter() {
        if !dirty && !module.is_changed() {
            continue;
        }
        for (path, n_inputs) in gantz_egui::node::gui::marker_paths(&registry, wg) {
            // The compiled module holds a singleton pull entrypoint per
            // marker instance.
            let n = n_inputs.min(u8::MAX as usize) as u8;
            cmds.trigger(bevy_gantz::vm::EvalEntryEvent {
                head: entity,
                entrypoint: gantz_core::compile::entrypoint::pull(path, n),
                time: None,
            });
        }
        if dirty {
            cmds.entity(entity).remove::<GuiMarkersDirty>();
        }
    }
}
