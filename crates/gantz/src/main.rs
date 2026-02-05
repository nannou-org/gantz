use bevy::{
    prelude::*,
    window::{Window, WindowPlugin},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use bevy_gantz::{
    BuiltinNodes, CompiledModule, FocusedHead, GantzPlugin, GuiState, HeadGuiState, HeadRef,
    HeadTabOrder, HeadVms, OpenHead, OpenHeadData, OpenHeadDataReadOnly, Registry, RegistryRef,
    Views, WorkingGraph,
    debounced_input::{DebouncedInputEvent, DebouncedInputPlugin},
    head, reg, timestamp, vm,
};
use bevy_pkv::PkvStore;
use builtin::Builtins;
use gantz_ca as ca;

mod builtin;
mod node;
mod storage;

/// A resource for capturing tracing logs for the `TraceView` widget.
#[derive(Default, Resource)]
struct TraceCapture(gantz_egui::widget::trace_view::TraceCapture);

/// Performance capture for VM execution timing.
#[derive(Default, Resource)]
struct PerfVm(gantz_egui::widget::PerfCapture);

/// Performance capture for GUI frame timing.
#[derive(Default, Resource)]
struct PerfGui(gantz_egui::widget::PerfCapture);

fn main() {
    App::new()
        .insert_resource(TraceCapture::default())
        .insert_resource(PerfVm::default())
        .insert_resource(PerfGui::default())
        // Gantz plugin (provides FocusedHead, HeadTabOrder, HeadVms, Registry, Views)
        .add_plugins(GantzPlugin::<Box<dyn node::Node>>::default())
        // App-specific builtins
        .insert_resource(BuiltinNodes::<Box<dyn node::Node>>(Box::new(
            Builtins::new(),
        )))
        .add_plugins(DefaultPlugins.set(log_plugin()).set(window_plugin()))
        .add_plugins(EguiPlugin::default())
        .add_plugins(DebouncedInputPlugin::new(0.25))
        .insert_resource(PkvStore::new("nannou-org", "gantz"))
        .add_systems(
            Startup,
            (
                setup_camera,
                setup_resources,
                setup_open.after(setup_resources),
                reg::prune_unused::<Box<dyn node::Node>>
                    .after(setup_resources)
                    .after(setup_open),
                vm::setup::<Box<dyn node::Node>>.after(reg::prune_unused::<Box<dyn node::Node>>),
            ),
        )
        .add_systems(EguiPrimaryContextPass, update_gui)
        .add_systems(
            Update,
            persist_resources.run_if(on_message::<DebouncedInputEvent>),
        )
        .run();
}

fn log_plugin() -> bevy::log::LogPlugin {
    bevy::log::LogPlugin {
        custom_layer: move |app| {
            let capture = app.world().resource_ref::<TraceCapture>();
            Some(Box::new(capture.0.clone().layer()))
        },
        ..Default::default()
    }
}

fn window_plugin() -> WindowPlugin {
    WindowPlugin {
        primary_window: Some(Window {
            title: "gantz".into(),
            name: Some("gantz".into()),
            fit_canvas_to_parent: true,
            // NOTE: This vastly improves input-latency on wayland. If you
            // notice tearing or simialr issues, open an issue so we can try and
            // select the right `PresentMode` for each system!
            present_mode: bevy::window::PresentMode::AutoNoVsync,
            ..default()
        }),
        ..default()
    }
}

fn setup_camera(mut cmds: Commands) {
    cmds.spawn(Camera2d);
}

fn setup_resources(storage: Res<PkvStore>, mut cmds: Commands) {
    let registry: Registry<Box<dyn node::Node>> = storage::load_registry(&*storage);
    let views = storage::load_views(&*storage);
    let gui_state = storage::load_gui_state(&*storage);
    cmds.insert_resource(registry);
    cmds.insert_resource(views);
    cmds.insert_resource(gui_state);
}

fn setup_open(
    storage: Res<PkvStore>,
    mut registry: ResMut<Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
    mut cmds: Commands,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
) {
    let loaded = storage::load_open(&*storage, &mut *registry, &*views, timestamp());
    let focused_head = storage::load_focused_head(&*storage);

    // Spawn entities for each open head.
    for (head, graph, head_views) in loaded {
        let is_focused = focused_head.as_ref() == Some(&head);
        let entity = cmds
            .spawn((
                OpenHead,
                HeadRef(head),
                WorkingGraph(graph),
                head_views,
                CompiledModule::default(),
                HeadGuiState::default(),
            ))
            .id();

        tab_order.push(entity);

        // Set focused to the persisted focused head, or first head as fallback.
        if is_focused || (**focused).is_none() {
            **focused = Some(entity);
        }
    }
}

fn update_gui(
    trace_capture: Res<TraceCapture>,
    mut perf_vm: ResMut<PerfVm>,
    mut perf_gui: ResMut<PerfGui>,
    mut ctxs: EguiContexts,
    mut registry: ResMut<Registry<Box<dyn node::Node>>>,
    builtins: Res<BuiltinNodes<Box<dyn node::Node>>>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<HeadVms>,
    tab_order: Res<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
    mut heads_query: Query<OpenHeadData<Box<dyn node::Node>>, With<OpenHead>>,
    mut storage: ResMut<PkvStore>,
    mut memory_loaded: Local<bool>,
    mut cmds: Commands,
) -> Result {
    let ctx = ctxs.ctx_mut()?;

    // Load egui memory once on first frame
    if !*memory_loaded {
        storage::load_egui_memory(&mut *storage, ctx);
        *memory_loaded = true;
    }

    // Measure GUI frame time.
    let gui_start = web_time::Instant::now();

    // Determine the focused head index from the focused entity.
    let focused_ix = (**focused)
        .and_then(|e| tab_order.iter().position(|&x| x == e))
        .unwrap_or(0);

    // Create the head access adapter.
    let mut access = head::HeadAccess::new(&tab_order, &mut heads_query, &mut vms);

    // Construct node registry on-demand for the widget.
    let node_reg = RegistryRef::new(&*registry, &*builtins);

    let level = bevy::log::tracing_subscriber::filter::LevelFilter::current();
    let response = egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctx, |ui| {
            gantz_egui::widget::Gantz::new(&node_reg)
                .trace_capture(trace_capture.0.clone(), level)
                .perf_captures(&mut perf_vm.0, &mut perf_gui.0)
                .show(&mut *gui_state, focused_ix, &mut access, ui)
        })
        .inner;

    // Update focused head from the widget's response.
    if let Some(&entity) = tab_order.get(response.focused_head) {
        **focused = Some(entity);
    }

    // The given graph name was removed.
    if let Some(name) = response.graph_name_removed() {
        // Update any open heads that reference this name.
        for mut data in heads_query.iter_mut() {
            if let ca::Head::Branch(head_name) = &**data.head_ref {
                if *head_name == name {
                    let commit_ca = *registry.head_commit_ca(&*data.head_ref).unwrap();
                    **data.head_ref = ca::Head::Commit(commit_ca);
                }
            }
        }
        registry.remove_name(&name);
    }

    // Trigger events for head operations (handled by observers).

    // Single click: replace the focused head with the selected one.
    if let Some(new_head) = response.graph_replaced() {
        cmds.trigger(head::ReplaceEvent(new_head.clone()));
    }

    // Open head as a new tab (or focus if already open).
    if let Some(new_head) = response.graph_opened() {
        cmds.trigger(head::OpenEvent(new_head.clone()));
    }

    // Close head.
    if let Some(h) = response.graph_closed() {
        cmds.trigger(head::CloseEvent(h.clone()));
    }

    // Create a new empty graph and open it.
    if response.new_graph() {
        let new_head = registry.init_head(timestamp());
        cmds.trigger(head::OpenEvent(new_head));
    }

    // Handle closed heads from tab close buttons.
    for closed_head in &response.closed_heads {
        cmds.trigger(head::CloseEvent(closed_head.clone()));
    }

    // Handle new branch created from tab double-click.
    if let Some((original_head, new_name)) = response.new_branch() {
        cmds.trigger(head::CreateBranchEvent {
            original: original_head.clone(),
            new_name: new_name.clone(),
        });
    }

    // Record GUI frame time.
    perf_gui.0.record(gui_start.elapsed());

    Ok(())
}

fn persist_resources(
    registry: Res<Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
    gui_state: Res<GuiState>,
    mut storage: ResMut<PkvStore>,
    mut ctxs: EguiContexts,
    tab_order: Res<HeadTabOrder>,
    focused: Res<FocusedHead>,
    heads_query: Query<OpenHeadDataReadOnly<Box<dyn node::Node>>, With<OpenHead>>,
) {
    // Save graphs.
    let mut addrs: Vec<_> = registry.graphs().keys().copied().collect();
    addrs.sort();
    storage::save_graph_addrs(&mut *storage, &addrs);
    storage::save_graphs(&mut *storage, &registry.graphs());

    // Save commits.
    let mut addrs: Vec<_> = registry.commits().keys().copied().collect();
    addrs.sort();
    storage::save_commit_addrs(&mut *storage, &addrs);
    storage::save_commits(&mut *storage, registry.commits());

    // Save names.
    storage::save_names(&mut *storage, registry.names());

    // Save all open heads in tab order.
    let heads: Vec<_> = tab_order
        .iter()
        .filter_map(|&entity| {
            heads_query
                .get(entity)
                .ok()
                .map(|data| (**data.head_ref).clone())
        })
        .collect();
    storage::save_open_heads(&mut *storage, &heads);

    // Save the focused head.
    if let Some(focused_entity) = **focused {
        if let Ok(data) = heads_query.get(focused_entity) {
            storage::save_focused_head(&mut *storage, &**data.head_ref);
        }
    }

    // Save all views (already updated in update_vm).
    storage::save_views(&mut *storage, &*views);

    // Save the gantz GUI state.
    storage::save_gui_state(&mut *storage, &gui_state);

    // Save egui memory (widget states).
    if let Ok(ctx) = ctxs.ctx_mut() {
        storage::save_egui_memory(&mut *storage, ctx);
    }
}
