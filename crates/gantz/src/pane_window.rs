//! Native OS windows for popped-out panes (#271 part 1).
//!
//! bevy_egui multi-pass requires every egui context to carry a *unique*
//! `EguiMultipassSchedule` label with a UI system registered into it, so a
//! dynamic number of pop-out windows is served by a fixed pool of pre-registered
//! slots: each slot is a distinct [`WindowSlotPass`] schedule + [`WindowSlot`]
//! camera marker + a monomorphised [`render_windowed_pane`] system. A slot binds
//! to a pane when it pops out and frees when it re-docks or its window closes.
//!
//! Each frame the widget reports its windowed set via
//! [`WindowedPanesRequested`]; [`reconcile_windowed_panes`] diffs that against
//! the live slots, spawning an OS window + camera for a newly-windowed pane and
//! despawning them when it is no longer windowed. Panes beyond the pool re-dock
//! (there is no silent drop). Closing a window re-docks its pane.
//!
//! Only native targets build this module - on web the widget keeps drawing
//! popped-out panes as in-canvas `egui::Window`s.
//!
//! Known first-cut limitations: a windowed Logs / perf pane renders empty (the
//! host trace / perf captures aren't wired into the per-window widget), and a
//! windowed config pane's global changes (compile config, DSP, new branch) take
//! effect only once re-docked ([`apply_pane_response`] applies per-pane edits and
//! payloads, not whole-widget outcomes).

use bevy::camera::RenderTarget;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::window::{WindowCloseRequested, WindowRef};
use bevy_egui::{EguiContexts, EguiMultipassSchedule, egui};
use bevy_gantz::{BuiltinNodes, CompileConfig, Registry, head};
use bevy_gantz_egui::{
    BaseImmutable, BaseNames, Demos, GuiState, HeadAccess, HostNativePaneWindows, OpenHeadViews,
    ResponseDispatchers, WindowedPanesRequested, apply_pane_response, registry_ref,
};
use gantz_ca as ca;
use gantz_core::Node;
use gantz_egui::widget::Pane;
use std::collections::HashMap;
use std::marker::PhantomData;

/// Maximum number of panes shown as native OS windows at once. Panes beyond this
/// re-dock (they cannot be shown, since each needs its own pre-registered slot).
/// Keep in sync with the slot registration in [`PaneWindowPlugin::build`] and the
/// slot dispatch in [`spawn_slot`].
pub const MAX_WINDOWED_PANES: usize = 8;

const _: () = assert!(
    MAX_WINDOWED_PANES == 8,
    "the slot registration and `spawn_slot` dispatch enumerate slots 0..8",
);

/// The unique egui multi-pass schedule for window slot `SLOT`.
#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WindowSlotPass<const SLOT: usize>;

/// Marker on the pop-out camera bound to window slot `SLOT`.
#[derive(Component, Clone, Copy)]
struct WindowSlot<const SLOT: usize>;

/// Marker on a pop-out `Window` entity.
#[derive(Component, Clone, Copy)]
struct PopoutWindow;

/// A bound window slot: the pane it shows and the window + camera entities
/// realising it.
struct SlotBinding {
    pane: Pane,
    window: Entity,
    camera: Entity,
}

/// The pool of window slots, indexed by slot number.
#[derive(Resource, Default)]
pub struct WindowedPanes {
    slots: [Option<SlotBinding>; MAX_WINDOWED_PANES],
}

impl WindowedPanes {
    /// The slot currently showing `pane`, if any.
    fn slot_of(&self, pane: &Pane) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| s.as_ref().is_some_and(|b| b.pane == *pane))
    }

    /// The first free slot, if the pool isn't full.
    fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(Option::is_none)
    }
}

/// Registers the native pop-out-window machinery: the slot pool, the reconciler,
/// the close handler, and one render system per slot.
pub struct PaneWindowPlugin<N>(PhantomData<fn() -> N>);

impl<N> Default for PaneWindowPlugin<N> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<N> Plugin for PaneWindowPlugin<N>
where
    N: 'static
        + Node
        + Clone
        + ca::CaHash
        + gantz_egui::NodeUi
        + gantz_egui::sync::AsNamedRef
        + Send
        + Sync,
{
    fn build(&self, app: &mut App) {
        app.init_resource::<WindowedPanes>()
            // Signals `bevy_gantz_egui::update` to build the widget in
            // `HostNative` mode so it stops drawing `egui::Window`s itself.
            .insert_resource(HostNativePaneWindows)
            .add_systems(Update, (reconcile_windowed_panes, on_window_close_redock));
        // One render system per slot, each in its own unique schedule.
        app.add_systems(WindowSlotPass::<0>, render_windowed_pane::<N, 0>);
        app.add_systems(WindowSlotPass::<1>, render_windowed_pane::<N, 1>);
        app.add_systems(WindowSlotPass::<2>, render_windowed_pane::<N, 2>);
        app.add_systems(WindowSlotPass::<3>, render_windowed_pane::<N, 3>);
        app.add_systems(WindowSlotPass::<4>, render_windowed_pane::<N, 4>);
        app.add_systems(WindowSlotPass::<5>, render_windowed_pane::<N, 5>);
        app.add_systems(WindowSlotPass::<6>, render_windowed_pane::<N, 6>);
        app.add_systems(WindowSlotPass::<7>, render_windowed_pane::<N, 7>);
    }
}

/// Diff the widget's reported windowed set against the live slots: bind a free
/// slot (spawning an OS window + camera) for each newly-windowed pane, and free
/// slots whose pane is no longer windowed. Panes that don't fit the pool
/// re-dock.
fn reconcile_windowed_panes(
    mut cmds: Commands,
    mut windows: ResMut<WindowedPanes>,
    requested: Res<WindowedPanesRequested>,
    mut ctxs: EguiContexts,
) {
    // Free slots whose pane is no longer windowed.
    for i in 0..MAX_WINDOWED_PANES {
        let still_wanted = windows.slots[i]
            .as_ref()
            .is_some_and(|b| requested.0.iter().any(|w| w.pane == b.pane));
        if windows.slots[i].is_some() && !still_wanted {
            let b = windows.slots[i].take().unwrap();
            cmds.entity(b.camera).try_despawn();
            cmds.entity(b.window).try_despawn();
        }
    }

    // Bind a slot for each newly-windowed pane; overflow re-docks.
    for wp in &requested.0 {
        if windows.slot_of(&wp.pane).is_some() {
            continue;
        }
        match windows.free_slot() {
            Some(slot) => spawn_slot(slot, &mut cmds, &mut windows, wp),
            None => {
                if let Ok(ctx) = ctxs.ctx_mut() {
                    gantz_egui::widget::redock_windowed_pane(ctx, &wp.pane);
                }
            }
        }
    }
}

/// Spawn the window + camera for `slot` and record the binding. Dispatches the
/// runtime slot index to the const generic that carries the slot's schedule /
/// marker.
fn spawn_slot(
    slot: usize,
    cmds: &mut Commands,
    windows: &mut WindowedPanes,
    wp: &gantz_egui::widget::WindowedPane,
) {
    match slot {
        0 => spawn_slot_const::<0>(cmds, windows, wp),
        1 => spawn_slot_const::<1>(cmds, windows, wp),
        2 => spawn_slot_const::<2>(cmds, windows, wp),
        3 => spawn_slot_const::<3>(cmds, windows, wp),
        4 => spawn_slot_const::<4>(cmds, windows, wp),
        5 => spawn_slot_const::<5>(cmds, windows, wp),
        6 => spawn_slot_const::<6>(cmds, windows, wp),
        7 => spawn_slot_const::<7>(cmds, windows, wp),
        _ => {}
    }
}

fn spawn_slot_const<const SLOT: usize>(
    cmds: &mut Commands,
    windows: &mut WindowedPanes,
    wp: &gantz_egui::widget::WindowedPane,
) {
    let window = cmds
        .spawn((
            Window {
                title: wp.title.clone(),
                ..default()
            },
            PopoutWindow,
        ))
        .id();
    let camera = cmds
        .spawn((
            Camera2d,
            RenderTarget::Window(WindowRef::Entity(window)),
            EguiMultipassSchedule::new(WindowSlotPass::<SLOT>),
            WindowSlot::<SLOT>,
        ))
        .id();
    windows.slots[SLOT] = Some(SlotBinding {
        pane: wp.pane.clone(),
        window,
        camera,
    });
}

/// When a pop-out window is closed, free its slot, re-dock its pane, and despawn
/// its camera.
///
/// Runs in `Update`, so the camera despawn flushes before the `PostUpdate` egui
/// pass loop - which would otherwise panic running a context whose window is
/// gone. The window entity itself is left for Bevy's `close_when_requested` to
/// despawn (we never despawn it, so no ordering constraint is needed).
fn on_window_close_redock(
    mut closed: MessageReader<WindowCloseRequested>,
    mut windows: ResMut<WindowedPanes>,
    mut ctxs: EguiContexts,
    mut cmds: Commands,
) {
    for ev in closed.read() {
        let Some(slot) = (0..MAX_WINDOWED_PANES).find(|&i| {
            windows.slots[i]
                .as_ref()
                .is_some_and(|b| b.window == ev.window)
        }) else {
            continue;
        };
        let b = windows.slots[slot].take().unwrap();
        if let Ok(ctx) = ctxs.ctx_mut() {
            gantz_egui::widget::redock_windowed_pane(ctx, &b.pane);
        }
        cmds.entity(b.camera).try_despawn();
        // Leave the window entity for `close_when_requested` to despawn.
    }
}

/// Render slot `SLOT`'s pane into its OS window's egui context.
///
/// A slimmer sibling of [`bevy_gantz_egui::update`] that draws one pane instead
/// of the whole tile tree, reusing the same head access + registry construction
/// and applying the pane's edits via [`apply_pane_response`].
#[allow(clippy::too_many_arguments)]
fn render_windowed_pane<N, const SLOT: usize>(
    mut ctxs: EguiContexts,
    windows: Res<WindowedPanes>,
    mut registry: ResMut<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    demos: Res<Demos>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    tab_order: Res<head::HeadTabOrder>,
    focused: Res<head::FocusedHead>,
    mut heads_query: Query<OpenHeadViews<N>, With<head::OpenHead>>,
    base_names: Res<BaseNames>,
    base_immutable: Res<BaseImmutable>,
    compile_config: Res<CompileConfig>,
    dispatchers: Res<ResponseDispatchers>,
    mut cmds: Commands,
) where
    N: 'static
        + Node
        + Clone
        + ca::CaHash
        + gantz_egui::NodeUi
        + gantz_egui::sync::AsNamedRef
        + Send
        + Sync,
{
    // This slot's pane and the window's egui context.
    let (camera, mut pane) = match windows.slots.get(SLOT).and_then(|s| s.as_ref()) {
        Some(b) => (b.camera, b.pane.clone()),
        None => return,
    };
    let ctx = match ctxs.ctx_for_entity_mut(camera) {
        Ok(ctx) => ctx.clone(),
        Err(_) => return, // window's context not realised yet
    };

    let focused_ix = (**focused)
        .and_then(|e| tab_order.iter().position(|&x| x == e))
        .unwrap_or(0);

    // Map heads to entities for response payload dispatch (after render).
    let head_to_entity: HashMap<ca::Head, Entity> = tab_order
        .iter()
        .filter_map(|&e| {
            let data = heads_query.get(e).ok()?;
            Some(((**data.core.head_ref).clone(), e))
        })
        .collect();

    // Render the pane into a `CentralPanel` filling the window. Scoped so the
    // registry / query borrows release before `apply_pane_response`.
    let mut response = {
        let node_reg = registry_ref(&registry, &builtins, &demos);
        let mut access = HeadAccess::new(&tab_order, &mut heads_query, &mut vms);
        let widget = gantz_egui::widget::Gantz::new(&node_reg, &base_names.0)
            .base_immutable(base_immutable.0)
            .demos(&demos.0)
            .compile_config(compile_config.0);

        // A background `Ui` spanning the window, as `bevy_gantz_egui::update`
        // builds for the primary context.
        let panel_id = egui::Id::new((ctx.viewport_id(), "gantz-windowed-pane-panel"));
        let mut panel_ui = egui::Ui::new(
            ctx.clone(),
            panel_id,
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::background())
                .max_rect(ctx.content_rect()),
        );
        panel_ui.set_clip_rect(ctx.content_rect());

        egui::CentralPanel::default()
            .frame(egui::Frame::default())
            .show_inside(&mut panel_ui, |ui| {
                widget.render_windowed_pane(
                    &mut gui_state.0,
                    focused_ix,
                    &mut access,
                    &mut pane,
                    ui,
                )
            })
            .inner
    };

    apply_pane_response::<N>(
        &mut response,
        &head_to_entity,
        &mut heads_query,
        &mut registry,
        &dispatchers,
        &mut cmds,
    );
}
