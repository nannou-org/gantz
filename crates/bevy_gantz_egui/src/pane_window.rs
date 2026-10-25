//! Native OS windows for popped-out panes.
//!
//! Each popped-out pane becomes a real OS window, for example for monitoring
//! on a second display. A pop-out window is a Bevy [`Window`] plus a camera
//! carrying an [`EguiContext`] and no `EguiSchedule`. It is a
//! single-pass secondary context. bevy_egui begins and ends its egui pass each
//! frame, so `render_windowed_panes`, one ordinary `Update` system, draws into
//! any number of them. There is no fixed window pool.
//!
//! Each frame the widget reports its windowed set via [`WindowedPanesRequested`],
//! mirrored by [`update`][crate::update]. `reconcile_windowed_panes` diffs it
//! against the live pop-out entities and spawns or despawns windows to match.
//! Closing a window re-docks its pane. A windowed pane is rendered with the
//! same widget inputs and response handling as a docked one, through
//! `handle_gantz_response`, so behaviour is identical either way.
//!
//! Native only. On web the widget draws popped-out panes as in-canvas
//! `egui::Window`s instead.

use crate::gui::{GantzInputs, show_gantz};
use crate::{
    GuiState, HeadAccess, HostNativePaneWindows, OpenHeadViews, ResponseDispatchers,
    WindowedPanesRequested, handle_gantz_response, head,
};
use bevy_app::prelude::*;
use bevy_camera::{Camera2d, RenderTarget};
use bevy_ecs::prelude::*;
use bevy_egui::{EguiContext, EguiContexts, EguiPreUpdateSet, EguiSchedule, egui};
use bevy_window::{PresentMode, PrimaryWindow, Window, WindowCloseRequested, WindowRef};
use gantz_ca as ca;
use gantz_egui::widget::Pane;
use std::collections::HashMap;

/// Marker on a pop-out `Window` entity.
#[derive(Component)]
struct PopoutWindow;

/// On a pop-out camera. The pane it renders and its window entity.
#[derive(Component)]
struct PopoutView {
    pane: Pane,
    window: Entity,
}

/// Adds the native pop-out window systems. These are the reconciler, the
/// shared render system, and the close handler. The app opts in by adding the
/// plugin. The inserted [`HostNativePaneWindows`] makes [`crate::update`] stop
/// drawing `egui::Window`s and leave the windows to this plugin.
#[derive(Default)]
pub struct PaneWindowPlugin;

impl Plugin for PaneWindowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WindowedPanesRequested>()
            .insert_resource(HostNativePaneWindows)
            // Spawn and despawn pop-out windows in `PreUpdate` before bevy_egui
            // initialises contexts. A newly spawned single-pass context then
            // gets its full first-frame lifecycle before it is drawn in
            // `Update` and tessellated in `PostUpdate`. That lifecycle is
            // screen rect, input and scale, then `begin_pass`, which builds
            // its fonts. Spawning in `Update` would miss `begin_pass` yet
            // still tessellate the same frame, and panic with "No fonts
            // loaded".
            .add_systems(
                PreUpdate,
                reconcile_windowed_panes.before(EguiPreUpdateSet::InitContexts),
            )
            .add_systems(
                Update,
                (
                    render_windowed_panes.after(bevy_gantz::VmSet),
                    on_window_close_redock,
                    track_popout_geometry,
                ),
            );
    }
}

/// Diff the widget's reported windowed set against the live pop-out windows.
/// Spawn a window and camera for each newly windowed pane. Despawn those
/// whose pane is not windowed any more. The ECS is the map. Each pop-out
/// camera carries its [`PopoutView`].
fn reconcile_windowed_panes(
    mut cmds: Commands,
    requested: Res<WindowedPanesRequested>,
    popouts: Query<(Entity, &PopoutView)>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    gui_state: Res<GuiState>,
) {
    // Despawn windows whose pane is not requested.
    for (camera, view) in &popouts {
        if !requested.0.iter().any(|w| w.pane == view.pane) {
            cmds.entity(camera).try_despawn();
            cmds.entity(view.window).try_despawn();
        }
    }
    // Inherit the primary window's present mode. A pop-out with a different
    // blocking surface-acquire cadence stalls the shared render thread. For
    // example, an `AutoNoVsync` primary paired with a default `Fifo` pop-out
    // judders on Wayland. Match the frame-latency queue depth for the same
    // reason.
    let (present_mode, desired_maximum_frame_latency) = primary_window
        .single()
        .map(|w| (w.present_mode, w.desired_maximum_frame_latency))
        .unwrap_or((PresentMode::AutoNoVsync, None));
    // Spawn a window for each newly-requested pane.
    for wp in &requested.0 {
        if popouts.iter().any(|(_, v)| v.pane == wp.pane) {
            continue;
        }
        let mut window = Window {
            title: wp.title.clone(),
            present_mode,
            desired_maximum_frame_latency,
            ..Default::default()
        };
        // Restore this pane's last window size so it reopens as it was.
        // `GantzState` tracks it and persists it with the rest of the GUI state.
        if let Some(geom) = gui_state
            .0
            .windowed_geometry
            .get(&gantz_egui::widget::pane_key(&wp.pane))
        {
            window.resolution.set(geom.width, geom.height);
        }
        let window = cmds.spawn((window, PopoutWindow)).id();
        cmds.spawn((
            Camera2d,
            RenderTarget::Window(WindowRef::Entity(window)),
            EguiContext::default(),
            PopoutView {
                pane: wp.pane.clone(),
                window,
            },
        ));
    }
}

/// Render every pop-out pane into its window's single-pass egui context.
///
/// Uses the same widget inputs and response handling as [`crate::update`],
/// so a windowed pane behaves like a docked one.
fn render_windowed_panes(
    mut popouts: Query<(&mut EguiContext, &PopoutView), Without<EguiSchedule>>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    tab_order: Res<head::HeadTabOrder>,
    mut focused: ResMut<head::FocusedHead>,
    mut heads_query: Query<OpenHeadViews, With<head::OpenHead>>,
    mut inputs: GantzInputs,
    dispatchers: Res<ResponseDispatchers>,
    mut cmds: Commands,
) {
    // Collect each window's context and pane up front. The context is a cheap
    // Arc clone. This releases the `popouts` borrow before the head and
    // registry resources are borrowed.
    let targets: Vec<(egui::Context, Pane)> = popouts
        .iter_mut()
        .map(|(mut ctx, view)| (ctx.get_mut().clone(), view.pane.clone()))
        .collect();
    if targets.is_empty() {
        return;
    }

    let focused_ix = (**focused)
        .and_then(|e| tab_order.iter().position(|&x| x == e))
        .unwrap_or(0);

    // Map heads to entities for response dispatch. Heads are stable across
    // the loop, since open and close events applied via `cmds` flush after
    // this system.
    let head_to_entity: HashMap<ca::Head, Entity> = tab_order
        .iter()
        .filter_map(|&e| {
            let data = heads_query.get(e).ok()?;
            Some(((**data.core.head_ref).clone(), e))
        })
        .collect();

    for (ctx, mut pane) in targets {
        // A background `Ui` spanning the window, as `update` builds for the
        // primary context.
        let panel_id = egui::Id::new((ctx.viewport_id(), "gantz-windowed-pane-panel"));
        let mut panel_ui = egui::Ui::new(
            ctx.clone(),
            panel_id,
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::background())
                .max_rect(ctx.content_rect()),
        );
        panel_ui.set_clip_rect(ctx.content_rect());

        // Render the pane into a `CentralPanel` filling the window, using the
        // regular pane frame. Scoped so the query borrows release before
        // `handle_gantz_response`.
        let mut response = {
            let mut access = HeadAccess::new(&tab_order, &mut heads_query, &mut vms);
            egui::CentralPanel::default()
                .show(&mut panel_ui, |ui| {
                    let state = &mut gui_state.0;
                    let pane = Some(&mut pane);
                    show_gantz(
                        &mut inputs,
                        **focused,
                        state,
                        focused_ix,
                        &mut access,
                        pane,
                        ui,
                    )
                })
                .inner
        };

        handle_gantz_response(
            &mut response,
            &tab_order,
            &mut focused,
            &mut heads_query,
            &mut inputs.registry,
            &mut inputs.cache,
            &inputs.codec,
            &head_to_entity,
            &dispatchers,
            &mut cmds,
        );
    }
}

/// When a pop-out window is closed, re-dock its pane and despawn its camera.
///
/// Runs in `Update`, so the camera despawn flushes before the `PostUpdate` egui
/// pass loop. The window entity is left for Bevy's `close_when_requested`.
fn on_window_close_redock(
    mut closed: MessageReader<WindowCloseRequested>,
    popouts: Query<(Entity, &PopoutView)>,
    mut ctxs: EguiContexts,
    mut cmds: Commands,
) {
    for ev in closed.read() {
        for (camera, view) in &popouts {
            if view.window == ev.window {
                if let Ok(ctx) = ctxs.ctx_mut() {
                    gantz_egui::widget::redock_windowed_pane(ctx, &view.pane);
                }
                cmds.entity(camera).try_despawn();
            }
        }
    }
}

/// Record each pop-out window's current size into `GantzState`, keyed by
/// [`pane_key`][gantz_egui::widget::pane_key], when it changes. It is
/// persisted with the rest of the GUI state and restored next session by
/// [`reconcile_windowed_panes`]. `Changed<Window>` keeps this to actual
/// resizes.
fn track_popout_geometry(
    mut gui_state: ResMut<GuiState>,
    changed: Query<(&Window, &PopoutView), Changed<Window>>,
) {
    for (window, view) in &changed {
        let width = window.resolution.width();
        let height = window.resolution.height();
        // Skip before the surface is realized, when the size is still near zero.
        if width >= 1.0 && height >= 1.0 {
            gui_state.0.windowed_geometry.insert(
                gantz_egui::widget::pane_key(&view.pane),
                gantz_egui::widget::PaneWindowGeometry { width, height },
            );
        }
    }
}
