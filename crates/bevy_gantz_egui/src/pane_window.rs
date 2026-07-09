//! Native OS windows for popped-out panes (#271 part 1).
//!
//! Each popped-out pane becomes a real OS window (for monitoring on a second
//! display / projection-mapping). A pop-out window is a Bevy [`Window`] + a
//! camera carrying an [`EguiContext`] but **no** `EguiMultipassSchedule`, i.e. a
//! *single-pass* secondary context: bevy_egui auto-begins/ends its egui pass each
//! frame, so `render_windowed_panes` - one ordinary `Update` system - draws
//! into an unbounded number of them. There is no fixed window pool.
//!
//! Each frame the widget reports its windowed set via [`WindowedPanesRequested`]
//! (mirrored by [`update`][crate::update]); `reconcile_windowed_panes` diffs it
//! against the live pop-out entities, spawning / despawning windows to match.
//! Closing a window re-docks its pane. A windowed pane is rendered with the exact
//! same widget inputs and response handling (`handle_gantz_response`) as a
//! docked one, so behaviour is identical either way.
//!
//! Native only - on web the widget draws popped-out panes as in-canvas
//! `egui::Window`s instead.

use crate::{
    BaseImmutable, BaseNames, BuiltinNodes, CompileConfig, Demos, GuiState, HeadAccess,
    HostNativePaneWindows, ImportTask, OpenHeadViews, PerfGui, PerfVm, Registry,
    ResponseDispatchers, SettingsTabs, TraceCapture, WindowedPanesRequested, handle_gantz_response,
    head, registry_ref,
};
use bevy_app::prelude::*;
use bevy_camera::{Camera2d, RenderTarget};
use bevy_ecs::prelude::*;
use bevy_egui::{EguiContext, EguiContexts, EguiMultipassSchedule, EguiPreUpdateSet, egui};
use bevy_window::{PresentMode, PrimaryWindow, Window, WindowCloseRequested, WindowRef};
use gantz_ca as ca;
use gantz_core::Node;
use gantz_egui::widget::Pane;
use std::collections::HashMap;
use std::marker::PhantomData;

/// Marker on a pop-out `Window` entity.
#[derive(Component)]
struct PopoutWindow;

/// On a pop-out camera: the pane it renders and its window entity.
#[derive(Component)]
struct PopoutView {
    pane: Pane,
    window: Entity,
}

/// The `N: Node` bounds shared by the render system and the plugin (same as
/// [`crate::update`]).
trait PaneNode:
    'static
    + Node
    + Clone
    + ca::CaHash
    + gantz_egui::NodeUi
    + gantz_egui::sync::AsNamedRef
    + Send
    + Sync
{
}
impl<N> PaneNode for N where
    N: 'static
        + Node
        + Clone
        + ca::CaHash
        + gantz_egui::NodeUi
        + gantz_egui::sync::AsNamedRef
        + Send
        + Sync
{
}

/// Registers native pop-out windows: the reconciler, the shared render system,
/// and the close handler. Opt-in (the app adds it); presence of the inserted
/// [`HostNativePaneWindows`] makes [`crate::update`] stop drawing `egui::Window`s
/// and leave the windows to this plugin.
pub struct PaneWindowPlugin<N>(PhantomData<fn() -> N>);

impl<N> Default for PaneWindowPlugin<N> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<N: PaneNode> Plugin for PaneWindowPlugin<N> {
    fn build(&self, app: &mut App) {
        app.init_resource::<WindowedPanesRequested>()
            .insert_resource(HostNativePaneWindows)
            // Spawn / despawn pop-out windows in `PreUpdate` before bevy_egui
            // initialises contexts, so a newly-spawned single-pass context gets
            // its full first-frame lifecycle (screen rect -> input/scale ->
            // `begin_pass`, which builds its fonts) before it is drawn in
            // `Update` and tessellated in `PostUpdate`. Spawning it in `Update`
            // would miss `begin_pass` yet still be tessellated the same frame -
            // "No fonts loaded".
            .add_systems(
                PreUpdate,
                reconcile_windowed_panes.before(EguiPreUpdateSet::InitContexts),
            )
            .add_systems(
                Update,
                (
                    render_windowed_panes::<N>.after(bevy_gantz::VmSet),
                    on_window_close_redock,
                    track_popout_geometry,
                ),
            );
    }
}

/// Diff the widget's reported windowed set against the live pop-out windows:
/// spawn a window + camera for each newly-windowed pane, and despawn those whose
/// pane is no longer windowed. The ECS is the map - each pop-out camera carries
/// its [`PopoutView`].
fn reconcile_windowed_panes(
    mut cmds: Commands,
    requested: Res<WindowedPanesRequested>,
    popouts: Query<(Entity, &PopoutView)>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    gui_state: Res<GuiState>,
) {
    // Despawn windows whose pane is no longer requested.
    for (camera, view) in &popouts {
        if !requested.0.iter().any(|w| w.pane == view.pane) {
            cmds.entity(camera).try_despawn();
            cmds.entity(view.window).try_despawn();
        }
    }
    // Inherit the primary window's present mode so a pop-out doesn't stall the
    // shared render thread with a different (blocking) surface-acquire cadence -
    // e.g. the app's `AutoNoVsync` primary paired with a default `Fifo` pop-out
    // judders on Wayland. Match the frame-latency queue depth for the same reason.
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
        // Restore this pane's last window size (tracked in `GantzState` and
        // persisted with the rest of the GUI state), so it reopens as it was.
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
/// Reuses the same head access / registry construction, widget inputs, and
/// response handling as [`crate::update`], so a windowed pane behaves exactly
/// like a docked one.
#[allow(clippy::too_many_arguments)]
fn render_windowed_panes<N: PaneNode>(
    mut popouts: Query<(&mut EguiContext, &PopoutView), Without<EguiMultipassSchedule>>,
    trace_capture: Res<TraceCapture>,
    mut perf_vm: ResMut<PerfVm>,
    mut perf_gui: ResMut<PerfGui>,
    mut registry: ResMut<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    tab_order: Res<head::HeadTabOrder>,
    mut focused: ResMut<head::FocusedHead>,
    mut heads_query: Query<OpenHeadViews<N>, With<head::OpenHead>>,
    import_task: Option<Res<ImportTask>>,
    (
        base_names,
        base_immutable,
        mut compile_config,
        mut change_validation,
        mut settings_tabs,
        mut demos,
        dispatchers,
    ): (
        Res<BaseNames>,
        Res<BaseImmutable>,
        ResMut<CompileConfig>,
        ResMut<bevy_gantz::ValidateCommitted>,
        ResMut<SettingsTabs>,
        ResMut<Demos>,
        Res<ResponseDispatchers>,
    ),
    mut cmds: Commands,
) {
    // Collect each window's context (cheap Arc clone) + pane up front, so the
    // `popouts` borrow is released before borrowing the head/registry resources.
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

    // Map heads to entities for response dispatch (heads are stable across the
    // loop - open/close events applied via `cmds` only flush after this system).
    let head_to_entity: HashMap<ca::Head, Entity> = tab_order
        .iter()
        .filter_map(|&e| {
            let data = heads_query.get(e).ok()?;
            Some(((**data.core.head_ref).clone(), e))
        })
        .collect();

    let level = bevy_log::tracing_subscriber::filter::LevelFilter::current();

    for (ctx, mut pane) in targets {
        // Render the pane into a `CentralPanel` filling the window, using the
        // regular pane frame. Scoped so the registry / query borrows release
        // before `handle_gantz_response`. The settings-tab view list is
        // rebuilt per window: each iteration's borrow of the boxes ends with
        // it (mirrors `update`, where it is built inside the panel closure).
        let mut response = {
            let node_reg = registry_ref(&registry, &builtins, &demos);
            let mut access = HeadAccess::new(&tab_order, &mut heads_query, &mut vms);
            let mut tabs: Vec<&mut dyn gantz_egui::widget::SettingsTab> = settings_tabs
                .0
                .iter_mut()
                .map(|t| &mut **t as &mut dyn gantz_egui::widget::SettingsTab)
                .collect();
            let widget = gantz_egui::widget::Gantz::new(&node_reg, &base_names.0)
                .base_immutable(base_immutable.0)
                .demos(&demos.0)
                .compile_config(compile_config.0)
                .validate_change_tracking(change_validation.0)
                .trace_capture(trace_capture.0.clone(), level)
                .perf_captures(&mut perf_vm.0, &mut perf_gui.0)
                .settings_tabs(&mut tabs);

            // A background `Ui` spanning the window (as `update` builds for the
            // primary context).
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

        handle_gantz_response::<N>(
            &mut response,
            &tab_order,
            &mut focused,
            &mut heads_query,
            &mut registry,
            &mut demos,
            &base_names,
            &mut compile_config,
            &mut change_validation,
            import_task.as_deref(),
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

/// Record each pop-out window's current size into `GantzState` (keyed by
/// [`pane_key`][gantz_egui::widget::pane_key]) when it changes, so it is
/// persisted with the rest of the GUI state and restored next session by
/// [`reconcile_windowed_panes`]. `Changed<Window>` keeps this to actual resizes.
fn track_popout_geometry(
    mut gui_state: ResMut<GuiState>,
    changed: Query<(&Window, &PopoutView), Changed<Window>>,
) {
    for (window, view) in &changed {
        let width = window.resolution.width();
        let height = window.resolution.height();
        // Skip before the surface is realized, when the size is still ~zero.
        if width >= 1.0 && height >= 1.0 {
            gui_state.0.windowed_geometry.insert(
                gantz_egui::widget::pane_key(&view.pane),
                gantz_egui::widget::PaneWindowGeometry { width, height },
            );
        }
    }
}
