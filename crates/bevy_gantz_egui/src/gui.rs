//! The GUI pass and the handling of its response.

use crate::{
    AudioHeads, BaseImmutable, BaseNames, BuiltinNodes, CollabUi, EdgeStyles, ExtPanes, GraphCache,
    GuiState, HeadAccess, HostNativePaneWindows, NodeCodecRes, OpenHeadViews, PerfGui, PerfVm,
    RefExtUis, ResponseDispatchers, SettingsTabs, TraceCapture, WindowedPanesRequested, base, env,
    refresh_cache,
};
use bevy_ecs::prelude::*;
use bevy_egui::EguiContexts;
use bevy_egui::egui;
use bevy_gantz::CompileConfig;
use bevy_gantz::Registry;
use bevy_gantz::head;
use bevy_log as log;
use gantz_ca as ca;
use std::collections::HashMap;

/// Update the Gantz GUI and process widget responses.
///
/// This system:
/// - Shows the Gantz widget in an egui CentralPanel
/// - Processes GUI responses such as head open, close and replace
/// - Dispatches dynamic response payloads via [`ResponseDispatchers`]
/// - Uses TraceCapture for tracing, and the focused head's PerfVm and PerfGui
///   for performance capture
pub fn update(
    trace_capture: Res<TraceCapture>,
    mut perf_vms: Query<&mut PerfVm>,
    mut perf_gui: ResMut<PerfGui>,
    mut ctxs: EguiContexts,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    tab_order: Res<head::HeadTabOrder>,
    mut focused: ResMut<head::FocusedHead>,
    mut heads_query: Query<OpenHeadViews, With<head::OpenHead>>,
    (
        codec,
        base_names,
        base_immutable,
        compile_config,
        change_validation,
        mut settings_tabs,
        mut ext_panes,
        ref_ext_uis,
        (edge_styles, audio_heads),
        mut requested,
        host_native,
        export_paths,
        base_sources,
        base_name_sources,
        collab_ui,
        clipboard,
    ): (
        Res<NodeCodecRes>,
        Res<BaseNames>,
        Res<BaseImmutable>,
        Res<CompileConfig>,
        Res<bevy_gantz::ValidateCommitted>,
        ResMut<SettingsTabs>,
        ResMut<ExtPanes>,
        Res<RefExtUis>,
        (Res<EdgeStyles>, Res<AudioHeads>),
        ResMut<WindowedPanesRequested>,
        Option<Res<HostNativePaneWindows>>,
        Option<Res<base::ExportPaths>>,
        Res<base::BaseSources>,
        Res<base::BaseNameSources>,
        Option<Res<CollabUi>>,
        Option<ResMut<bevy_egui::EguiClipboard>>,
    ),
    dispatchers: Res<ResponseDispatchers>,
    mut cmds: Commands,
) -> Result {
    let ctx = ctxs.ctx_mut()?;

    let gui_start = web_time::Instant::now();

    let focused_ix = (**focused)
        .and_then(|e| tab_order.iter().position(|&x| x == e))
        .unwrap_or(0);
    let mut perf_vm = (**focused).and_then(|e| perf_vms.get_mut(e).ok());

    // Map heads to entities for response payload dispatch after `show`.
    let head_to_entity: HashMap<ca::Head, Entity> = tab_order
        .iter()
        .filter_map(|&e| {
            let data = heads_query.get(e).ok()?;
            Some(((**data.core.head_ref).clone(), e))
        })
        .collect();

    let mut access = HeadAccess::new(&tab_order, &mut heads_query, &mut vms);

    let node_reg = env(&registry, &cache, &builtins, &codec);

    let level = bevy_log::tracing_subscriber::filter::LevelFilter::current();

    let current_compile_config = compile_config.0;
    let current_validate_change_tracking = change_validation.0;
    let panel_id = egui::Id::new((ctx.viewport_id(), "central_panel"));
    let mut panel_ui = egui::Ui::new(
        ctx.clone(),
        panel_id,
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.content_rect()),
    );
    panel_ui.set_clip_rect(ctx.content_rect());

    // A native host owns the pop-out windows when `HostNativePaneWindows` is
    // present. Otherwise the widget draws them itself as `egui::Window`s.
    let pane_window_mode = match host_native {
        Some(_) => gantz_egui::widget::PaneWindowMode::HostNative,
        None => gantz_egui::widget::PaneWindowMode::EguiWindow,
    };

    // The base source names, for the graph config pane's source dropdown.
    let source_names: Vec<&str> = base_sources.0.iter().map(|s| s.name).collect();

    // Clipboard reader for widget paste affordances. The `RefCell` is needed
    // because the widget takes a shared `Fn` while `EguiClipboard::get_text`
    // needs `&mut`.
    let clipboard = clipboard.map(std::cell::RefCell::new);
    let read_clipboard = || {
        clipboard
            .as_ref()
            .and_then(|c| c.borrow_mut().get_text())
            .filter(|t| !t.is_empty())
    };

    let mut response = egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show_inside(&mut panel_ui, |ui| {
            // Built inside the closure. `&mut dyn` slices are invariant, so
            // building the view list outside would escape its lifetime.
            let mut tabs: Vec<&mut dyn gantz_egui::widget::SettingsTab> = settings_tabs
                .0
                .iter_mut()
                .map(|t| &mut **t as &mut dyn gantz_egui::widget::SettingsTab)
                .collect();
            let mut panes: Vec<&mut dyn gantz_egui::widget::ExtPane> = ext_panes
                .0
                .iter_mut()
                .map(|p| &mut **p as &mut dyn gantz_egui::widget::ExtPane)
                .collect();
            let exts: Vec<&dyn gantz_egui::node::RefExtUi> = ref_ext_uis
                .0
                .iter()
                .map(|e| &**e as &dyn gantz_egui::node::RefExtUi)
                .collect();
            let stylers: Vec<&dyn gantz_egui::widget::EdgeStyle> = edge_styles
                .0
                .iter()
                .map(|s| &**s as &dyn gantz_egui::widget::EdgeStyle)
                .collect();
            let mut widget = gantz_egui::widget::Gantz::new(&node_reg, &base_names.0)
                .base_immutable(base_immutable.0)
                .compile_config(current_compile_config)
                .validate_change_tracking(current_validate_change_tracking)
                .trace_capture(trace_capture.0.clone(), level)
                .perf_captures(perf_vm.as_deref_mut().map(|p| &mut p.0), &mut perf_gui.0)
                .pane_window_mode(pane_window_mode)
                .settings_tabs(&mut tabs)
                .ext_panes(&mut panes)
                .ref_ext_uis(&exts)
                .edge_styles(&stylers)
                .audio_heads(&audio_heads.0);
            // Base-source authoring context. Present only where the per-source
            // write-back runs, since `update-base` inserts ExportPaths. The
            // main app never shows a non-durable source dropdown.
            if let Some(paths) = &export_paths {
                widget = widget.base_sources(gantz_egui::widget::BaseSourcesCtx {
                    sources: &source_names,
                    name_sources: &base_name_sources.0,
                    default_source: paths.default_source,
                });
            }
            if let Some(collab_ui) = collab_ui.as_ref() {
                widget = widget.collab(&collab_ui.0);
            }
            widget = widget.clipboard(&read_clipboard);
            widget.show(&mut *gui_state, focused_ix, &mut access, ui)
        })
        .inner;

    // Mirror the windowed set so a native host can create and destroy OS
    // windows to match. See `pane_window::reconcile_windowed_panes`.
    requested.0 = std::mem::take(&mut response.windowed_panes);

    // Apply every outcome of the GUI pass. Shared with each native pop-out
    // window's render pass, see `pane_window::render_windowed_panes`, so a
    // pane behaves identically whether docked or windowed. `access` is not
    // used past `show`, which frees `heads_query` for the handler.
    handle_gantz_response(
        &mut response,
        &tab_order,
        &mut focused,
        &mut heads_query,
        &mut registry,
        &mut cache,
        &codec,
        &head_to_entity,
        &dispatchers,
        &mut cmds,
    );

    perf_gui.0.record(gui_start.elapsed());

    Ok(())
}

/// Apply every outcome of a `Gantz` GUI pass to the app.
///
/// Shared by the primary [`update`] pass and each native pop-out window's render
/// pass, [`pane_window::render_windowed_panes`], so a pane behaves identically
/// whether docked or windowed. Applies the focus, commits the heads the GUI
/// edited in place, and dispatches the payloads.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_gantz_response(
    response: &mut gantz_egui::widget::gantz::GantzResponse,
    tab_order: &head::HeadTabOrder,
    focused: &mut head::FocusedHead,
    heads_query: &mut Query<OpenHeadViews, With<head::OpenHead>>,
    registry: &mut Registry,
    cache: &mut GraphCache,
    codec: &NodeCodecRes,
    head_to_entity: &HashMap<ca::Head, Entity>,
    dispatchers: &ResponseDispatchers,
    cmds: &mut Commands,
) {
    if let Some(&entity) = tab_order.get(response.focused_head) {
        **focused = Some(entity);
    }

    // Commit each head whose graph the GUI edited in place this pass, so
    // `vm::sync` recompiles it from the committed address. Graph ops applied
    // via response observers commit themselves.
    for head in &response.changed_heads {
        let Some(&entity) = head_to_entity.get(head) else {
            continue;
        };
        let Ok(mut data) = heads_query.get_mut(entity) else {
            continue;
        };
        bevy_gantz::commit_working_graph(
            registry,
            cmds,
            entity,
            &mut data.core.head_ref.0,
            &data.core.working_graph.0,
        );
    }

    // Dispatch the dynamic response payloads emitted during the GUI pass.
    // `RegisterResponseExt` registers the types in `ResponseDispatchers`.
    // Unregistered payloads are reported.
    for (head, payload) in response.responses.drain() {
        log::debug!("{payload:?}");
        let entity = head.and_then(|h| head_to_entity.get(&h).copied());
        match dispatchers.0.get(&payload.type_id()) {
            Some(dispatch) => dispatch(entity, payload, cmds),
            None => log::warn!("unhandled response payload: {}", payload.type_name()),
        }
    }

    // The pass may have mutated the registry, so bring the reified cache
    // back in step before any typed reads.
    refresh_cache(registry, cache, &codec.0);
}
