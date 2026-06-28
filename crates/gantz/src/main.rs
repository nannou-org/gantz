use bevy::{
    prelude::*,
    window::{PrimaryWindow, Window},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_gantz::{
    BuiltinNodes, FocusedHead, GantzPlugin, HeadRef, HeadTabOrder, OpenHead, OpenHeadDataReadOnly,
    Registry, WorkingGraph,
    debounced_input::{DebouncedEvent, DebouncedInputEvent, DebouncedInputPlugin},
    reg, timestamp,
};
use bevy_gantz_egui::{GantzEguiPlugin, GuiState, HeadGuiState, TraceCapture, Views};
use bevy_pkv::PkvStore;
use builtin::Builtins;
use storage::Pkv;

#[cfg(not(target_arch = "wasm32"))]
use bevy::tasks::{IoTaskPool, Task, block_on};

mod builtin;
mod node;
mod storage;
mod window;

fn main() {
    let mut app = App::new();
    // Core gantz plugin (provides FocusedHead, HeadTabOrder, HeadVms, Registry, Views)
    app.add_plugins(GantzPlugin::<Box<dyn node::Node>>::default())
        // Egui plugin (provides GuiState, TraceCapture, PerfVm, PerfGui, GUI systems)
        .add_plugins(GantzEguiPlugin::<Box<dyn node::Node>>::default())
        // App-specific builtins
        .insert_resource(BuiltinNodes::<Box<dyn node::Node>>(Box::new(
            Builtins::new(),
        )))
        .add_plugins(DefaultPlugins.set(log_plugin()).set(window::plugin()))
        .add_plugins(EguiPlugin::default())
        .add_plugins(DebouncedInputPlugin::<DebouncedInputEvent>::new(0.25))
        // A separate, slower debounce for egui memory so it doesn't persist on
        // the same frame as the registry/views.
        .add_plugins(DebouncedInputPlugin::<PersistEguiMemory>::new(0.6))
        .insert_resource(Pkv::new(PkvStore::new("nannou-org", "gantz")))
        .add_systems(
            Startup,
            (
                setup_camera,
                setup_window,
                setup_resources,
                bevy_gantz_egui::base::load::<Box<dyn node::Node>>
                    .after(setup_resources)
                    .before(setup_open),
                setup_open.after(setup_resources),
                reg::prune_unused::<Box<dyn node::Node>>
                    .after(setup_resources)
                    .after(setup_open),
                // Spawn the background disk writer once the store is populated.
                setup_persister.after(setup_resources).after(setup_open),
            ),
        )
        .add_systems(EguiPrimaryContextPass, load_egui_memory)
        .add_systems(
            Update,
            persist_resources
                // After `settle_layout` so a layout commit settled this frame
                // (and its seeded view) is saved in the same pass.
                .after(bevy_gantz_egui::settle_layout::<Box<dyn node::Node>>)
                .run_if(on_message::<DebouncedInputEvent>),
        )
        .add_systems(
            Update,
            persist_egui_memory.run_if(on_message::<PersistEguiMemory>),
        );
    // Flush the background writer before the process exits (native only; wasm
    // writes inline, and the World is consumed by the runner so this can't run
    // after `run()`).
    #[cfg(not(target_arch = "wasm32"))]
    app.add_systems(Last, flush_on_exit);
    app.run();
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

fn setup_camera(mut cmds: Commands) {
    cmds.spawn(Camera2d);
}

/// Restore the persisted window size (native only; no-op on web).
fn setup_window(storage: Res<Pkv>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    if let Ok(mut window) = windows.single_mut() {
        window::apply_saved_size(&*storage, &mut window);
    }
}

fn setup_resources(storage: Res<Pkv>, mut cmds: Commands) {
    let registry: Registry<Box<dyn node::Node>> = bevy_gantz::storage::load_registry(&*storage);
    // Seed the persist tracker from the disk-loaded registry: everything loaded
    // is, by definition, already on disk. Done before `base::load` merges base
    // graphs (so they're written on first persist) and before `prune_unused`
    // (so prunes are detected on the first incremental save).
    let persisted = bevy_gantz::storage::PersistedRegistry::from_registry(&registry);
    let views = bevy_gantz_egui::storage::load_views(&*storage);
    let gui_state = bevy_gantz_egui::storage::load_gui_state(&*storage);
    cmds.insert_resource(registry);
    cmds.insert_resource(persisted);
    cmds.insert_resource(views);
    cmds.insert_resource(gui_state);
}

fn setup_open(
    storage: Res<Pkv>,
    mut registry: ResMut<Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
    mut cmds: Commands,
    mut tab_order: ResMut<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
) {
    let loaded =
        bevy_gantz_egui::storage::load_open(&*storage, &mut *registry, &*views, timestamp());
    let focused_head = bevy_gantz::storage::load_focused_head(&*storage);

    // Spawn entities for each open head. `OpenHead`'s required components
    // cover the compile outcome; `vm::sync` initializes the VMs on the first
    // `Update`.
    for (head, graph, head_views) in loaded {
        let is_focused = focused_head.as_ref() == Some(&head);
        let entity = cmds
            .spawn((
                OpenHead,
                HeadRef(head),
                WorkingGraph(graph),
                head_views,
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

/// Load egui memory from storage once on first frame.
fn load_egui_memory(mut ctxs: EguiContexts, mut storage: ResMut<Pkv>, mut loaded: Local<bool>) {
    if !*loaded {
        if let Ok(ctx) = ctxs.ctx_mut() {
            bevy_gantz_egui::storage::load_egui_memory(&mut *storage, ctx);
            *loaded = true;
        }
    }
}

/// Hands persistence batches to a background writer (native) or writes them
/// inline on the main thread (wasm - localStorage, no fsync, no threads).
///
/// The native/wasm split is encapsulated here so the persist systems stay
/// platform-agnostic: they build a batch and call [`submit`](Self::submit).
#[derive(Resource)]
struct Persister {
    #[cfg(not(target_arch = "wasm32"))]
    tx: async_channel::Sender<Vec<(String, String)>>,
    #[cfg(not(target_arch = "wasm32"))]
    task: Option<Task<()>>,
    #[cfg(target_arch = "wasm32")]
    store: Pkv,
}

impl Persister {
    /// Hand a batch of `(key, value)` writes off for persistence.
    ///
    /// Non-blocking on native (the worker does the fsync'd writes); inline on
    /// wasm.
    fn submit(&mut self, batch: Vec<(String, String)>) {
        if batch.is_empty() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Err(e) = self.tx.try_send(batch) {
            error!("persist: failed to enqueue batch: {e}");
        }
        #[cfg(target_arch = "wasm32")]
        {
            let mut store = self.store.0.lock().unwrap();
            for (key, value) in batch {
                if let Err(e) = store.set_string(&key, &value) {
                    error!("persist: {key}: {e}");
                }
            }
        }
    }

    /// Flush the worker's queue and join it before the process exits.
    ///
    /// `close` lets the worker drain queued batches before `recv` errors, so the
    /// FIFO-final batch is written; `block_on` waits for that drain. Native only.
    #[cfg(not(target_arch = "wasm32"))]
    fn shutdown(&mut self) {
        self.tx.close();
        if let Some(task) = self.task.take() {
            block_on(task);
        }
    }
}

/// Create the [`Persister`], spawning the background writer on native.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_persister(pkv: Pkv) -> Persister {
    let (tx, rx) = async_channel::unbounded::<Vec<(String, String)>>();
    let task = IoTaskPool::get().spawn(async move {
        while let Ok(batch) = rx.recv().await {
            let mut store = pkv.0.lock().unwrap();
            for (key, value) in batch {
                if let Err(e) = store.set_string(&key, &value) {
                    error!("persist worker: {key}: {e}");
                }
            }
        }
    });
    Persister {
        tx,
        task: Some(task),
    }
}

/// Create the [`Persister`]; on wasm it writes inline (no worker).
#[cfg(target_arch = "wasm32")]
fn spawn_persister(pkv: Pkv) -> Persister {
    Persister { store: pkv }
}

/// Spawn the background disk writer, sharing the populated store.
fn setup_persister(pkv: Res<Pkv>, mut cmds: Commands) {
    cmds.insert_resource(spawn_persister(pkv.clone()));
}

/// Serialize the full persisted app state into a `(key, value)` batch.
///
/// Registry writes dedup against `persisted` (pass a fresh
/// [`PersistedRegistry`](bevy_gantz::storage::PersistedRegistry) to force a
/// complete write); everything else is written each call.
#[allow(clippy::too_many_arguments)]
fn build_state_batch(
    registry: &Registry<Box<dyn node::Node>>,
    persisted: &mut bevy_gantz::storage::PersistedRegistry,
    views: &Views,
    gui_state: &GuiState,
    tab_order: &HeadTabOrder,
    focused: &FocusedHead,
    heads_query: &Query<OpenHeadDataReadOnly<Box<dyn node::Node>>, With<OpenHead>>,
    window: Option<&Window>,
) -> Vec<(String, String)> {
    let mut batch = bevy_gantz::storage::BatchWriter::default();
    // Registry: only newly-seen graphs/commits and any changed name maps.
    bevy_gantz::storage::save_registry_incremental(&mut batch, registry, persisted);
    // Open heads in tab order.
    let heads: Vec<_> = tab_order
        .iter()
        .filter_map(|&entity| {
            heads_query
                .get(entity)
                .ok()
                .map(|data| (**data.head_ref).clone())
        })
        .collect();
    bevy_gantz::storage::save_open_heads(&mut batch, &heads);
    // Focused head.
    if let Some(focused_entity) = **focused {
        if let Ok(data) = heads_query.get(focused_entity) {
            bevy_gantz::storage::save_focused_head(&mut batch, &**data.head_ref);
        }
    }
    // Views (kept current by `persist_camera_and_seed` and `settle_layout`).
    bevy_gantz_egui::storage::save_views(&mut batch, views);
    // GUI state.
    bevy_gantz_egui::storage::save_gui_state(&mut batch, gui_state);
    // Native window size (no-op on web).
    if let Some(window) = window {
        window::save(&mut batch, window);
    }
    batch.take()
}

fn persist_resources(
    registry: Res<Registry<Box<dyn node::Node>>>,
    mut persisted: ResMut<bevy_gantz::storage::PersistedRegistry>,
    views: Res<Views>,
    gui_state: Res<GuiState>,
    mut persister: ResMut<Persister>,
    tab_order: Res<HeadTabOrder>,
    focused: Res<FocusedHead>,
    heads_query: Query<OpenHeadDataReadOnly<Box<dyn node::Node>>, With<OpenHead>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
) {
    let start = web_time::Instant::now();
    let window = primary_window.single().ok();
    let batch = build_state_batch(
        &registry,
        &mut persisted,
        &views,
        &gui_state,
        &tab_order,
        &focused,
        &heads_query,
        window,
    );
    let writes = batch.len();
    // Hand the writes to the worker; the fsync'd writes happen off-thread.
    persister.submit(batch);
    bevy::log::debug!(
        "persisted state ({:?}, {writes} writes, {} graphs, {} commits on disk)",
        start.elapsed(),
        persisted.graphs_len(),
        persisted.commits_len(),
    );
}

/// Debounced event driving egui-memory persistence, on a slower cadence than
/// the registry/views persist so the two don't fsync on the same frame.
#[derive(Message)]
struct PersistEguiMemory;

impl DebouncedEvent for PersistEguiMemory {
    fn debounced(_triggered_by_focus_loss: bool) -> Self {
        Self
    }
}

/// Persist egui memory (widget state) on the slower debounce, so it doesn't
/// fsync on the same frame as the registry/views persist.
fn persist_egui_memory(mut persister: ResMut<Persister>, mut ctxs: EguiContexts) {
    let start = web_time::Instant::now();
    let mut batch = bevy_gantz::storage::BatchWriter::default();
    if let Ok(ctx) = ctxs.ctx_mut() {
        bevy_gantz_egui::storage::save_egui_memory(&mut batch, ctx);
    }
    persister.submit(batch.take());
    bevy::log::debug!("persisted egui memory ({:?})", start.elapsed());
}

/// On exit, write the full current state and drain the worker before quitting.
///
/// A fresh tracker forces a complete registry write as a backstop for any blob
/// optimistically tracked as persisted but not yet drained by the worker; FIFO
/// + `shutdown` guarantee it lands last. egui memory is left to the worker's
/// normal drain (best-effort widget state).
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn flush_on_exit(
    mut exit: MessageReader<AppExit>,
    registry: Res<Registry<Box<dyn node::Node>>>,
    views: Res<Views>,
    gui_state: Res<GuiState>,
    mut persister: ResMut<Persister>,
    tab_order: Res<HeadTabOrder>,
    focused: Res<FocusedHead>,
    heads_query: Query<OpenHeadDataReadOnly<Box<dyn node::Node>>, With<OpenHead>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
) {
    if exit.read().next().is_none() {
        return;
    }
    let mut full = bevy_gantz::storage::PersistedRegistry::default();
    let window = primary_window.single().ok();
    let batch = build_state_batch(
        &registry,
        &mut full,
        &views,
        &gui_state,
        &tab_order,
        &focused,
        &heads_query,
        window,
    );
    persister.submit(batch);
    persister.shutdown();
}

#[cfg(test)]
mod tests {
    const BASE_GANTZ: &[u8] = gantz_base::BYTES;

    #[test]
    fn base_gantz_deserializes() {
        let _export: gantz_egui::export::Export<
            gantz_core::node::graph::Graph<Box<dyn super::node::Node>>,
        > = gantz_egui::export::parse_export(BASE_GANTZ).expect("valid .gantz");
    }
}
