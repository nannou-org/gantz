//! Off-thread, debounced persistence of app state to the [`Pkv`] store.
//!
//! [`PersistPlugin`] wires up the whole thing. On a debounced input the persist
//! systems serialize the current state into a `(key, value)` batch on the main
//! thread and hand it to a [`Persister`]; on native a background [`IoTaskPool`]
//! worker does the fsync'd writes off the render thread, on wasm they're written
//! inline (no threads, no fsync). On exit the worker is flushed and joined so
//! nothing is lost.
//!
//! This is deliberately self-contained: the goal is to eventually lift it into a
//! reusable "sync-friendly persistence" crate so downstream gantz apps don't
//! reimplement this boilerplate.

use crate::storage::Pkv;
use crate::window;
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, Window};
use bevy_egui::EguiContexts;
use bevy_gantz::{
    FocusedHead, HeadTabOrder, OpenHead, OpenHeadDataReadOnly, Registry,
    debounced_input::{DebouncedEvent, DebouncedInputEvent, DebouncedInputPlugin},
};
use bevy_gantz_egui::{GuiState, Views};

#[cfg(not(target_arch = "wasm32"))]
use bevy::tasks::{IoTaskPool, Task, block_on};

/// The app's node type, as stored in the registry and head entities.
type BoxNode = Box<dyn crate::node::Node>;

/// Registers off-thread, debounced persistence of the app's state.
///
/// Expects the [`Pkv`] resource and a `DebouncedInputPlugin<DebouncedInputEvent>`
/// to already be present (the latter also drives layout settling, so it lives in
/// the app, not here).
pub struct PersistPlugin;

impl Plugin for PersistPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(DebouncedInputPlugin::<PersistEguiMemory>::new(0.3))
            // Spawn the background writer once the store is populated. No ordering
            // vs the load systems is needed: the worker only locks the store when
            // draining a batch, which can't happen until the first `Update`.
            .add_systems(Startup, setup_persister)
            .add_systems(
                Update,
                persist_resources
                    // After `settle_layout` so a layout commit settled this frame
                    // (and its seeded view) is saved in the same pass.
                    .after(bevy_gantz_egui::settle_layout::<BoxNode>)
                    .run_if(on_message::<DebouncedInputEvent>),
            )
            .add_systems(
                Update,
                persist_egui_memory.run_if(on_message::<PersistEguiMemory>),
            );
        // Flush the worker before the process exits (native only; wasm writes
        // inline, and the World is consumed by the runner so this can't run after
        // `run()`).
        #[cfg(not(target_arch = "wasm32"))]
        app.add_systems(Last, flush_on_exit);
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

/// Collect the registry/heads/views/gui/window into a `(key, value)` batch to
/// persist.
///
/// Registry writes dedup against `persisted` (pass a fresh
/// [`PersistedRegistry`](bevy_gantz::storage::PersistedRegistry) to force a
/// complete write); everything else is written each call.
#[allow(clippy::too_many_arguments)]
fn collect_batch_to_persist(
    registry: &Registry<BoxNode>,
    persisted: &mut bevy_gantz::storage::PersistedRegistry,
    views: &Views,
    persisted_views: &mut bevy_gantz_egui::storage::PersistedViews,
    gui_state: &GuiState,
    tab_order: &HeadTabOrder,
    focused: &FocusedHead,
    heads_query: &Query<OpenHeadDataReadOnly<BoxNode>, With<OpenHead>>,
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
    // Views (kept current by `persist_camera_and_seed` and `settle_layout`):
    // write only the per-commit views that changed, for commits that still exist.
    let valid_commits: std::collections::HashSet<_> = registry.commits().keys().copied().collect();
    bevy_gantz_egui::storage::save_views_incremental(
        &mut batch,
        views,
        &valid_commits,
        persisted_views,
    );
    // GUI state.
    bevy_gantz_egui::storage::save_gui_state(&mut batch, gui_state);
    // Native window size (no-op on web).
    if let Some(window) = window {
        window::save(&mut batch, window);
    }
    batch.take()
}

fn persist_resources(
    registry: Res<Registry<BoxNode>>,
    mut persisted: ResMut<bevy_gantz::storage::PersistedRegistry>,
    views: Res<Views>,
    mut persisted_views: ResMut<bevy_gantz_egui::storage::PersistedViews>,
    gui_state: Res<GuiState>,
    mut persister: ResMut<Persister>,
    tab_order: Res<HeadTabOrder>,
    focused: Res<FocusedHead>,
    heads_query: Query<OpenHeadDataReadOnly<BoxNode>, With<OpenHead>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
) {
    let start = web_time::Instant::now();
    let window = primary_window.single().ok();
    let batch = collect_batch_to_persist(
        &registry,
        &mut persisted,
        &views,
        &mut persisted_views,
        &gui_state,
        &tab_order,
        &focused,
        &heads_query,
        window,
    );
    let writes = batch.len();
    // Hand the writes to the worker; the fsync'd writes happen off-thread.
    persister.submit(batch);
    debug!(
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
    debug!("persisted egui memory ({:?})", start.elapsed());
}

/// On exit, write the full current state and drain the worker before quitting.
///
/// Fresh trackers force a complete registry + views write as a backstop for any
/// blob optimistically tracked as persisted but not yet drained by the worker;
/// FIFO + `shutdown` guarantee it lands last. egui memory is left to the
/// worker's normal drain (best-effort widget state).
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn flush_on_exit(
    mut exit: MessageReader<AppExit>,
    registry: Res<Registry<BoxNode>>,
    views: Res<Views>,
    gui_state: Res<GuiState>,
    mut persister: ResMut<Persister>,
    tab_order: Res<HeadTabOrder>,
    focused: Res<FocusedHead>,
    heads_query: Query<OpenHeadDataReadOnly<BoxNode>, With<OpenHead>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
) {
    if exit.read().next().is_none() {
        return;
    }
    let mut full = bevy_gantz::storage::PersistedRegistry::default();
    let mut full_views = bevy_gantz_egui::storage::PersistedViews::default();
    let window = primary_window.single().ok();
    let batch = collect_batch_to_persist(
        &registry,
        &mut full,
        &views,
        &mut full_views,
        &gui_state,
        &tab_order,
        &focused,
        &heads_query,
        window,
    );
    persister.submit(batch);
    persister.shutdown();
}
