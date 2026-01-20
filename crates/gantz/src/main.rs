use bevy::{
    prelude::*,
    window::{Window, WindowPlugin},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use bevy_gantz::debounced_input::{DebouncedInputEvent, DebouncedInputPlugin};
use bevy_pkv::PkvStore;
use env::Environment;
use gantz_ca as ca;
use graph::Graph;
use steel::{SteelVal, parser::ast::ExprKind, steel_vm::engine::Engine};

mod env;
mod graph;
mod node;
mod storage;

/// The currently open graphs/heads.
///
/// Each entry is a head (branch or commit), its associated graph, and its views.
/// Multiple heads can be open simultaneously, displayed as tabs.
#[derive(Resource)]
struct Open {
    heads: Vec<(ca::Head, Graph, env::GraphViews)>,
}

#[derive(Resource)]
struct GuiState {
    gantz: gantz_egui::widget::GantzState,
}

/// The compiled module for a single graph as a `String`.
struct CompiledModule(String);

/// Per-head compiled modules.
///
/// Each entry corresponds to a head in the `Open` resource at the same index.
#[derive(Resource)]
struct CompiledModules(Vec<CompiledModule>);

/// Per-head VMs.
///
/// This is a non-send resource because `Engine` is not `Send`.
/// Each entry corresponds to a head in the `Open` resource at the same index.
struct HeadVms(Vec<Engine>);

/// A resource for capturing tracing logs for the `TraceView` widget.
#[derive(Default, Resource)]
struct TraceCapture(gantz_egui::widget::trace_view::TraceCapture);

fn main() {
    App::new()
        .insert_resource(TraceCapture::default())
        .add_plugins(DefaultPlugins.set(log_plugin()).set(window_plugin()))
        .add_plugins(EguiPlugin::default())
        .add_plugins(DebouncedInputPlugin::new(0.25))
        .insert_resource(PkvStore::new("nannou-org", "gantz"))
        .add_systems(
            Startup,
            (
                setup_camera,
                setup_environment,
                setup_open.after(setup_environment),
                prune_unused_graphs_and_commits
                    .after(setup_environment)
                    .after(setup_open),
                setup_vm.after(prune_unused_graphs_and_commits),
                setup_gui_state,
            ),
        )
        .add_systems(EguiPrimaryContextPass, update_gui)
        .add_systems(
            Update,
            (
                update_vm.after(update_gui),
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

fn setup_environment(storage: Res<PkvStore>, mut cmds: Commands) {
    let env = storage::load_environment(&*storage);
    cmds.insert_resource(env);
}

fn setup_open(storage: Res<PkvStore>, mut env: ResMut<Environment>, mut cmds: Commands) {
    let open = storage::load_open(&*storage, &mut *env);
    cmds.insert_resource(open);
}

fn prune_unused_graphs_and_commits(mut env: ResMut<Environment>, open: Res<Open>) {
    let heads = open.heads.iter().map(|(h, _, _)| h);
    env.registry
        .prune_unnamed_graphs(heads, env::graph_contains);

    // Prune views for commits that no longer exist.
    let existing_commits: std::collections::HashSet<_> =
        env.registry.commits().keys().copied().collect();
    env.views
        .retain(|commit_addr, _| existing_commits.contains(commit_addr));
}

fn setup_gui_state(storage: Res<PkvStore>, mut cmds: Commands) {
    let gantz = storage::load_gantz_gui_state(&*storage);
    let gui = GuiState { gantz };
    cmds.insert_resource(gui);
}

fn setup_vm(world: &mut World) {
    bevy::log::info!("Setting up VMs for all open heads!");
    let env = world.resource_ref::<Environment>();
    let open = world.resource_ref::<Open>();

    // Initialize a VM for each open head.
    let mut vms = Vec::with_capacity(open.heads.len());
    let mut compiled_modules = Vec::with_capacity(open.heads.len());
    for (_, graph, _) in &open.heads {
        let (vm, compiled_module) = init_vm(&*env, graph);
        vms.push(vm);
        compiled_modules.push(compiled_module);
    }

    world.insert_non_send_resource(HeadVms(vms));
    world.insert_resource(CompiledModules(compiled_modules));
}

fn update_gui(
    trace_capture: Res<TraceCapture>,
    mut ctxs: EguiContexts,
    mut env: ResMut<Environment>,
    mut open: ResMut<Open>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<HeadVms>,
    mut compiled_modules: ResMut<CompiledModules>,
    mut storage: ResMut<PkvStore>,
    mut memory_loaded: Local<bool>,
) -> Result {
    let ctx = ctxs.ctx_mut()?;

    // Load egui memory once on first frame
    if !*memory_loaded {
        storage::load_egui_memory(&mut *storage, ctx);
        *memory_loaded = true;
    }
    egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctx, |ui| {
            // Build a slice of (Head, &mut Graph, &mut GraphLayout) for the Gantz widget.
            let mut heads: Vec<_> = open
                .heads
                .iter_mut()
                .map(|(h, g, l)| (h.clone(), g, l))
                .collect();
            let get_module = |ix: usize| compiled_modules.0.get(ix).map(|m| m.0.as_str());
            let response = gantz_egui::widget::Gantz::new(&mut *env, &mut heads)
                .trace_capture(trace_capture.0.clone())
                .show(&mut gui_state.gantz, &get_module, &mut vms.0, ui);

            // The given graph name was removed.
            if let Some(name) = response.graph_name_removed() {
                // Update any open heads that reference this name.
                for (head, _, _) in &mut open.heads {
                    if let ca::Head::Branch(head_name) = &*head {
                        if *head_name == name {
                            let commit_ca = *env.registry.head_commit_ca(head).unwrap();
                            *head = ca::Head::Commit(commit_ca);
                        }
                    }
                }
                env.registry.remove_name(&name);
            }

            // Single click: replace the focused head with the selected one.
            if let Some(new_head) = response.graph_replaced() {
                replace_head(
                    ctx,
                    &mut env,
                    &mut open,
                    &mut vms,
                    &mut compiled_modules,
                    &mut gui_state.gantz,
                    new_head.clone(),
                );
            }

            // Open head as a new tab (or focus if already open).
            if let Some(new_head) = response.graph_opened() {
                open_head(
                    &mut env,
                    &mut open,
                    &mut vms,
                    &mut compiled_modules,
                    &mut gui_state.gantz,
                    new_head.clone(),
                );
            }

            // Close head.
            if let Some(head) = response.graph_closed() {
                close_head(
                    &mut open,
                    &mut vms,
                    &mut compiled_modules,
                    &mut gui_state.gantz,
                    head,
                );
            }

            // Create a new empty graph and open it.
            if response.new_graph() {
                let new_head = env.registry.init_head(env::timestamp());
                open_head(
                    &mut env,
                    &mut open,
                    &mut vms,
                    &mut compiled_modules,
                    &mut gui_state.gantz,
                    new_head,
                );
            }

            // Handle closed heads from tab close buttons.
            for closed_head in &response.closed_heads {
                close_head(
                    &mut open,
                    &mut vms,
                    &mut compiled_modules,
                    &mut gui_state.gantz,
                    closed_head,
                );
            }

            // Handle new branch created from tab double-click.
            if let Some((original_head, new_name)) = response.new_branch() {
                create_branch_from_head(
                    ctx,
                    &mut env,
                    &mut open,
                    &mut gui_state.gantz,
                    original_head,
                    new_name.clone(),
                );
            }
        });
    Ok(())
}

fn update_vm(
    mut ctxs: EguiContexts,
    mut env: ResMut<Environment>,
    mut open: ResMut<Open>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<HeadVms>,
    mut compiled_modules: ResMut<CompiledModules>,
) {
    // Check for changes to each open graph and commit/recompile them.
    // FIXME: Rather than checking changed CA to monitor changes, ideally
    // `Gantz` widget can tell us this in a custom response.
    for (ix, (head, graph, views)) in open.heads.iter_mut().enumerate() {
        // Always update the views in env.views for this head's commit.
        if let Some(commit_addr) = env.registry.head_commit_ca(head).copied() {
            env.views.insert(commit_addr, views.clone());
        }

        let new_graph_ca = ca::graph_addr(&*graph);
        let head_commit = env.registry.head_commit(head).unwrap();
        if head_commit.graph != new_graph_ca {
            let old_head = head.clone();
            env.registry.commit_graph_to_head(
                env::timestamp(),
                new_graph_ca,
                || graph::clone(graph),
                head,
            );
            // Update the graph pane if the head's commit CA changed.
            if let Ok(ctx) = ctxs.ctx_mut() {
                gantz_egui::widget::update_graph_pane_head(ctx, &old_head, head);
            }

            // Migrate open_heads entry from old key to new key.
            if let Some(state) = gui_state.gantz.open_heads.remove(&old_head) {
                gui_state.gantz.open_heads.insert(head.clone(), state);
            }

            // Recompile this head's graph into its VM.
            let vm = &mut vms.0[ix];
            let module = compile_graph(&env, graph, vm);
            compiled_modules.0[ix] = CompiledModule(fmt_compiled_module(&module));
        }
    }
}

// Drain the commands provided by the UI and process them.
fn process_gantz_gui_cmds(
    mut env: ResMut<Environment>,
    mut open: ResMut<Open>,
    mut vms: NonSendMut<HeadVms>,
    mut compiled_modules: ResMut<CompiledModules>,
    mut gui_state: ResMut<GuiState>,
) {
    // Collect heads with their indices to process.
    let heads_to_process: Vec<_> = open
        .heads
        .iter()
        .enumerate()
        .map(|(ix, (h, _, _))| (ix, h.clone()))
        .collect();

    for (ix, head) in heads_to_process {
        let head_state = gui_state.gantz.open_heads.entry(head.clone()).or_default();
        for cmd in std::mem::take(&mut head_state.scene.cmds) {
            bevy::log::debug!("{cmd:?}");
            match cmd {
                gantz_egui::Cmd::PushEval(path) => {
                    let fn_name = gantz_core::compile::push_eval_fn_name(&path);
                    if let Err(e) = vms.0[ix].call_function_by_name_with_args(&fn_name, vec![]) {
                        bevy::log::error!("{e}");
                    }
                }
                gantz_egui::Cmd::PullEval(path) => {
                    let fn_name = gantz_core::compile::pull_eval_fn_name(&path);
                    if let Err(e) = vms.0[ix].call_function_by_name_with_args(&fn_name, vec![]) {
                        bevy::log::error!("{e}");
                    }
                }
                gantz_egui::Cmd::OpenGraph(path) => {
                    // Re-borrow head_state to modify path.
                    let head_state = gui_state.gantz.open_heads.get_mut(&head).unwrap();
                    head_state.path = path;
                }
                gantz_egui::Cmd::OpenNamedGraph(name, graph_ca) => {
                    if let Some(commit) = env.registry.named_commit(&name) {
                        if graph_ca == commit.graph {
                            open_head(
                                &mut env,
                                &mut open,
                                &mut vms,
                                &mut compiled_modules,
                                &mut gui_state.gantz,
                                ca::Head::Branch(name.to_string()),
                            );
                        } else {
                            bevy::log::debug!(
                                "Attempted to open named graph, but the graph address has changed"
                            );
                        }
                    }
                }
            }
        }
    }
}

fn persist_resources(
    env: Res<Environment>,
    open: Res<Open>,
    gui_state: Res<GuiState>,
    mut storage: ResMut<PkvStore>,
    mut ctxs: EguiContexts,
) {
    // Save graphs.
    let mut addrs: Vec<_> = env.registry.graphs().keys().copied().collect();
    addrs.sort();
    storage::save_graph_addrs(&mut *storage, &addrs);
    storage::save_graphs(&mut *storage, &env.registry.graphs());

    // Save commits.
    let mut addrs: Vec<_> = env.registry.commits().keys().copied().collect();
    addrs.sort();
    storage::save_commit_addrs(&mut *storage, &addrs);
    storage::save_commits(&mut *storage, env.registry.commits());

    // Save names.
    storage::save_names(&mut *storage, env.registry.names());

    // Save all open heads.
    let heads: Vec<_> = open.heads.iter().map(|(h, _, _)| h.clone()).collect();
    storage::save_open_heads(&mut *storage, &heads);

    // Save all views (already updated in update_vm).
    storage::save_views(&mut *storage, &env.views);

    // Save the gantz GUI state.
    storage::save_gantz_gui_state(&mut *storage, &gui_state.gantz);

    // Save egui memory (widget states).
    if let Ok(ctx) = ctxs.ctx_mut() {
        storage::save_egui_memory(&mut *storage, ctx);
    }
}

/// Initialise the VM for the given environment and graph.
///
/// Also returns the compiled module for the initial state.
///
/// TODO: Allow loading state from storage.
fn init_vm(env: &Environment, graph: &Graph) -> (Engine, CompiledModule) {
    let mut vm = Engine::new_base();
    vm.register_value(gantz_core::ROOT_STATE, SteelVal::empty_hashmap());
    gantz_core::graph::register(env, graph, &[], &mut vm);
    let module = compile_graph(env, graph, &mut vm);
    let compiled_module = CompiledModule(fmt_compiled_module(&module));
    (vm, compiled_module)
}

fn compile_graph(env: &Environment, graph: &Graph, vm: &mut Engine) -> Vec<ExprKind> {
    // Generate the steel module.
    let module = gantz_core::compile::module(env, graph);
    // Compile the eval fns.
    for expr in &module {
        if let Err(e) = vm.run(expr.to_pretty(80)) {
            bevy::log::error!("{e}");
        }
    }
    module
}

fn fmt_compiled_module(module: &[ExprKind]) -> String {
    module
        .iter()
        .map(|expr| expr.to_pretty(80))
        .collect::<Vec<String>>()
        .join("\n\n")
}

/// Open a head as a new tab, or focus it if already open.
///
/// This is only used when selecting from GraphSelect.
fn open_head(
    env: &mut Environment,
    open: &mut Open,
    vms: &mut HeadVms,
    compiled_modules: &mut CompiledModules,
    gantz: &mut gantz_egui::widget::GantzState,
    new_head: ca::Head,
) {
    // Check if the head is already open.
    if let Some(ix) = open.heads.iter().position(|(h, _, _)| *h == new_head) {
        // Just focus the existing tab.
        gantz.focused_head = ix;
        return;
    }

    // Head is not open - add it as a new tab.
    let graph = env.registry.head_graph(&new_head).unwrap();
    let new_graph = graph::clone(graph);

    // Load the views for this head's commit, or create empty.
    let views = env
        .registry
        .head_commit_ca(&new_head)
        .and_then(|ca| env.views.get(&ca).cloned())
        .unwrap_or_default();

    open.heads
        .push((new_head.clone(), new_graph.clone(), views));
    gantz.focused_head = open.heads.len() - 1;

    // Initialise the VM for the new graph and add it to the per-head collections.
    let (new_vm, new_module) = init_vm(env, &new_graph);
    vms.0.push(new_vm);
    compiled_modules.0.push(new_module);

    // Initialize GUI state for the new head.
    gantz.open_heads.entry(new_head).or_default();
}

/// Replace the focused head with a new head in-place.
///
/// If the new head is already open elsewhere, focuses that instead.
fn replace_head(
    ctx: &egui::Context,
    env: &mut Environment,
    open: &mut Open,
    vms: &mut HeadVms,
    compiled_modules: &mut CompiledModules,
    gantz: &mut gantz_egui::widget::GantzState,
    new_head: ca::Head,
) {
    // If the new head is already open, just focus it.
    if let Some(ix) = open.heads.iter().position(|(h, _, _)| *h == new_head) {
        gantz.focused_head = ix;
        return;
    }

    let ix = gantz.focused_head;
    let old_head = open.heads[ix].0.clone();

    // Load the new graph.
    let graph = env.registry.head_graph(&new_head).unwrap();
    let new_graph = graph::clone(graph);

    // Load the views for this head's commit, or create empty.
    let views = env
        .registry
        .head_commit_ca(&new_head)
        .and_then(|ca| env.views.get(&ca).cloned())
        .unwrap_or_default();

    // Replace at the focused index.
    open.heads[ix] = (new_head.clone(), new_graph.clone(), views);

    // Reinitialize the VM for the new graph.
    let (new_vm, new_module) = init_vm(env, &new_graph);
    vms.0[ix] = new_vm;
    compiled_modules.0[ix] = new_module;

    // Update the graph pane to show the new head.
    gantz_egui::widget::update_graph_pane_head(ctx, &old_head, &new_head);

    // Move GUI state from old head to new head.
    if let Some(state) = gantz.open_heads.remove(&old_head) {
        gantz.open_heads.insert(new_head, state);
    } else {
        gantz.open_heads.entry(new_head).or_default();
    }
}

/// Close a head, removing it from the open tabs.
///
/// Does nothing if the head is not open or if it's the last open head.
fn close_head(
    open: &mut Open,
    vms: &mut HeadVms,
    compiled_modules: &mut CompiledModules,
    gantz: &mut gantz_egui::widget::GantzState,
    head: &ca::Head,
) {
    // Don't close if it's the last open head.
    // TODO: Consider opening default empty graph when closing last head.
    if open.heads.len() <= 1 {
        return;
    }
    if let Some(ix) = open.heads.iter().position(|(h, _, _)| h == head) {
        open.heads.remove(ix);
        vms.0.remove(ix);
        compiled_modules.0.remove(ix);
        gantz.open_heads.remove(head);

        // Update focused_head to remain valid.
        if ix <= gantz.focused_head {
            gantz.focused_head = gantz.focused_head.saturating_sub(1);
        }
    }
}

/// Create a new branch from an existing head and replace the open head with it.
fn create_branch_from_head(
    ctx: &egui::Context,
    env: &mut Environment,
    open: &mut Open,
    gantz: &mut gantz_egui::widget::GantzState,
    original_head: &ca::Head,
    new_name: String,
) {
    // Get the commit CA from the original head.
    let Some(commit_ca) = env.registry.head_commit_ca(original_head).copied() else {
        bevy::log::error!("Failed to get commit address for head: {:?}", original_head);
        return;
    };

    // Insert the new branch name into the registry.
    env.registry.insert_name(new_name.clone(), commit_ca);

    // Find the index of the original head and replace it.
    let new_head = ca::Head::Branch(new_name);
    if let Some(ix) = open.heads.iter().position(|(h, _, _)| h == original_head) {
        let old_head = open.heads[ix].0.clone();
        open.heads[ix].0 = new_head.clone();

        // Update the graph pane to show the new head.
        gantz_egui::widget::update_graph_pane_head(ctx, &old_head, &new_head);

        // Move GUI state from old head to new head.
        if let Some(state) = gantz.open_heads.remove(&old_head) {
            gantz.open_heads.insert(new_head, state);
        }
    }
}
