use bevy::{
    prelude::*,
    window::{Window, WindowPlugin},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use bevy_gantz::{
    BuiltinNodes, CompiledModule, FocusedHead, GantzPlugin, GraphViews, GuiState, HeadGuiState,
    HeadRef, HeadTabOrder, HeadVms, OpenHead, OpenHeadData, OpenHeadDataReadOnly, Registry, Views,
    WorkingGraph,
    debounced_input::{DebouncedInputEvent, DebouncedInputPlugin},
    eval, head, timestamp,
};
use bevy_pkv::PkvStore;
use builtin::Builtins;
use env::Environment;
use gantz_ca as ca;
use steel::steel_vm::engine::Engine;

mod builtin;
mod env;
mod node;
mod storage;

type Graph = gantz_core::node::graph::Graph<Box<dyn node::Node>>;
type GraphNode = gantz_core::node::GraphNode<Box<dyn node::Node>>;

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
        // Hook observers for app-specific handling (VM init)
        .add_observer(on_head_opened)
        .add_observer(on_head_replaced)
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
                prune_unused_graphs_and_commits
                    .after(setup_resources)
                    .after(setup_open),
                setup_vm.after(prune_unused_graphs_and_commits),
            ),
        )
        .add_systems(EguiPrimaryContextPass, update_gui)
        .add_systems(
            Update,
            (
                update_vm,
                process_gantz_gui_cmds.after(update_vm),
                persist_resources.run_if(on_message::<DebouncedInputEvent>),
            ),
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

fn prune_unused_graphs_and_commits(
    mut registry: ResMut<Registry<Box<dyn node::Node>>>,
    mut views: ResMut<Views>,
    builtins: Res<BuiltinNodes<Box<dyn node::Node>>>,
    heads: Query<&HeadRef, With<OpenHead>>,
) {
    let env = Environment::new(&*registry, &*views, &*builtins);
    let head_iter = heads.iter().map(|h| &**h);
    let required = gantz_core::reg::required_commits(&env, &registry, head_iter);
    registry.prune_unreachable(&required);
    views.retain(|ca, _| required.contains(ca));
}

fn setup_vm(world: &mut World) {
    bevy::log::info!("Setting up VMs for all open heads!");

    // Collect entity IDs first.
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, With<OpenHead>>()
        .iter(world)
        .collect();

    // Initialize VMs.
    let mut vms = HeadVms::default();
    let mut compiled_updates: Vec<(Entity, String)> = vec![];
    for entity in entities {
        let registry = world.resource::<Registry<Box<dyn node::Node>>>();
        let views = world.resource::<Views>();
        let builtins = world.resource::<BuiltinNodes<Box<dyn node::Node>>>();
        let env = Environment::new(registry, views, &*builtins);
        let Some(wg) = world.get::<WorkingGraph<Box<dyn node::Node>>>(entity) else {
            continue;
        };
        let (vm, module) = match bevy_gantz::vm::init(&env, &**wg) {
            Ok(result) => result,
            Err(e) => {
                bevy::log::error!("Failed to init VM for entity {entity}: {e}");
                continue;
            }
        };
        vms.insert(entity, vm);
        compiled_updates.push((entity, module));
    }

    // Update CompiledModule components.
    for (entity, compiled_module) in compiled_updates {
        if let Some(mut compiled) = world.get_mut::<CompiledModule>(entity) {
            *compiled = CompiledModule(compiled_module);
        }
    }

    world.insert_non_send_resource(vms);
}

fn update_gui(
    trace_capture: Res<TraceCapture>,
    mut perf_vm: ResMut<PerfVm>,
    mut perf_gui: ResMut<PerfGui>,
    mut ctxs: EguiContexts,
    mut registry: ResMut<Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
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

    // Construct environment on-demand for the widget.
    let mut env = Environment::new(&*registry, &*views, &*builtins);

    let level = bevy::log::tracing_subscriber::filter::LevelFilter::current();
    let response = egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctx, |ui| {
            gantz_egui::widget::Gantz::new(&mut env)
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

fn update_vm(
    mut ctxs: EguiContexts,
    mut registry: ResMut<Registry<Box<dyn node::Node>>>,
    mut views: ResMut<Views>,
    builtins: Res<BuiltinNodes<Box<dyn node::Node>>>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<HeadVms>,
    mut heads_query: Query<OpenHeadData<Box<dyn node::Node>>, With<OpenHead>>,
) {
    // Check for changes to each open graph and commit/recompile them.
    // FIXME: Rather than checking changed CA to monitor changes, ideally
    // `Gantz` widget can tell us this in a custom response.
    for mut data in heads_query.iter_mut() {
        let head: &mut ca::Head = &mut *data.head_ref;
        let graph: &Graph = &*data.working_graph;
        let head_views: &gantz_egui::GraphViews = &*data.views;

        // Always update the views in global Views for this head's commit.
        if let Some(commit_addr) = registry.head_commit_ca(head).copied() {
            views.insert(commit_addr, head_views.clone());
        }

        let new_graph_ca = ca::graph_addr(graph);
        let Some(head_commit) = registry.head_commit(head) else {
            continue;
        };
        if head_commit.graph != new_graph_ca {
            let old_head = head.clone();
            let old_commit_ca = registry.head_commit_ca(head).copied().unwrap();
            let new_commit_ca = registry.commit_graph_to_head(
                timestamp(),
                new_graph_ca,
                || bevy_gantz::clone_graph(graph),
                head,
            );
            bevy::log::debug!(
                "Graph changed: {} -> {}",
                old_commit_ca.display_short(),
                new_commit_ca.display_short()
            );
            // Update the graph pane if the head's commit CA changed.
            if let Ok(ctx) = ctxs.ctx_mut() {
                gantz_egui::widget::update_graph_pane_head(ctx, &old_head, head);
            }

            // Migrate open_heads entry from old key to new key.
            if let Some(state) = gui_state.open_heads.remove(&old_head) {
                gui_state.open_heads.insert(head.clone(), state);
            }

            // Re-register and recompile this head's graph into its VM.
            // Registration is idempotent - existing state is preserved.
            if let Some(vm) = vms.get_mut(&data.entity) {
                let env = Environment::new(&*registry, &*views, &*builtins);
                gantz_core::graph::register(&env, graph, &[], vm);
                match bevy_gantz::vm::compile(&env, graph, vm) {
                    Ok(module) => data.compiled.0 = module,
                    Err(e) => bevy::log::error!("Failed to compile graph: {e}"),
                }
            }
        }
    }
}

/// Insert an Inspect node on the given edge, replacing the edge with two edges.
fn inspect_edge(
    env: &Environment,
    wg: &mut WorkingGraph<Box<dyn node::Node>>,
    gv: &mut GraphViews,
    vm: &mut Engine,
    cmd: gantz_egui::InspectEdge,
) {
    use gantz_egui::widget::gantz::NodeTypeRegistry;

    let gantz_egui::InspectEdge { path, edge, pos } = cmd;

    let graph: &mut Graph = &mut *wg;
    let views: &mut gantz_egui::GraphViews = &mut *gv;

    // Navigate to the nested graph at the path.
    let Some(nested) = gantz_egui::widget::graph_scene::index_path_graph_mut(graph, &path) else {
        bevy::log::error!("InspectEdge: could not find graph at path");
        return;
    };

    // Get edge endpoints and weight.
    let Some((src_node, dst_node)) = nested.edge_endpoints(edge) else {
        bevy::log::error!("InspectEdge: edge not found");
        return;
    };
    let edge_weight = *nested.edge_weight(edge).unwrap();

    // Remove the edge.
    nested.remove_edge(edge);

    // Create a new Inspect node.
    let Some(inspect_node) = env.new_node("inspect") else {
        bevy::log::error!("InspectEdge: could not create inspect node");
        return;
    };
    let inspect_id = nested.add_node(inspect_node);

    // Determine the node path and register it with the VM.
    let node_path: Vec<_> = path
        .iter()
        .copied()
        .chain(Some(inspect_id.index()))
        .collect();
    nested[inspect_id].register(env, &node_path, vm);

    // Add edge: src -> inspect (using original output, input 0).
    nested.add_edge(
        src_node,
        inspect_id,
        gantz_core::Edge::new(edge_weight.output, gantz_core::node::Input(0)),
    );

    // Add edge: inspect -> dst (using output 0, original input).
    nested.add_edge(
        inspect_id,
        dst_node,
        gantz_core::Edge::new(gantz_core::node::Output(0), edge_weight.input),
    );

    // Position the new node at the click position.
    let node_id = egui_graph::NodeId::from_u64(inspect_id.index() as u64);
    let view = views.entry(path).or_default();
    view.layout.insert(node_id, pos);
}

// Drain the commands provided by the UI and process them.
fn process_gantz_gui_cmds(
    mut registry: ResMut<Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
    builtins: Res<BuiltinNodes<Box<dyn node::Node>>>,
    mut vms: NonSendMut<HeadVms>,
    mut gui_state: ResMut<GuiState>,
    mut heads: Query<OpenHeadData<Box<dyn node::Node>>, With<OpenHead>>,
    mut cmds: Commands,
) {
    // Collect heads with their entities to process.
    let heads_to_process: Vec<_> = heads
        .iter()
        .map(|data| (data.entity, (**data.head_ref).clone()))
        .collect();

    for (entity, head) in heads_to_process {
        let head_state = gui_state.open_heads.entry(head.clone()).or_default();
        for cmd in std::mem::take(&mut head_state.scene.cmds) {
            bevy::log::debug!("{cmd:?}");
            match cmd {
                gantz_egui::Cmd::PushEval(path) => {
                    cmds.trigger(eval::EvalEvent {
                        head: entity,
                        path,
                        kind: eval::EvalKind::Push,
                    });
                }
                gantz_egui::Cmd::PullEval(path) => {
                    cmds.trigger(eval::EvalEvent {
                        head: entity,
                        path,
                        kind: eval::EvalKind::Pull,
                    });
                }
                gantz_egui::Cmd::OpenGraph(path) => {
                    // Re-borrow head_state to modify path.
                    let head_state = gui_state.open_heads.get_mut(&head).unwrap();
                    head_state.path = path;
                }
                gantz_egui::Cmd::OpenNamedNode(name, content_ca) => {
                    // The content_ca represents a CommitAddr for graph nodes.
                    let commit_ca = ca::CommitAddr::from(content_ca);
                    if registry.names().get(&name) == Some(&commit_ca) {
                        cmds.trigger(head::OpenEvent(ca::Head::Branch(name.to_string())));
                    } else {
                        bevy::log::debug!(
                            "Attempted to open named node, but the content address has changed"
                        );
                    }
                }
                gantz_egui::Cmd::ForkNamedNode { new_name, ca } => {
                    // The CA represents a CommitAddr for graph nodes.
                    let commit_ca = ca::CommitAddr::from(ca);
                    registry.insert_name(new_name.clone(), commit_ca);
                    bevy::log::info!("Forked node to new name: {new_name}");
                }
                gantz_egui::Cmd::InspectEdge(cmd) => {
                    if let Ok(mut data) = heads.get_mut(entity) {
                        if let Some(vm) = vms.get_mut(&entity) {
                            let env = Environment::new(&*registry, &*views, &*builtins);
                            inspect_edge(&env, &mut data.working_graph, &mut data.views, vm, cmd);
                        }
                    }
                }
            }
        }
    }
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

// ----------------------------------------------------------------------------
// Hook Observers (respond to hook events from bevy_gantz handlers)
// ----------------------------------------------------------------------------

/// VM init for opened heads.
fn on_head_opened(
    trigger: On<head::HeadOpened>,
    registry: Res<Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
    builtins: Res<BuiltinNodes<Box<dyn node::Node>>>,
    mut vms: NonSendMut<HeadVms>,
    mut cmds: Commands,
    graphs: Query<&WorkingGraph<Box<dyn node::Node>>>,
) {
    let event = trigger.event();
    let graph = graphs.get(event.entity).unwrap();
    let env = Environment::new(&*registry, &*views, &*builtins);
    let (vm, module) = match bevy_gantz::vm::init(&env, &**graph) {
        Ok(result) => result,
        Err(e) => {
            bevy::log::error!("Failed to init VM for new head: {e}");
            return;
        }
    };
    cmds.entity(event.entity).insert(CompiledModule(module));
    vms.insert(event.entity, vm);
}

/// VM init for replaced heads.
fn on_head_replaced(
    trigger: On<head::HeadReplaced>,
    registry: Res<Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
    builtins: Res<BuiltinNodes<Box<dyn node::Node>>>,
    mut vms: NonSendMut<HeadVms>,
    mut cmds: Commands,
    graphs: Query<&WorkingGraph<Box<dyn node::Node>>>,
) {
    let event = trigger.event();
    let graph = graphs.get(event.entity).unwrap();
    let env = Environment::new(&*registry, &*views, &*builtins);
    let (vm, module) = match bevy_gantz::vm::init(&env, &**graph) {
        Ok(result) => result,
        Err(e) => {
            bevy::log::error!("Failed to init VM for replaced head: {e}");
            return;
        }
    };
    cmds.entity(event.entity).insert(CompiledModule(module));
    vms.insert(event.entity, vm);
}
