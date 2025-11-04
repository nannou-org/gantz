use bevy::{
    prelude::*,
    window::{Window, WindowPlugin},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use bevy_gantz::debounced_input::{DebouncedInputEvent, DebouncedInputPlugin};
use bevy_pkv::PkvStore;
use env::Environment;
use gantz_egui::ContentAddr;
use graph::{Graph, GraphNode};
use steel::{SteelVal, parser::ast::ExprKind, steel_vm::engine::Engine};

mod env;
mod graph;
mod node;
mod storage;

/// The active graph.
///
/// If we're working with a name, a mapping from the name to the graph's CA
/// will be persisted.
#[derive(Resource)]
struct Active {
    graph: GraphNode,
    graph_ca: ContentAddr,
    graph_name: Option<String>,
}

#[derive(Resource)]
struct GuiState {
    gantz: gantz_egui::widget::GantzState,
}

/// The full compiled module as a `String`.
#[derive(Resource)]
struct CompiledModule(String);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(window_plugin()))
        .add_plugins(EguiPlugin::default())
        .add_plugins(DebouncedInputPlugin::new(0.25))
        .insert_resource(PkvStore::new("nannou-org", "gantz"))
        .add_systems(
            Startup,
            (
                setup_camera,
                setup_environment,
                setup_active.after(setup_environment),
                prune_unused_graphs
                    .after(setup_environment)
                    .after(setup_active),
                setup_vm.after(prune_unused_graphs),
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

fn window_plugin() -> WindowPlugin {
    WindowPlugin {
        primary_window: Some(Window {
            title: "gantz".into(),
            name: Some("gantz".into()),
            fit_canvas_to_parent: true,
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

fn setup_active(storage: Res<PkvStore>, env: Res<Environment>, mut cmds: Commands) {
    let active = storage::load_active(&*storage, &env.registry);
    cmds.insert_resource(active);
}

fn prune_unused_graphs(mut env: ResMut<Environment>, active: ResMut<Active>) {
    env::prune_unused_graphs(&mut env.registry, active.graph_ca);
}

fn setup_gui_state(storage: Res<PkvStore>, mut cmds: Commands) {
    let gantz = storage::load_gantz_gui_state(&*storage);
    let gui = GuiState { gantz };
    cmds.insert_resource(gui);
}

fn setup_vm(world: &mut World) {
    bevy::log::info!("Setting up VM!");
    let env = world.resource_ref::<Environment>();
    let active = world.resource_ref::<Active>();
    let (vm, compiled_module) = init_vm(&*env, &active.graph.graph);
    world.insert_non_send_resource(vm);
    world.insert_resource(compiled_module);
}

fn update_gui(
    mut ctxs: EguiContexts,
    mut env: ResMut<Environment>,
    mut active: ResMut<Active>,
    mut gui_state: ResMut<GuiState>,
    mut vm: NonSendMut<Engine>,
    mut compiled_module: ResMut<CompiledModule>,
) -> Result {
    let ctx = ctxs.ctx_mut()?;
    egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctx, |ui| {
            let ca = active.graph_ca;
            let graph_name = active.graph_name.clone();
            let name = graph_name.as_deref();
            let head = gantz_egui::widget::graph_select::Head { ca, name };
            let response = gantz_egui::widget::Gantz::new(&mut *env, &mut active.graph, head).show(
                &mut gui_state.gantz,
                None, // logger
                &compiled_module.0,
                &mut vm,
                ui,
            );

            // The graph name was updated, ensure a mapping exists if necessary.
            if let Some(name_opt) = response.graph_name_updated() {
                // If a name was given, ensure it maps to the CA.
                if let Some(ref name) = name_opt {
                    env.registry.names.insert(name.to_string(), ca);
                }
                active.graph_name = name_opt.clone();
            // The given graph name was removed.
            } else if let Some(name) = response.graph_name_removed() {
                if Some(&name) == active.graph_name.as_ref() {
                    active.graph_name.take();
                }
                env.registry.names.remove(&name);
            }

            // A graph was selected.
            if let Some((name, ca)) = response.graph_selected() {
                // TODO: Load state for named graphs?
                set_head(
                    &mut env,
                    &mut active,
                    &mut vm,
                    &mut compiled_module,
                    &mut gui_state.gantz,
                    *ca,
                    name.clone(),
                );
            }

            // Create a new empty graph and select it.
            if response.new_graph() {
                let graph = Graph::default();
                let ca = gantz_egui::graph_content_addr(&graph);
                env.registry.graphs.insert(ca, graph);
                set_head(
                    &mut env,
                    &mut active,
                    &mut vm,
                    &mut compiled_module,
                    &mut gui_state.gantz,
                    ca,
                    None,
                );
            }
        });
    Ok(())
}

fn update_vm(
    mut env: ResMut<Environment>,
    mut active: ResMut<Active>,
    mut vm: NonSendMut<Engine>,
    mut compiled_module: ResMut<CompiledModule>,
) {
    // Check for changes to the graph.
    // FIXME: Rather than checking changed CA to monitor changes, ideally
    // `Gantz` widget can tell us this in a custom response.
    let new_graph_ca = gantz_egui::graph_content_addr(&active.graph);
    if active.graph_ca != new_graph_ca {
        active.graph_ca = new_graph_ca;
        // If there's some name tracking the graph changes, ensure the
        // mapping is updated.
        if let Some(name) = active.graph_name.as_ref() {
            let graph = graph::clone(&active.graph.graph);
            env.registry.graphs.entry(new_graph_ca).or_insert(graph);
            env.registry.names.insert(name.clone(), new_graph_ca);
        }
        let module = compile_graph(&env, &active.graph, &mut vm);
        *compiled_module = CompiledModule(fmt_compiled_module(&module));
    }
}

// Drain the commands provided by the UI and process them.
fn process_gantz_gui_cmds(
    mut env: ResMut<Environment>,
    mut active: ResMut<Active>,
    mut vm: NonSendMut<Engine>,
    mut compiled_module: ResMut<CompiledModule>,
    mut gui_state: ResMut<GuiState>,
) {
    // Process any pending commands.
    for cmd in std::mem::take(&mut gui_state.gantz.graph_scene.cmds) {
        bevy::log::debug!("{cmd:?}");
        match cmd {
            gantz_egui::Cmd::PushEval(path) => {
                let fn_name = gantz_core::compile::push_eval_fn_name(&path);
                if let Err(e) = vm.call_function_by_name_with_args(&fn_name, vec![]) {
                    bevy::log::error!("{e}");
                }
            }
            gantz_egui::Cmd::PullEval(path) => {
                let fn_name = gantz_core::compile::pull_eval_fn_name(&path);
                if let Err(e) = vm.call_function_by_name_with_args(&fn_name, vec![]) {
                    bevy::log::error!("{e}");
                }
            }
            gantz_egui::Cmd::OpenGraph(path) => {
                gui_state.gantz.path = path;
            }
            gantz_egui::Cmd::OpenNamedGraph(name, ca) => {
                if let Some(&n_ca) = env.registry.names.get(&name) {
                    if ca == n_ca {
                        set_head(
                            &mut env,
                            &mut active,
                            &mut vm,
                            &mut compiled_module,
                            &mut gui_state.gantz,
                            ca,
                            Some(name),
                        );
                    }
                }
            }
        }
    }
}

fn persist_resources(
    mut env: ResMut<Environment>,
    active: Res<Active>,
    gui_state: Res<GuiState>,
    mut storage: ResMut<PkvStore>,
) {
    // Ensure the active graph is registered.
    let active_ca = gantz_egui::graph_content_addr(&active.graph.graph);
    env.registry
        .graphs
        .entry(active_ca)
        .or_insert_with(|| graph::clone(&active.graph.graph));

    // Save the graph addresses, the graphs and the graph names.
    let mut addrs: Vec<_> = env.registry.graphs.keys().copied().collect();
    addrs.sort();
    storage::save_graph_addrs(&mut *storage, &addrs);

    storage::save_graphs(&mut *storage, &env.registry.graphs);
    storage::save_graph_names(&mut *storage, &env.registry.names);

    // Save the active graph.
    storage::save_active_graph(&mut *storage, active_ca);
    storage::save_active_graph_name(&mut *storage, active.graph_name.as_deref());

    // Save the gantz GUI state.
    storage::save_gantz_gui_state(&mut *storage, &gui_state.gantz);
}

/// Initialise the VM for the given environment and graph.
///
/// Also returns the compiled module for the initial state.
///
/// TODO: Allow loading state from storage.
fn init_vm(env: &Environment, graph: &Graph) -> (Engine, CompiledModule) {
    let mut vm = Engine::new();
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

/// Set the active graph as the graph with the given CA.
fn set_head(
    env: &mut Environment,
    active: &mut Active,
    vm: &mut Engine,
    compiled_module: &mut CompiledModule,
    gantz: &mut gantz_egui::widget::GantzState,
    ca: ContentAddr,
    name: Option<String>,
) {
    let graph = &env.registry.graphs[&ca];

    // Clone the graph.
    let graph = graph::clone(graph);
    active.graph_ca = ca;
    active.graph_name = name;
    active.graph = gantz_core::node::GraphNode { graph };

    // Initialise the VM.
    let (new_vm, new_module) = init_vm(env, &active.graph.graph);
    *vm = new_vm;
    *compiled_module = new_module;

    // Clear the graph GUI state (layout, etc).
    gantz.path.clear();
    gantz.graphs.clear();
    gantz.graph_scene.interaction.selection.clear();
}
