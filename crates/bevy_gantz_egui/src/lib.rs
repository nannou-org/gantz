//! Egui integration for bevy_gantz.
//!
//! This crate provides:
//! - [`GantzEguiPlugin`], the Bevy plugin for the egui-based UI
//! - GUI state resources and observers
//! - The main `update` system for rendering the gantz GUI

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryData;
use bevy_egui::egui;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};
use bevy_gantz::Registry;
use bevy_gantz::head;
use bevy_gantz::vm::EvalEntryEvent;
use bevy_gantz::{CompileConfig, EvalEntryComplete};
use bevy_log as log;
use gantz_ca as ca;
use gantz_egui::{DynResponse, HeadDataMut, ResponseData};
use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};

pub mod base;
pub mod node;
#[cfg(not(target_arch = "wasm32"))]
pub mod pane_window;
pub mod reg;
pub mod storage;
pub mod sugar;
pub mod vm;

pub use node::builtins;
pub use reg::{BuiltinNodes, GraphCache, env, lookup_node, prune_unused, refresh_cache};
pub use sugar::BevySugar;
pub use vm::{EntrypointFn, EntrypointFns};

/// Plugin providing egui-based UI for gantz.
///
/// This plugin:
/// - Initializes the GUI resources such as `GuiState` and `TraceCapture`
/// - Owns the typed side of the registry, [`GraphCache`] and [`BuiltinNodes`],
///   and keeps head VMs in sync with their compile inputs via [`vm::sync`]
/// - Registers the GUI state and response payload observers
/// - Runs the main GUI update system
pub struct GantzEguiPlugin {
    base_immutable: bool,
}

impl Default for GantzEguiPlugin {
    fn default() -> Self {
        Self {
            base_immutable: true,
        }
    }
}

impl GantzEguiPlugin {
    /// Whether base node graphs are immutable in the GUI.
    ///
    /// When `true`, the default, graphs for heads whose branch name appears
    /// in `BaseNames` are shown in view-only mode.
    ///
    /// Set to `false` for developer tools like `update-base` that need to
    /// edit base nodes.
    pub fn base_immutable(mut self, base_immutable: bool) -> Self {
        self.base_immutable = base_immutable;
        self
    }
}

/// The system set containing the per-frame view persistence passes
/// [`persist_camera_and_seed`] and [`settle_layout`] in the `Update`
/// schedule.
///
/// Layers that publish commit views beyond the process, for example the
/// collaborative-session announce, should run `.after(ViewPersistSet)` so a
/// commit rendered this frame has its view seeded before it can be served.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub struct ViewPersistSet;

impl Plugin for GantzEguiPlugin {
    fn build(&self, app: &mut App) {
        // Push the entrypoint providers. See `vm::EntrypointFns`.
        let mut entrypoint_fns = app.world_mut().get_resource_or_init::<vm::EntrypointFns>();
        entrypoint_fns.0.push(Box::new(|get_node, graph| {
            gantz_core::compile::push_pull_entrypoints(get_node, graph)
        }));
        entrypoint_fns.0.push(Box::new(|get_node, graph| {
            node::update_bang::entrypoints(get_node, graph)
        }));
        entrypoint_fns.0.push(Box::new(|get_node, graph| {
            node::tick_bang::entrypoints(get_node, graph)
        }));

        // The core base source. Domain plugins push their own the same way.
        app.world_mut()
            .get_resource_or_init::<base::BaseSources>()
            .0
            .push(base::BaseSource {
                name: "gantz",
                bytes: gantz_base::BYTES,
            });

        // Builtin GUI response payload dispatchers. Head-scoped payloads
        // arrive at the observers below as `ForHead<T>` events. The rest map
        // onto existing event types via custom dispatch fns. Observers that
        // edit a head's working graph commit it before returning, see
        // `head::WorkingGraph`, so no dispatch-side handling is needed.
        app.register_head_response::<gantz_egui::BranchNode>()
            .register_head_response::<gantz_egui::CopyNodes>()
            .register_head_response::<gantz_egui::CutNodes>()
            .register_head_response::<gantz_egui::DuplicateNodes>()
            .register_head_response::<gantz_egui::NestNodes>()
            .register_head_response::<gantz_egui::CreateNode>()
            .register_head_response::<gantz_egui::CreateNestedGraph>()
            .register_head_response::<gantz_egui::InspectEdge>()
            .register_head_response::<gantz_egui::MergeHead>()
            .register_head_response::<gantz_egui::Paste>()
            .register_head_response::<gantz_egui::Redo>()
            .register_head_response::<gantz_egui::Undo>()
            .register_response_with::<gantz_egui::EvalEntry>(dispatch_eval_entry)
            .register_response_with::<gantz_egui::StateWritten>(dispatch_state_written)
            .register_response_with::<gantz_egui::OpenHead>(dispatch_open_head)
            .register_response_with::<gantz_egui::ReplaceHead>(dispatch_replace_head)
            .register_response_with::<gantz_egui::ExportHead>(dispatch_export_head)
            .register_response_with::<gantz_egui::ExportAllNamed>(dispatch_export_all_named)
            .register_response_with::<gantz_egui::ExportStyle>(dispatch_export_style)
            .register_response_with::<gantz_egui::ImportStyle>(dispatch_import_style);

        app.insert_resource(BaseImmutable(self.base_immutable))
            .init_resource::<GraphCache>()
            .init_resource::<BuiltinNodes>()
            .init_resource::<BaseNames>()
            .init_resource::<base::BaseNameSources>()
            .init_resource::<GuiState>()
            .init_resource::<TraceCapture>()
            .init_resource::<PerfVm>()
            .init_resource::<PerfGui>()
            .init_resource::<WindowedPanesRequested>()
            .init_resource::<SettingsTabs>()
            .init_resource::<ExtPanes>()
            .init_resource::<RefExtUis>()
            .init_resource::<EdgeStyles>()
            .init_resource::<AudioHeads>()
            // GUI state observers
            .add_observer(on_head_opened)
            .add_observer(on_head_changed)
            .add_observer(on_head_closed)
            .add_observer(on_branch_created)
            .add_observer(on_head_committed)
            .add_observer(on_head_committed_resync)
            .add_observer(on_branched_head_fork_nested)
            // VM timing observer
            .add_observer(on_eval_entry_complete)
            // Marker refresh dirty flag, see `node::gui_refresh`
            .add_observer(node::gui_refresh::mark_gui_dirty_on_push)
            // GUI response payload observers
            .add_observer(on_create_node)
            .add_observer(on_create_nested_graph)
            .add_observer(on_branch_node)
            .add_observer(on_inspect_edge)
            .add_observer(on_copy_nodes)
            .add_observer(on_cut_nodes)
            .add_observer(on_nest_nodes)
            .add_observer(on_duplicate_nodes)
            .add_observer(on_merge_head)
            .add_observer(on_sync_remote_tip)
            .add_observer(on_resync_refs)
            .add_observer(on_paste)
            .add_observer(on_undo)
            .add_observer(on_redo)
            .add_observer(on_export_head)
            .add_observer(on_export_all_named)
            .add_observer(on_export_style)
            .add_observer(on_import_style)
            .add_observer(on_import_file)
            .add_observer(on_reset_base_graph)
            // Systems. `drive_update_bangs` evaluates head VMs, so it must not
            // observe the gap between a head pointing at a new graph and
            // `vm::sync` reinitializing its VM.
            .add_systems(
                Update,
                (
                    // Recompiles whenever a head's compile inputs change.
                    vm::sync.in_set(bevy_gantz::VmSet),
                    node::update_bang::drive_update_bangs
                        .after(bevy_gantz::VmSet)
                        .in_set(bevy_gantz::EntrypointSet),
                    node::tick_bang::drive_tick_bangs
                        .after(bevy_gantz::VmSet)
                        .in_set(bevy_gantz::EntrypointSet),
                    node::await_::drive_awaits
                        .after(bevy_gantz::VmSet)
                        .in_set(bevy_gantz::EntrypointSet),
                    // Re-pulls gui markers of dirty or recompiled heads. Runs
                    // after the entrypoint drivers so their pushes refresh
                    // markers the same frame.
                    node::gui_refresh::refresh_gui_markers
                        .after(bevy_gantz::VmSet)
                        .after(node::update_bang::drive_update_bangs)
                        .after(node::tick_bang::drive_tick_bangs)
                        .after(node::await_::drive_awaits)
                        .in_set(bevy_gantz::EntrypointSet),
                    persist_camera_and_seed.in_set(ViewPersistSet),
                    // On layout settle, fork a layout-only commit. Runs after
                    // `VmSet` so a graph edit commits first with its baseline
                    // seeded, and after the camera and seed pass so the head's
                    // baseline exists.
                    settle_layout
                        .in_set(ViewPersistSet)
                        .after(bevy_gantz::VmSet)
                        .after(persist_camera_and_seed)
                        .run_if(on_message::<bevy_gantz::debounced_input::DebouncedInputEvent>),
                    poll_import_task,
                    poll_style_import_task,
                ),
            )
            .add_systems(First, clear_ui_providers)
            .add_systems(EguiPrimaryContextPass, update);
    }
}

/// Empty the domain-provided GUI collections so domain providers refill them
/// with fresh snapshots each frame. See [`SettingsTabs`] for the schedule
/// contract.
fn clear_ui_providers(
    mut tabs: ResMut<SettingsTabs>,
    mut ext_panes: ResMut<ExtPanes>,
    mut ref_ext_uis: ResMut<RefExtUis>,
    mut edge_styles: ResMut<EdgeStyles>,
    mut audio_heads: ResMut<AudioHeads>,
) {
    tabs.0.clear();
    ext_panes.0.clear();
    ref_ext_uis.0.clear();
    edge_styles.0.clear();
    audio_heads.0.clear();
}

/// Per-head GUI state component.
///
/// Wraps `gantz_egui::widget::gantz::OpenHeadState` for each open head entity.
#[derive(Component, Default)]
pub struct HeadGuiState(pub gantz_egui::widget::gantz::OpenHeadState);

/// Views for a single head's graphs, keyed by subgraph path.
///
/// Requires the rest of the per-head GUI state so every spawn path gets the
/// full trio. App-side session restore bypasses the [`on_head_opened`]
/// observer. The `OpenHeadViews` query silently skips entities missing any
/// of them.
#[derive(Component, Default, Clone)]
#[require(HeadGuiState, HeadNodeInstances)]
pub struct GraphView(pub gantz_egui::SceneView);

/// Per-head cache of reified node instances for the working graph.
///
/// Reset wholesale on head navigation in `on_head_changed`. Not required for
/// correctness, since each entry's witness check self-heals, but it bounds
/// memory when the graph is replaced outright.
#[derive(Component, Default)]
pub struct HeadNodeInstances(pub gantz_egui::node::NodeInstances);

/// Marker. This open head participates in a live collaborative session.
///
/// The session layer inserts it on join and removes it on leave. While
/// present, the undo and redo handlers mint forward revert commits, see
/// [`gantz_egui::ops::session_undo`], instead of navigating backwards. Peers
/// plan an ancestor tip as up-to-date and drop it, so plain navigation undo
/// never syncs.
#[derive(Component)]
pub struct SessionHead;

/// Captures tracing logs for the TraceView widget.
#[derive(Default, Resource)]
pub struct TraceCapture(pub gantz_egui::widget::trace_view::TraceCapture);

/// Performance capture for VM execution timing.
#[derive(Default, Resource)]
pub struct PerfVm(pub gantz_egui::widget::PerfCapture);

/// Performance capture for GUI frame timing.
#[derive(Default, Resource)]
pub struct PerfGui(pub gantz_egui::widget::PerfCapture);

/// The gantz GUI state, such as open head states.
#[derive(Resource, Default)]
pub struct GuiState(pub gantz_egui::widget::GantzState);

/// The application's value-level [`NodeCodec`][gantz_egui::node::NodeCodec].
/// The seam through which `.gantz` parse and export paths validate and
/// normalize stored nodes.
///
/// The application inserts this alongside [`BuiltinNodes`], typically as
/// `NodeCodecRes(node::codec())`. [`GantzEguiPlugin`]'s import, export and
/// base systems read it.
#[derive(Clone, Copy, Resource)]
pub struct NodeCodecRes(pub gantz_egui::node::NodeCodec);

/// The collaborative-session display state threaded into the Gantz widget.
///
/// Absent unless a collab layer such as `bevy_gantz_collab` inserts and fills
/// it. When present, the Graph Config pane renders its collab row. The
/// Settings > Collab subtab arrives separately, via [`SettingsTabs`].
#[derive(Resource, Default)]
pub struct CollabUi(pub gantz_egui::collab::CollabUiState);

/// Names of base nodes baked into the binary.
///
/// When present, these names are displayed with a `[base]` prefix and
/// cannot be deleted from the Graphs pane.
#[derive(Resource, Default)]
pub struct BaseNames(pub gantz_egui::reg::Names);

/// Whether base node graphs are immutable in the GUI.
///
/// Inserted by [`GantzEguiPlugin`] based on its `base_immutable` setting.
#[derive(Resource)]
pub struct BaseImmutable(pub bool);

/// The panes the widget currently has popped out into windows, mirrored from
/// [`GantzResponse::windowed_panes`][gantz_egui::widget::gantz::GantzResponse]
/// each frame by [`update`]. A native host reads this to create and destroy
/// its OS windows. Present on all targets, but only read natively.
#[derive(Resource, Default)]
pub struct WindowedPanesRequested(pub Vec<gantz_egui::widget::WindowedPane>);

/// Marker a native host inserts to signal that it owns the pop-out windows, so
/// [`update`] builds the widget in
/// [`PaneWindowMode::HostNative`][gantz_egui::widget::PaneWindowMode] and stops
/// drawing `egui::Window`s itself.
#[derive(Resource)]
pub struct HostNativePaneWindows;

/// In-flight import file dialog task.
#[derive(Resource)]
pub struct ImportTask(bevy_tasks::Task<Option<Vec<u8>>>);

/// In-flight style import file dialog task.
#[derive(Resource)]
pub struct StyleImportTask(bevy_tasks::Task<Option<Vec<u8>>>);

/// Settings subtabs contributed by domains. See
/// [`SettingsTab`][gantz_egui::widget::SettingsTab].
///
/// The provider contract: cleared each frame in `First`. Domain provider
/// systems refill it in `PreUpdate`, so the tabs exist before both GUI
/// render paths. Consumed by [`update`] and the native pop-out window render
/// pass `pane_window::render_windowed_panes`. A provider typically pushes a
/// fresh snapshot of its domain's config and status, and applies the change
/// payloads its tab emitted on the previous frame.
#[derive(Default, Resource)]
pub struct SettingsTabs(pub Vec<Box<dyn gantz_egui::widget::SettingsTab + Send + Sync>>);

/// Top-level panes contributed by domains. See
/// [`ExtPane`][gantz_egui::widget::ExtPane].
///
/// Same contract as [`SettingsTabs`]. A provider typically pushes a fresh
/// snapshot of its domain's per-head data.
#[derive(Default, Resource)]
pub struct ExtPanes(pub Vec<Box<dyn gantz_egui::widget::ExtPane + Send + Sync>>);

/// `NamedRef` inspector extensions contributed by domains. See
/// [`RefExtUi`][gantz_egui::node::RefExtUi].
///
/// Same contract as [`SettingsTabs`]. Extensions take `&self`, so consumers
/// borrow the resource shared.
#[derive(Default, Resource)]
pub struct RefExtUis(pub Vec<Box<dyn gantz_egui::node::RefExtUi + Send + Sync>>);

/// Graph-scene edge stylers contributed by domains. See
/// [`EdgeStyle`][gantz_egui::widget::EdgeStyle].
///
/// Same contract as [`SettingsTabs`]. Stylers take `&self`, so consumers
/// borrow the resource shared. A provider typically pushes a styler holding
/// precomputed per-head classifications, since the erased GUI registry hides
/// concrete node types.
#[derive(Default, Resource)]
pub struct EdgeStyles(pub Vec<Box<dyn gantz_egui::widget::EdgeStyle + Send + Sync>>);

/// The open heads whose domain runtime derives an audio output this frame.
/// Their tabs show the speaker that toggles the head's mute. See
/// [`OpenHeadState::muted`][gantz_egui::widget::gantz::OpenHeadState::muted].
///
/// Same contract as [`SettingsTabs`].
#[derive(Default, Resource)]
pub struct AudioHeads(pub HashSet<ca::Head>);

/// A GUI response payload targeting an open-head entity.
///
/// The `update` system drains the payloads emitted during the GUI pass and
/// dispatches each via [`ResponseDispatchers`]. Head-scoped payloads arrive
/// as this event. Observers take the mutable per-head queries that the GUI
/// system itself cannot, due to ECS borrow rules.
#[derive(EntityEvent)]
pub struct ForHead<T: Send + Sync + 'static> {
    /// The open-head entity the payload targets.
    #[event_target]
    pub head: Entity,
    /// The payload emitted from the GUI.
    pub data: T,
}

/// Event emitted when the user requests exporting a head.
#[derive(Event)]
pub struct ExportHeadEvent {
    /// The head entity to export.
    pub head: Entity,
}

/// Event emitted when the user requests exporting all named graphs.
#[derive(Event)]
pub struct ExportAllNamedEvent;

/// Event emitted when the user requests exporting the GUI style.
#[derive(Event)]
pub struct ExportStyleEvent;

/// Event emitted when the user requests importing a GUI style.
#[derive(Event)]
pub struct ImportStyleEvent;

/// Event emitted when a `.gantz` file is dropped onto a pane.
#[derive(Event)]
pub struct ImportFileEvent {
    /// The raw bytes of the dropped file.
    pub bytes: Vec<u8>,
    /// Whether to open the root head after merging. Set for a GraphScene drop.
    pub open_head: bool,
}

/// Event emitted when a base graph should be reset to its original state.
#[derive(Event)]
pub struct ResetBaseGraphEvent(pub ca::Name);

/// The signature of dispatch fns stored in [`ResponseDispatchers`].
///
/// The `Option<Entity>` is the open-head entity resolved from the payload's
/// head tag. It is `None` for app-level payloads.
pub type DispatchFn = fn(Option<Entity>, DynResponse, &mut Commands);

/// `TypeId`-keyed dispatchers turning the dynamic GUI response payloads into
/// typed events. Register payload types via [`RegisterResponseExt`].
#[derive(Default, Resource)]
pub struct ResponseDispatchers(pub HashMap<TypeId, DispatchFn>);

/// App extension for registering GUI response payload handlers.
///
/// Nodes declared in independent plugins receive custom payloads emitted from
/// their UI this way. The node's returned [`gantz_egui::NodeUiResponse`] and
/// friends carry an `emit` helper. Register the payload type here and add an
/// observer for [`ForHead<T>`]:
///
/// ```ignore
/// app.register_head_response::<MyPayload>()
///     .add_observer(|t: On<ForHead<MyPayload>>, /* any system params */| { .. });
/// ```
pub trait RegisterResponseExt {
    /// Dispatch payloads of type `T` as [`ForHead<T>`] events targeting the
    /// emitting head's entity. Pair with an observer for `On<ForHead<T>>`.
    fn register_head_response<T: ResponseData>(&mut self) -> &mut Self;

    /// Dispatch payloads of type `T` with a custom fn. For example, to map onto
    /// an existing event type or to handle payloads with no associated head.
    fn register_response_with<T: ResponseData>(&mut self, f: DispatchFn) -> &mut Self;
}

impl RegisterResponseExt for App {
    fn register_head_response<T: ResponseData>(&mut self) -> &mut Self {
        self.register_response_with::<T>(dispatch_for_head::<T>)
    }

    fn register_response_with<T: ResponseData>(&mut self, f: DispatchFn) -> &mut Self {
        self.world_mut()
            .get_resource_or_init::<ResponseDispatchers>()
            .0
            .insert(TypeId::of::<T>(), f);
        self
    }
}

/// Bundled query data for open heads. Core data plus views.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct OpenHeadViews {
    pub core: head::OpenHeadData,
    pub view: &'static mut GraphView,
    pub instances: &'static mut HeadNodeInstances,
}

/// The [`gantz_egui::HeadAccess`] implementation for Bevy ECS.
///
/// Wraps the Bevy queries and resources so the gantz_egui widget can access
/// head data without knowing about Bevy's ECS.
pub struct HeadAccess<'q, 'w, 's> {
    /// Heads in tab order, pre-collected.
    heads: Vec<ca::Head>,
    /// Map from head to entity for lookup.
    head_to_entity: HashMap<ca::Head, Entity>,
    /// Query for accessing head data and views mutably.
    query: &'q mut Query<'w, 's, OpenHeadViews, With<head::OpenHead>>,
    /// The VMs keyed by entity.
    vms: &'q mut head::HeadVms,
}

impl<'q, 'w, 's> HeadAccess<'q, 'w, 's> {
    pub fn new(
        tab_order: &head::HeadTabOrder,
        query: &'q mut Query<'w, 's, OpenHeadViews, With<head::OpenHead>>,
        vms: &'q mut head::HeadVms,
    ) -> Self {
        let mut heads = Vec::new();
        let mut head_to_entity = HashMap::new();

        for &entity in tab_order.iter() {
            match query.get(entity) {
                Ok(data) => {
                    let head: ca::Head = (**data.core.head_ref).clone();
                    heads.push(head.clone());
                    head_to_entity.insert(head, entity);
                }
                // A tab-order entity outside the query means a spawn path
                // missed one of the per-head GUI components. Dropping it here
                // would hide the head from the UI while its VM keeps running,
                // so log it loudly instead.
                Err(e) => {
                    log::error!("open head {entity} missing from the views query: {e}");
                }
            }
        }

        Self {
            heads,
            head_to_entity,
            query,
            vms,
        }
    }

    /// Iterate over all heads mutably, for post-GUI updates.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = OpenHeadViewsItem<'_, '_>> + '_ {
        self.query.iter_mut()
    }
}

impl gantz_egui::HeadAccess for HeadAccess<'_, '_, '_> {
    fn heads(&self) -> &[ca::Head] {
        &self.heads
    }

    fn with_head_mut<R>(
        &mut self,
        head: &ca::Head,
        f: impl FnOnce(HeadDataMut<'_>) -> R,
    ) -> Option<R> {
        let entity = *self.head_to_entity.get(head)?;
        let mut data = self.query.get_mut(entity).ok()?;
        let vm = self.vms.get_mut(&entity)?;
        Some(f(HeadDataMut {
            graph: &mut *data.core.working_graph,
            view: &mut *data.view,
            vm,
            instances: &mut data.instances.0,
        }))
    }

    fn module(&self, head: &ca::Head) -> Option<&gantz_core::vm::Compiled> {
        let entity = *self.head_to_entity.get(head)?;
        let data = self.query.get(entity).ok()?;
        data.core.module.compiled.as_ref()
    }

    fn compile_error(&self, head: &ca::Head) -> Option<&str> {
        let entity = *self.head_to_entity.get(head)?;
        let data = self.query.get(entity).ok()?;
        data.core.module.error.as_deref()
    }

    fn diagnostics(&self, head: &ca::Head) -> &[gantz_core::Diagnostic] {
        let Some(&entity) = self.head_to_entity.get(head) else {
            return &[];
        };
        let Ok(data) = self.query.get(entity) else {
            return &[];
        };
        &data.core.diagnostics.0
    }
}

impl Deref for HeadGuiState {
    type Target = gantz_egui::widget::gantz::OpenHeadState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for HeadGuiState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for GuiState {
    type Target = gantz_egui::widget::GantzState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GuiState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for GraphView {
    type Target = gantz_egui::SceneView;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GraphView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

// Observers

/// Record VM execution timing from `EvalEntryComplete` events into `PerfVm`
/// for the performance widget.
fn on_eval_entry_complete(trigger: On<EvalEntryComplete>, mut perf_vm: ResMut<PerfVm>) {
    perf_vm.0.record(trigger.event().duration);
}

/// Initialize GUI state entry and components for an opened head.
///
/// Loads the view from the registry's view section and inserts the per-head
/// GUI components.
pub fn on_head_opened(
    trigger: On<head::OpenedEvent>,
    registry: Res<Registry>,
    mut gui_state: ResMut<GuiState>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    gui_state.open_heads.entry(event.head.clone()).or_default();

    let head_view = registry
        .head_commit_ca(&event.head)
        .and_then(|ca| gantz_egui::section::view(&registry, &ca))
        .unwrap_or_default();

    cmds.entity(event.entity)
        .insert(HeadGuiState::default())
        .insert(GraphView(head_view))
        .insert(HeadNodeInstances::default());
}

/// Migrate GUI state for a changed head and reset its components.
///
/// Loads the view for the new head and resets the per-head GUI components.
pub fn on_head_changed(
    trigger: On<head::ChangedEvent>,
    mut registry: ResMut<Registry>,
    mut gui_state: ResMut<GuiState>,
    mut ctxs: EguiContexts,
    graph_views: Query<&GraphView>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    gui_state.migrate_head(&event.old_head, &event.new_head, false);
    if let Ok(ctx) = ctxs.ctx_mut() {
        gantz_egui::widget::update_graph_pane_head(ctx, &event.old_head, &event.new_head);
    }

    // Load the view for the new head's commit. The target commit may have no
    // stored view. For example, a wire-fetched merge tip whose view blob went
    // missing, or a commit minted by a headless peer. Then carry the live
    // layout forward through the navigation node-identity matching. An empty
    // layout would make the scene auto-layout destructively. Seed the store
    // so the commit has a view before any same-frame session announce.
    let stored = event
        .new_commit
        .and_then(|ca| gantz_egui::section::view(&registry, &ca));
    let mut head_view = match stored {
        Some(view) => view,
        None => {
            let live = graph_views.get(event.entity).ok();
            let carried = match (live, event.old_commit, event.new_commit) {
                (Some(live), _, _) if event.same_graph => live.0.clone(),
                (Some(live), Some(old), Some(new)) => {
                    let node_count = registry
                        .commit_graph_ref(&new)
                        .map(|g| g.node_count())
                        .unwrap_or(0);
                    bevy_gantz::vm::navigation_matching(&registry, old, new)
                        .map(|m| gantz_egui::ops::carry_layout(&live.0, &m, node_count))
                        .unwrap_or_default()
                }
                _ => Default::default(),
            };
            if let Some(new) = event.new_commit {
                if !carried.layout.is_empty() {
                    gantz_egui::section::set_view(&mut registry.0, new, &carried);
                }
            }
            carried
        }
    };

    // Camera is excluded from undo. On a same-graph navigation, such as a
    // layout undo, keep the live camera rather than the target commit's
    // stored camera.
    if event.same_graph {
        if let Ok(current) = graph_views.get(event.entity) {
            head_view.camera = current.camera;
        }
    }

    cmds.entity(event.entity)
        .insert(HeadGuiState::default())
        .insert(GraphView(head_view))
        .insert(HeadNodeInstances::default());
}

/// Remove GUI state for closed head.
pub fn on_head_closed(trigger: On<head::ClosedEvent>, mut gui_state: ResMut<GuiState>) {
    let head = &trigger.event().head;
    gui_state.open_heads.remove(head);
    gui_state.redo_stacks.remove(head);
    gui_state.undo_cursors.remove(head);
}

/// Migrate GUI state for branch creation.
pub fn on_branch_created(
    trigger: On<head::BranchedHeadEvent>,
    mut gui_state: ResMut<GuiState>,
    mut ctxs: EguiContexts,
) {
    let event = trigger.event();
    gui_state.migrate_head(&event.old_head, &event.new_head, false);
    if let Ok(ctx) = ctxs.ctx_mut() {
        gantz_egui::widget::update_graph_pane_head(ctx, &event.old_head, &event.new_head);
    }
}

/// Handle a graph commit by updating egui state.
///
/// Also clears the redo stack, since a new edit invalidates the redo history.
pub fn on_head_committed(
    trigger: On<head::CommittedEvent>,
    mut gui_state: ResMut<GuiState>,
    mut ctxs: EguiContexts,
) {
    let event = trigger.event();
    gui_state.migrate_head(&event.old_head, &event.new_head, true);
    if let Ok(ctx) = ctxs.ctx_mut() {
        gantz_egui::widget::update_graph_pane_head(ctx, &event.old_head, &event.new_head);
    }
}

/// On any head commit, propagate the change to referrers. Bring all
/// sync-enabled `NamedRef`s up to date and refresh any open head whose commit
/// moved. For example, a nested graph edit propagates up to its open parent.
pub fn on_head_committed_resync(
    _trigger: On<head::CommittedEvent>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
) {
    let moves = gantz_egui::sync::resync(&mut registry, bevy_gantz::reg::timestamp());
    refresh_cache(&registry, &mut cache, &codec.0);
    refresh_moved_heads(&moves, &mut registry, &mut heads);
}

/// On a fork, give the fork independent nested children. Copy the original's
/// `parent:*` subtree to the fork and rewrite its references, then refresh
/// the open fork.
pub fn on_branched_head_fork_nested(
    trigger: On<head::BranchedHeadEvent>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let (ca::Head::Branch(old), ca::Head::Branch(new)) = (&event.old_head, &event.new_head) else {
        return;
    };
    let ts = bevy_gantz::reg::timestamp();
    // Give the fork independent nested children. When the fork renamed a
    // nested graph to a root name, repoint the parent's references to it.
    let mut moves = gantz_egui::sync::fork_nested(&mut registry, ts, old, new);
    moves.extend(gantz_egui::sync::promote_nested(
        &mut registry,
        ts,
        old,
        new,
    ));
    refresh_cache(&registry, &mut cache, &codec.0);
    refresh_moved_heads(&moves, &mut registry, &mut heads);
}

/// Carry moved graphs' views forward to their new commits, and refresh any open
/// head whose commit moved. Reload its working graph to the new version and
/// clear its compile memo so `vm::sync` recompiles it. No re-commit is needed,
/// since the registry already holds this graph.
fn refresh_moved_heads(
    moves: &[gantz_egui::sync::Moved],
    registry: &mut Registry,
    heads: &mut Query<head::OpenHeadData, With<head::OpenHead>>,
) {
    if moves.is_empty() {
        return;
    }
    for m in moves {
        if gantz_egui::section::view(registry, &m.new_commit).is_none() {
            if let Some(gv) = gantz_egui::section::view(registry, &m.old_commit) {
                gantz_egui::section::set_view(&mut registry.0, m.new_commit, &gv);
            }
        }
    }
    for mut data in heads.iter_mut() {
        let ca::Head::Branch(name) = data.head_ref.0.clone() else {
            continue;
        };
        let Some(m) = moves.iter().find(|m| m.name == name) else {
            continue;
        };
        let Some(graph) = registry
            .commits()
            .get(&m.new_commit)
            .and_then(|c| registry.graph(&c.graph))
        else {
            continue;
        };
        data.working_graph.0 = graph.clone();
        *data.compiled_inputs = bevy_gantz::vm::CompiledInputs::default();
    }
}

/// Handle create node payloads.
pub fn on_create_node(
    trigger: On<ForHead<gantz_egui::CreateNode>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
    mut views_query: Query<&mut GraphView, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!("CreateNode: head not found for entity {:?}", event.head);
        return;
    };
    let editing = match &**data.head_ref {
        ca::Head::Branch(name) => Some(name.to_string()),
        ca::Head::Commit(_) => None,
    };
    let Ok(mut views) = views_query.get_mut(event.head) else {
        log::error!("CreateNode: views not found for entity {:?}", event.head);
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("CreateNode: VM not found for entity {:?}", event.head);
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**data.head_ref) else {
        log::error!("CreateNode: GUI state not found for head");
        return;
    };

    let node_reg = env(&registry, &cache, &builtins, &codec);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    gantz_egui::ops::create_node(
        node_reg.registry,
        editing.as_deref(),
        &codec.0,
        &get_node,
        |node_type| node_reg.create_node(node_type),
        &mut data.working_graph,
        &mut views,
        head_state,
        vm,
        event.data.clone(),
    );
    // See `head::WorkingGraph`.
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle branch node payloads.
///
/// Creates a new commit with the same graph content, a new timestamp and the
/// original as parent. Inserts the new name and replaces the NamedRef node in
/// the working graph.
pub fn on_branch_node(
    trigger: On<ForHead<gantz_egui::BranchNode>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!("BranchNode: head not found for entity {:?}", event.head);
        return;
    };
    gantz_egui::ops::branch_node(
        &mut registry,
        bevy_gantz::reg::timestamp(),
        &mut data.working_graph,
        event.data.new_name.clone(),
        event.data.ca,
        &event.data.path,
    );
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle create nested graph payloads.
///
/// Commits a fresh empty graph named `<parent>:<n>`, where `<parent>` is the
/// head's branch name. Inserts a synced `NamedRef` to it in the head's
/// working graph. Requires the head to be named.
pub fn on_create_nested_graph(
    trigger: On<ForHead<gantz_egui::CreateNestedGraph>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
    mut views_query: Query<&mut GraphView, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!(
            "CreateNestedGraph: head not found for entity {:?}",
            event.head
        );
        return;
    };
    let ca::Head::Branch(parent) = data.head_ref.0.clone() else {
        log::warn!("CreateNestedGraph: name the graph before adding a nested graph");
        return;
    };
    let Ok(mut views) = views_query.get_mut(event.head) else {
        log::error!(
            "CreateNestedGraph: views not found for entity {:?}",
            event.head
        );
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**data.head_ref) else {
        log::error!("CreateNestedGraph: GUI state not found for head");
        return;
    };
    gantz_egui::ops::create_nested_graph(
        &mut registry,
        bevy_gantz::reg::timestamp(),
        &mut data.working_graph,
        &mut views,
        head_state,
        event.data.pos,
        &parent,
    );
    // The fresh nested graph must be reified before its NamedRef resolves.
    refresh_cache(&registry, &mut cache, &codec.0);
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle inspect edge payloads.
pub fn on_inspect_edge(
    trigger: On<ForHead<gantz_egui::InspectEdge>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
    mut views_query: Query<&mut GraphView, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!("InspectEdge: head not found for entity {:?}", event.head);
        return;
    };
    let Ok(mut views) = views_query.get_mut(event.head) else {
        log::error!("InspectEdge: views not found for entity {:?}", event.head);
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("InspectEdge: VM not found for entity {:?}", event.head);
        return;
    };

    let node_reg = env(&registry, &cache, &builtins, &codec);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    gantz_egui::ops::inspect_edge(
        &codec.0,
        &get_node,
        || node_reg.create_node("inspect"),
        &mut data.working_graph,
        &mut views,
        vm,
        event.data.clone(),
    );
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle copy selection payloads.
///
/// Serializes the selected nodes and their registry dependencies to a
/// `.gantz` document and writes the result to the system clipboard via
/// [`bevy_egui::EguiClipboard`].
pub fn on_copy_nodes(
    trigger: On<ForHead<gantz_egui::CopyNodes>>,
    registry: Res<Registry>,
    codec: Res<NodeCodecRes>,
    mut clipboard: ResMut<bevy_egui::EguiClipboard>,
    mut heads: Query<(&mut head::WorkingGraph, &GraphView), With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok((wg, gv)) = heads.get_mut(event.head) else {
        log::error!("CopySelection: head not found for entity {:?}", event.head);
        return;
    };

    let text = gantz_egui::ops::copy_nodes(&registry, &wg, gv, &event.data.0, &codec.0);
    if let Some(text) = text {
        clipboard.set_text(&text);
    }
}

/// Handle paste selection payloads.
///
/// Resolves the clipboard text via [`bevy_egui::EguiClipboard`] when the
/// payload does not carry it. Parses it into a [`gantz_egui::export::Copied`],
/// merges registry dependencies, adds the subgraph, maps positions, and
/// selects the pasted nodes.
pub fn on_paste(
    trigger: On<ForHead<gantz_egui::Paste>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut clipboard: ResMut<bevy_egui::EguiClipboard>,
    mut cmds: Commands,
    mut heads: Query<
        (&mut head::HeadRef, &mut head::WorkingGraph, &mut GraphView),
        With<head::OpenHead>,
    >,
) {
    let event = trigger.event();
    let Some(text) = event.data.text.clone().or_else(|| clipboard.get_text()) else {
        return;
    };
    let Ok((mut head_ref, mut wg, mut gv)) = heads.get_mut(event.head) else {
        log::error!("PasteSelection: head not found for entity {:?}", event.head);
        return;
    };
    let editing = match &**head_ref {
        ca::Head::Branch(name) => Some(name.to_string()),
        ca::Head::Commit(_) => None,
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("PasteSelection: GUI state not found for head");
        return;
    };

    let pasted = gantz_egui::ops::paste(
        &mut registry,
        editing.as_deref(),
        &mut wg,
        &mut gv,
        head_state,
        &text,
        &event.data.pos,
        &codec.0,
    );

    // Re-register the full root graph so pasted nodes get their state
    // initialized with the correct nested hashmap structure. Idempotent for
    // existing nodes. Registration reifies the graph transiently. The paste
    // may have merged new dependency graphs into the registry, so refresh
    // the cache first.
    if pasted {
        refresh_cache(&registry, &mut cache, &codec.0);
        if let Some(vm) = vms.get_mut(&event.head) {
            let node_reg = env(&registry, &cache, &builtins, &codec);
            let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
            match codec.0.reify_graph(&wg.0) {
                Ok(g) => gantz_core::graph::register(&get_node, &g, &[], vm),
                Err(e) => log::error!("Paste: cannot re-register the pasted graph: {e}"),
            }
        }
    }

    bevy_gantz::commit_working_graph(&mut registry, &mut cmds, event.head, &mut head_ref.0, &wg.0);
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle cut payloads. Copy the selection to the clipboard, then remove it.
pub fn on_cut_nodes(
    trigger: On<ForHead<gantz_egui::CutNodes>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut clipboard: ResMut<bevy_egui::EguiClipboard>,
    mut cmds: Commands,
    mut heads: Query<
        (
            &mut head::HeadRef,
            &mut head::WorkingGraph,
            &mut GraphView,
            &mut HeadNodeInstances,
        ),
        With<head::OpenHead>,
    >,
) {
    let event = trigger.event();
    let Ok((mut head_ref, mut wg, mut gv, mut instances)) = heads.get_mut(event.head) else {
        log::error!("CutNodes: head not found for entity {:?}", event.head);
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("CutNodes: GUI state not found for head");
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("CutNodes: VM not found for head");
        return;
    };

    let text = gantz_egui::ops::cut_nodes(
        &registry,
        &mut wg,
        vm,
        &mut gv,
        &mut head_state.scene.interaction.selection,
        &mut instances.0,
        &event.data.0,
        &codec.0,
    );
    if let Some(text) = text {
        clipboard.set_text(&text);
    }

    bevy_gantz::commit_working_graph(&mut registry, &mut cmds, event.head, &mut head_ref.0, &wg.0);
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle nest payloads. Cut the selected nodes into a new nested graph node.
pub fn on_nest_nodes(
    trigger: On<ForHead<gantz_egui::NestNodes>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<
        (
            &mut head::HeadRef,
            &mut head::WorkingGraph,
            &mut GraphView,
            &mut HeadNodeInstances,
        ),
        With<head::OpenHead>,
    >,
) {
    let event = trigger.event();
    let Ok((mut head_ref, mut wg, mut gv, mut instances)) = heads.get_mut(event.head) else {
        log::error!("NestNodes: head not found for entity {:?}", event.head);
        return;
    };
    let ca::Head::Branch(parent) = head_ref.0.clone() else {
        log::warn!("NestNodes: name the graph before nesting nodes");
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("NestNodes: GUI state not found for head");
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("NestNodes: VM not found for head");
        return;
    };

    gantz_egui::ops::nest_nodes(
        &mut registry,
        bevy_gantz::reg::timestamp(),
        &mut wg,
        vm,
        &mut gv,
        head_state,
        &mut instances.0,
        &event.data.0,
        &parent,
    );

    // The fresh nested graph must be reified before its NamedRef resolves.
    refresh_cache(&registry, &mut cache, &codec.0);
    bevy_gantz::commit_working_graph(&mut registry, &mut cmds, event.head, &mut head_ref.0, &wg.0);
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Handle duplicate payloads. Copy the selection, then paste it at an offset.
pub fn on_duplicate_nodes(
    trigger: On<ForHead<gantz_egui::DuplicateNodes>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<
        (&mut head::HeadRef, &mut head::WorkingGraph, &mut GraphView),
        With<head::OpenHead>,
    >,
) {
    let event = trigger.event();
    let Ok((mut head_ref, mut wg, mut gv)) = heads.get_mut(event.head) else {
        log::error!("DuplicateNodes: head not found for entity {:?}", event.head);
        return;
    };
    let editing = match &**head_ref {
        ca::Head::Branch(name) => Some(name.to_string()),
        ca::Head::Commit(_) => None,
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("DuplicateNodes: GUI state not found for head");
        return;
    };

    let duplicated = gantz_egui::ops::duplicate_nodes(
        &mut registry,
        editing.as_deref(),
        &mut wg,
        &mut gv,
        head_state,
        &event.data.0,
        &codec.0,
    );

    // Re-register the full root graph so the new nodes get their state
    // initialized. Idempotent for existing nodes. Registration reifies the
    // graph transiently.
    if duplicated {
        refresh_cache(&registry, &mut cache, &codec.0);
        if let Some(vm) = vms.get_mut(&event.head) {
            let node_reg = env(&registry, &cache, &builtins, &codec);
            let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
            match codec.0.reify_graph(&wg.0) {
                Ok(g) => gantz_core::graph::register(&get_node, &g, &[], vm),
                Err(e) => log::error!("DuplicateNodes: cannot re-register the graph: {e}"),
            }
        }
    }

    bevy_gantz::commit_working_graph(&mut registry, &mut cmds, event.head, &mut head_ref.0, &wg.0);
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// Seed `commit`'s stored view, ignoring empty layouts. An empty layout reads
/// as never laid out to the scene, which then auto-layouts destructively, so
/// it must never become a commit's baseline.
pub fn seed_view(registry: &mut Registry, commit: ca::CommitAddr, view: gantz_egui::SceneView) {
    if !view.layout.is_empty() {
        gantz_egui::section::set_view(&mut registry.0, commit, &view);
    }
}

/// Finish a locally-minted merge commit. The op has already committed with
/// both parents, for a local merge or session convergence. Seed the minted
/// commit's view from the migrated live layout so it exists before any
/// same-frame session announce. A viewless tip auto-layouts on adopting
/// peers. Re-register the root graph so merged-in nodes get their state
/// initialized. Then fire the committed machinery. That is GUI-state
/// migration, redo-stack clear, NamedRef resync and `vm::sync` recompile.
#[allow(clippy::too_many_arguments)] // A cohesive tail over ECS-owned data.
fn finish_merge_commit(
    new_commit: ca::CommitAddr,
    committed: head::CommittedEvent,
    graph: &ca::DataGraph,
    live_view: &gantz_egui::SceneView,
    registry: &mut Registry,
    (cache, builtins, codec): (&GraphCache, &BuiltinNodes, &NodeCodecRes),
    vm: &mut steel::steel_vm::engine::Engine,
    cmds: &mut Commands,
) {
    seed_view(registry, new_commit, live_view.clone());
    let node_reg = env(registry, cache, builtins, codec);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    match codec.0.reify_graph(graph) {
        Ok(g) => gantz_core::graph::register(&get_node, &g, &[], vm),
        Err(e) => log::error!("merge finish: cannot re-register the merged graph: {e}"),
    }
    cmds.trigger(committed);
}

/// Handle merge payloads. Merge the named source branch into the head.
///
/// A fast-forward navigates the head to the source's tip and reloads the
/// working graph, VM and views. A true merge is applied to the working graph
/// and committed with two parents by [`gantz_egui::ops::merge_head`] itself.
/// So this triggers [`head::CommittedEvent`] directly. `commit_working_graph`
/// would see the already-committed graph and skip the event.
pub fn on_merge_head(
    trigger: On<ForHead<gantz_egui::MergeHead>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<
        (&mut head::HeadRef, &mut head::WorkingGraph, &mut GraphView),
        With<head::OpenHead>,
    >,
) {
    let event = trigger.event();
    let Ok((mut head_ref, mut wg, mut gv)) = heads.get_mut(event.head) else {
        log::error!("MergeHead: head not found for entity {:?}", event.head);
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("MergeHead: GUI state not found for head");
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("MergeHead: VM not found for head");
        return;
    };

    let old_head = head_ref.0.clone();
    let outcome = gantz_egui::ops::merge_head(
        &mut registry,
        bevy_gantz::reg::timestamp(),
        &mut head_ref.0,
        &mut wg,
        vm,
        &mut gv,
        &mut head_state.scene.interaction.selection,
        &event.data.source,
        event.data.resolutions,
        event.data.auto_resolve,
    );
    // The merge may have committed new graphs.
    refresh_cache(&registry, &mut cache, &codec.0);

    match outcome {
        gantz_egui::ops::MergeHeadOutcome::FastForward(target) => {
            navigate_head(&mut cmds, event.head, &old_head, target);
        }
        gantz_egui::ops::MergeHeadOutcome::Merged { new_commit, .. } => {
            log::debug!(
                "Merged '{}' -> {}",
                event.data.source,
                new_commit.display_short()
            );
            let committed = head::CommittedEvent {
                entity: event.head,
                old_head,
                new_head: head_ref.0.clone(),
            };
            finish_merge_commit(
                new_commit,
                committed,
                &wg.0,
                &gv.0,
                &mut registry,
                (&cache, &builtins, &codec),
                vm,
                &mut cmds,
            );
        }
        gantz_egui::ops::MergeHeadOutcome::Refused(reasons) => {
            // The UI disables conflicted and blocked candidates.
            log::warn!(
                "MergeHead: refused to merge '{}': {}",
                event.data.source,
                reasons.join("; ")
            );
        }
        gantz_egui::ops::MergeHeadOutcome::Noop => (),
    }
}

/// Bring an open head up to date with a remote session tip. See
/// [`gantz_egui::ops::sync_remote_tip`].
///
/// The collaborative-session layer triggers it as `ForHead<SyncRemoteTip>`
/// once the remote tip's closure has been fetched, validated and applied to
/// the registry. Not a GUI payload. Nothing emits it from widgets.
#[derive(Clone, Copy, Debug)]
pub struct SyncRemoteTip {
    /// The remote tip to converge with.
    pub remote: ca::CommitAddr,
    /// The session's fixed conflict-resolution policy.
    pub resolutions: ca::merge::Resolutions,
    /// Adopt an unrelated remote tip instead of surfacing it. Set when the
    /// head points at the join flow's placeholder graph.
    pub adopt_unrelated: bool,
}

/// Handle [`SyncRemoteTip`]. The session analogue of [`on_merge_head`].
pub fn on_sync_remote_tip(
    trigger: On<ForHead<SyncRemoteTip>>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    builtins: Res<BuiltinNodes>,
    codec: Res<NodeCodecRes>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<
        (&mut head::HeadRef, &mut head::WorkingGraph, &mut GraphView),
        With<head::OpenHead>,
    >,
) {
    let event = trigger.event();
    let Ok((mut head_ref, mut wg, mut gv)) = heads.get_mut(event.head) else {
        log::error!("SyncRemoteTip: head not found for entity {:?}", event.head);
        return;
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("SyncRemoteTip: GUI state not found for head");
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("SyncRemoteTip: VM not found for head");
        return;
    };

    let old_head = head_ref.0.clone();
    let outcome = gantz_egui::ops::sync_remote_tip(
        &mut registry,
        &mut head_ref.0,
        &mut wg,
        vm,
        &mut gv,
        &mut head_state.scene.interaction.selection,
        event.data.remote,
        event.data.resolutions,
        event.data.adopt_unrelated,
    );
    // The sync may have minted a merge commit and graph.
    refresh_cache(&registry, &mut cache, &codec.0);

    match outcome {
        gantz_egui::ops::SyncTipOutcome::UpToDate => (),
        gantz_egui::ops::SyncTipOutcome::Moved(target) => {
            // A remote edit invalidates local redo just like a local one. The
            // Merged arm clears via CommittedEvent. A stale session-redo would
            // otherwise mint a whole-graph revert that clobbers the peers'
            // newer work.
            gui_state.redo_stacks.remove(&old_head);
            gui_state.undo_cursors.remove(&old_head);
            navigate_head(&mut cmds, event.head, &old_head, target);
        }
        gantz_egui::ops::SyncTipOutcome::Merged {
            new_commit,
            conflicts,
            ..
        } => {
            if conflicts > 0 {
                log::info!("session merge auto-resolved {conflicts} conflict(s)");
            }
            log::debug!("session merge -> {}", new_commit.display_short());
            let committed = head::CommittedEvent {
                entity: event.head,
                old_head,
                new_head: head_ref.0.clone(),
            };
            finish_merge_commit(
                new_commit,
                committed,
                &wg.0,
                &gv.0,
                &mut registry,
                (&cache, &builtins, &codec),
                vm,
                &mut cmds,
            );
        }
        gantz_egui::ops::SyncTipOutcome::Blocked(reasons) => {
            log::warn!("session sync blocked: {}", reasons.join("; "));
        }
        gantz_egui::ops::SyncTipOutcome::Unrelated => {
            log::warn!("session sync: remote tip shares no history with the local graph");
        }
    }
}

/// Request a reference resync outside the usual committed flow. Bring
/// sync-enabled `NamedRef`s up to date and refresh open heads whose commits
/// moved. The collaborative-session layer triggers it after it moves scoped
/// names that no open head points at, such as fast-forwards of nested or
/// referenced graphs.
#[derive(Debug, Event)]
pub struct ResyncRefsEvent;

/// Handle [`ResyncRefsEvent`]. The same pass as [`on_head_committed_resync`].
pub fn on_resync_refs(
    _trigger: On<ResyncRefsEvent>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut heads: Query<head::OpenHeadData, With<head::OpenHead>>,
) {
    let moves = gantz_egui::sync::resync(&mut registry, bevy_gantz::reg::timestamp());
    refresh_cache(&registry, &mut cache, &codec.0);
    refresh_moved_heads(&moves, &mut registry, &mut heads);
}

/// Handle undo payloads. Move the head back to its parent commit.
///
/// A session head, see [`SessionHead`], instead mints a forward revert
/// commit so the undo propagates to peers like any other edit.
pub fn on_undo(
    trigger: On<ForHead<gantz_egui::Undo>>,
    mut registry: ResMut<Registry>,
    mut gui_state: ResMut<GuiState>,
    heads: Query<(&head::HeadRef, Has<SessionHead>), With<head::OpenHead>>,
    graph_views: Query<&GraphView>,
    mut cmds: Commands,
) {
    let entity = trigger.event().head;
    let Ok((head_ref, in_session)) = heads.get(entity) else {
        log::error!("Undo: head not found for entity {entity:?}");
        return;
    };
    let head = (**head_ref).clone();
    let gui = &mut gui_state.0;
    let target = if in_session {
        let live_camera = graph_views.get(entity).ok().map(|gv| gv.0.camera);
        gantz_egui::ops::session_undo(
            &mut registry.0,
            &mut gui.redo_stacks,
            &mut gui.undo_cursors,
            bevy_gantz::reg::timestamp(),
            &head,
            live_camera,
        )
    } else {
        gantz_egui::ops::undo(&registry.0, &mut gui.redo_stacks, &head)
    };
    if let Some(target) = target {
        navigate_head(&mut cmds, entity, &head, target);
    }
}

/// Handle redo payloads. Move the head forward to an undone commit.
///
/// A session head, see [`SessionHead`], instead mints a forward revert
/// commit restoring the undone position, mirroring [`on_undo`].
pub fn on_redo(
    trigger: On<ForHead<gantz_egui::Redo>>,
    mut registry: ResMut<Registry>,
    mut gui_state: ResMut<GuiState>,
    heads: Query<(&head::HeadRef, Has<SessionHead>), With<head::OpenHead>>,
    graph_views: Query<&GraphView>,
    mut cmds: Commands,
) {
    let entity = trigger.event().head;
    let Ok((head_ref, in_session)) = heads.get(entity) else {
        log::error!("Redo: head not found for entity {entity:?}");
        return;
    };
    let head = (**head_ref).clone();
    let gui = &mut gui_state.0;
    let target = if in_session {
        let live_camera = graph_views.get(entity).ok().map(|gv| gv.0.camera);
        gantz_egui::ops::session_redo(
            &mut registry.0,
            &mut gui.redo_stacks,
            &mut gui.undo_cursors,
            bevy_gantz::reg::timestamp(),
            &head,
            live_camera,
        )
    } else {
        gantz_egui::ops::redo(&mut gui.redo_stacks, &head)
    };
    if let Some(target) = target {
        navigate_head(&mut cmds, entity, &head, target);
    }
}

/// Handle export head events.
///
/// Exports the head's graph with transitive dependencies and views to a
/// `.gantz` file chosen via an `rfd` file dialog. The export is serialized as
/// `.gantz` text by [`gantz_egui::export`].
pub fn on_export_head(
    trigger: On<ExportHeadEvent>,
    registry: Res<Registry>,
    codec: Res<NodeCodecRes>,
    heads: Query<&head::HeadRef, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(head_ref) = heads.get(event.head) else {
        log::error!("ExportHead: head not found for entity {:?}", event.head);
        return;
    };
    let head: &ca::Head = &**head_ref;

    let text = match gantz_egui::export::export_heads_sexpr(&registry, [head], &codec.0) {
        Ok(s) => s,
        Err(e) => {
            log::error!("ExportHead: failed to serialize: {e}");
            return;
        }
    };

    let default_name = gantz_egui::export::default_filename(&head);

    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Export Graph")
        .set_file_name(&default_name)
        .add_filter("Gantz Export", &[gantz_egui::export::FILE_EXTENSION]);
    bevy_tasks::AsyncComputeTaskPool::get()
        .spawn(async move {
            if let Some(handle) = dialog.save_file().await {
                if let Err(e) = handle.write(text.as_bytes()).await {
                    log::error!("ExportHead: failed to write: {e}");
                } else {
                    log::info!("Exported graph to {}", handle.file_name());
                }
            }
        })
        .detach();
}

/// Handle export-all-named events.
///
/// Exports every named graph with transitive dependencies and views to a
/// single `.gantz` file chosen via an `rfd` file dialog.
pub fn on_export_all_named(
    _trigger: On<ExportAllNamedEvent>,
    registry: Res<Registry>,
    codec: Res<NodeCodecRes>,
) {
    let named_heads: Vec<ca::Head> = registry
        .heads()
        .map(|(name, _)| ca::Head::Branch(name.clone()))
        .collect();

    if named_heads.is_empty() {
        log::info!("ExportAllNamed: no named graphs to export");
        return;
    }

    let text = match gantz_egui::export::export_heads_sexpr(&registry, named_heads.iter(), &codec.0)
    {
        Ok(s) => s,
        Err(e) => {
            log::error!("ExportAllNamed: failed to serialize: {e}");
            return;
        }
    };

    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Export All Named Graphs")
        .set_file_name(&format!("gantz.{}", gantz_egui::export::FILE_EXTENSION))
        .add_filter("Gantz Export", &[gantz_egui::export::FILE_EXTENSION]);
    bevy_tasks::AsyncComputeTaskPool::get()
        .spawn(async move {
            if let Some(handle) = dialog.save_file().await {
                if let Err(e) = handle.write(text.as_bytes()).await {
                    log::error!("ExportAllNamed: failed to write: {e}");
                } else {
                    log::info!("Exported all named graphs to {}", handle.file_name());
                }
            }
        })
        .detach();
}

/// Handle style export events.
///
/// Writes the GUI's [`gantz_egui::StyleConfig`] to a `.ron` file chosen via an
/// `rfd` file dialog.
pub fn on_export_style(_trigger: On<ExportStyleEvent>, gui_state: Res<GuiState>) {
    let text = match gantz_egui::style::to_ron(&gui_state.style) {
        Ok(s) => s,
        Err(e) => {
            log::error!("ExportStyle: failed to serialize: {e}");
            return;
        }
    };

    let ext = gantz_egui::style::FILE_EXTENSION;
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Export Style")
        .set_file_name(&format!("gantz-style.{ext}"))
        .add_filter("Gantz Style", &[ext]);
    bevy_tasks::AsyncComputeTaskPool::get()
        .spawn(async move {
            if let Some(handle) = dialog.save_file().await {
                if let Err(e) = handle.write(text.as_bytes()).await {
                    log::error!("ExportStyle: failed to write: {e}");
                } else {
                    log::info!("Exported style to {}", handle.file_name());
                }
            }
        })
        .detach();
}

/// Handle style import events by opening a file dialog.
///
/// The chosen file's bytes return to the main world via [`StyleImportTask`],
/// polled by `poll_style_import_task`.
pub fn on_import_style(
    _trigger: On<ImportStyleEvent>,
    task: Option<Res<StyleImportTask>>,
    mut cmds: Commands,
) {
    // Only one dialog at a time.
    if task.is_some() {
        return;
    }
    let ext = gantz_egui::style::FILE_EXTENSION;
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Import Style")
        .add_filter("Gantz Style", &[ext]);
    let task = bevy_tasks::AsyncComputeTaskPool::get().spawn(async move {
        let handle = dialog.pick_file().await?;
        Some(handle.read().await)
    });
    cmds.insert_resource(StyleImportTask(task));
}

/// Handle import file events for dropped `.gantz` files.
///
/// Deserializes the export, optionally computes root names, merges into the
/// registry, and opens the unique root head if requested.
pub fn on_import_file(
    trigger: On<ImportFileEvent>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    let export = match gantz_egui::export::parse_export(&event.bytes, &codec.0) {
        Ok(e) => e,
        Err(e) => {
            log::error!("ImportFile: {e}");
            return;
        }
    };

    // The root name is needed only to open a head. Compute it before the
    // merge consumes the export.
    let root_name = if event.open_head {
        gantz_egui::export::unique_root_name(&export)
    } else {
        None
    };

    let report = registry.merge(export);
    refresh_cache(&registry, &mut cache, &codec.0);
    log::info!(
        "Imported: {} names added, {} replaced",
        report.heads_added.len(),
        report.heads_replaced.len(),
    );

    if let Some(name) = root_name {
        cmds.trigger(head::OpenEvent(ca::Head::Branch(name)));
    }
}

/// Reset a base graph to its original state by re-merging from the base export.
pub fn on_reset_base_graph(
    trigger: On<ResetBaseGraphEvent>,
    sources: Res<base::BaseSources>,
    name_sources: Res<base::BaseNameSources>,
    base_names: Res<BaseNames>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
) {
    let name = &trigger.event().0;
    // Re-parse the source that defined the name, seeded with the loaded base
    // names so the source's cross-source refs resolve. Its own names shadow
    // the seed via the in-document lookup. Every source parses at
    // BASE_TIMESTAMP, so addresses match the startup parse.
    let Some(source) = name_sources
        .0
        .get(&name.to_string())
        .and_then(|source_name| sources.0.iter().find(|s| s.name == *source_name))
    else {
        log::warn!("ResetBaseGraph: no base source recorded for '{name}'");
        return;
    };
    let seed = base::seed_graph_addrs(&base_names.0, &registry);
    let export: gantz_ca::Registry = match gantz_egui::export::parse_export_seeded_at(
        source.bytes,
        crate::base::BASE_TIMESTAMP,
        &seed,
        &codec.0,
    ) {
        Ok(e) => e,
        Err(e) => {
            log::error!(
                "ResetBaseGraph: failed to parse base source `{}`: {e}",
                source.name
            );
            return;
        }
    };
    // Extract just the content reachable from the target name. All parents
    // are included, so merge ancestry survives a reset.
    if let Some(base_commit_ca) = export.head(name) {
        let live = ca::closure_from(&export, [base_commit_ca]);
        let mut subset = ca::export(&export, &live);
        subset.set_head(name.clone(), base_commit_ca);
        registry.merge(subset);
        refresh_cache(&registry, &mut cache, &codec.0);
        log::info!("Reset base graph '{name}' to original version");
    } else {
        log::warn!(
            "ResetBaseGraph: name '{name}' not found in base source `{}`",
            source.name,
        );
    }
}

// Systems

/// Keep each open head's camera current and seed its commit's layout baseline.
///
/// Runs every frame. Each commit's stored `layout` stays frozen as an undo
/// baseline. It is written exactly once, when a commit first has a populated
/// live layout, and is never overwritten in place. Node-position changes only
/// ever produce a new commit, see [`settle_layout`]. The camera is excluded
/// from undo, so it tracks the live view in place every frame.
pub fn persist_camera_and_seed(
    mut registry: ResMut<Registry>,
    heads: Query<(&head::HeadRef, &GraphView), With<head::OpenHead>>,
) {
    for (head_ref, head_view) in heads.iter() {
        let Some(commit_addr) = registry.head_commit_ca(&**head_ref) else {
            continue;
        };
        match gantz_egui::section::view(&registry, &commit_addr) {
            // Layout frozen as the undo baseline. Only the camera tracks live.
            Some(mut view) => {
                if view.camera != head_view.camera {
                    view.camera = head_view.camera;
                    gantz_egui::section::set_view(&mut registry.0, commit_addr, &view);
                }
            }
            // Seed the baseline once the scene has laid the graph out. Guarding
            // on a non-empty layout avoids capturing an empty pre-layout frame.
            None if !head_view.layout.is_empty() => {
                gantz_egui::section::set_view(&mut registry.0, commit_addr, head_view);
            }
            None => {}
        }
    }
}

/// On layout settle, signalled by [`DebouncedInputEvent`], fork a layout-only
/// commit for any open head whose node positions changed since its frozen
/// baseline view.
///
/// Mirrors the graph-commit path's GUI bookkeeping. It migrates per-head GUI
/// state, clears redo and migrates the graph pane. It does not fire
/// [`head::CommittedEvent`]. The graph content is unchanged, so there is
/// nothing to resync, and firing it would churn every sync-enabled referrer's
/// history for a pure layout move. `GraphView` is left untouched. It already
/// holds the settled layout, which matches the new commit's seeded baseline.
///
/// [`DebouncedInputEvent`]: bevy_gantz::debounced_input::DebouncedInputEvent
pub fn settle_layout(
    mut registry: ResMut<Registry>,
    mut gui_state: ResMut<GuiState>,
    mut ctxs: EguiContexts,
    mut heads: Query<(Entity, &mut head::HeadRef, &GraphView), With<head::OpenHead>>,
    mut cmds: Commands,
) {
    for (entity, mut head_ref, head_view) in heads.iter_mut() {
        let old_head = (**head_ref).clone();
        let Some(new_commit) = gantz_egui::ops::commit_layout(
            &mut registry,
            bevy_gantz::reg::timestamp(),
            &mut head_ref,
            head_view,
        ) else {
            continue;
        };
        // Freeze the new commit's layout baseline this frame, before any
        // debounce-gated save/export reads the registry's view section.
        gantz_egui::section::set_view(&mut registry.0, new_commit, head_view);
        // Clear redo, since a new commit invalidates it, and migrate GUI state.
        let new_head = (**head_ref).clone();
        gui_state.migrate_head(&old_head, &new_head, true);
        if let Ok(ctx) = ctxs.ctx_mut() {
            gantz_egui::widget::update_graph_pane_head(ctx, &old_head, &new_head);
        }
        // Not a `CommittedEvent`, so no resync cascade. Layers that mirror
        // commits elsewhere, such as collab sessions syncing node positions,
        // still need to hear about it.
        cmds.trigger(LayoutCommittedEvent { entity, new_commit });
    }
}

/// Emitted after a settled node-move produced a layout-only commit, with the
/// same graph and a new commit. Unlike [`head::CommittedEvent`] this triggers
/// no resync or recompile machinery. It exists for layers that follow the
/// commit chain, such as collaborative sessions syncing node positions.
#[derive(Debug, Event)]
pub struct LayoutCommittedEvent {
    pub entity: Entity,
    pub new_commit: ca::CommitAddr,
}

/// Poll the in-flight import file dialog task.
///
/// When the task completes with file bytes, triggers [`ImportFileEvent`].
/// The resource is removed regardless of whether a file was selected.
fn poll_import_task(task: Option<ResMut<ImportTask>>, mut cmds: Commands) {
    let Some(mut task) = task else { return };
    if let Some(result) = bevy_tasks::futures::check_ready(&mut task.0) {
        cmds.remove_resource::<ImportTask>();
        if let Some(bytes) = result {
            cmds.trigger(ImportFileEvent {
                bytes,
                open_head: true,
            });
        }
    }
}

/// Poll the in-flight style import file dialog task.
///
/// When the task completes with file bytes, replaces the GUI's style config.
/// `gantz_egui::style::apply` picks it up on the next pass. The resource is
/// removed regardless of whether a file was selected.
fn poll_style_import_task(
    task: Option<ResMut<StyleImportTask>>,
    mut gui_state: ResMut<GuiState>,
    mut cmds: Commands,
) {
    let Some(mut task) = task else { return };
    let Some(result) = bevy_tasks::futures::check_ready(&mut task.0) else {
        return;
    };
    cmds.remove_resource::<StyleImportTask>();
    let Some(bytes) = result else { return };
    match std::str::from_utf8(&bytes).map(gantz_egui::style::from_ron) {
        Ok(Ok(style)) => {
            gui_state.style = style;
            log::info!("Imported style");
        }
        Ok(Err(e)) => log::error!("ImportStyle: failed to parse: {e}"),
        Err(e) => log::error!("ImportStyle: invalid UTF-8: {e}"),
    }
}

/// Update the Gantz GUI and process widget responses.
///
/// This system:
/// - Shows the Gantz widget in an egui CentralPanel
/// - Processes GUI responses such as head open, close and replace
/// - Dispatches dynamic response payloads via [`ResponseDispatchers`]
/// - Uses TraceCapture for tracing and PerfVm and PerfGui for performance
///   capture
pub fn update(
    trace_capture: Res<TraceCapture>,
    mut perf_vm: ResMut<PerfVm>,
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
    import_task: Option<Res<ImportTask>>,
    (
        codec,
        base_names,
        base_immutable,
        mut compile_config,
        mut change_validation,
        mut settings_tabs,
        mut ext_panes,
        ref_ext_uis,
        (edge_styles, audio_heads),
        mut requested,
        host_native,
        export_paths,
        base_sources,
        mut base_name_sources,
        collab_ui,
        clipboard,
    ): (
        Res<NodeCodecRes>,
        Res<BaseNames>,
        Res<BaseImmutable>,
        ResMut<CompileConfig>,
        ResMut<bevy_gantz::ValidateCommitted>,
        ResMut<SettingsTabs>,
        ResMut<ExtPanes>,
        Res<RefExtUis>,
        (Res<EdgeStyles>, Res<AudioHeads>),
        ResMut<WindowedPanesRequested>,
        Option<Res<HostNativePaneWindows>>,
        Option<Res<base::ExportPaths>>,
        Res<base::BaseSources>,
        ResMut<base::BaseNameSources>,
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
                .perf_captures(&mut perf_vm.0, &mut perf_gui.0)
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
        &base_names,
        &mut compile_config,
        &mut change_validation,
        &base_sources,
        &mut base_name_sources,
        import_task.as_deref(),
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
/// whether docked or windowed. Handles focus, graph open, close, replace and
/// new, branch, file drops, demo and description edits, resets, compile config,
/// change tracking, import, in-place head commits, and dynamic payload dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_gantz_response(
    response: &mut gantz_egui::widget::gantz::GantzResponse,
    tab_order: &head::HeadTabOrder,
    focused: &mut head::FocusedHead,
    heads_query: &mut Query<OpenHeadViews, With<head::OpenHead>>,
    registry: &mut Registry,
    cache: &mut GraphCache,
    codec: &NodeCodecRes,
    base_names: &BaseNames,
    compile_config: &mut CompileConfig,
    change_validation: &mut bevy_gantz::ValidateCommitted,
    base_sources: &base::BaseSources,
    base_name_sources: &mut base::BaseNameSources,
    import_task: Option<&ImportTask>,
    head_to_entity: &HashMap<ca::Head, Entity>,
    dispatchers: &ResponseDispatchers,
    cmds: &mut Commands,
) {
    if let Some(&entity) = tab_order.get(response.focused_head) {
        **focused = Some(entity);
    }

    if let Some(name) = response.graph_name_removed() {
        // Detach any open heads that reference the removed name.
        for mut data in heads_query.iter_mut() {
            if let ca::Head::Branch(head_name) = &**data.core.head_ref {
                if *head_name == name {
                    let commit_ca = registry.head_commit_ca(&data.core.head_ref).unwrap();
                    **data.core.head_ref = ca::Head::Commit(commit_ca);
                }
            }
        }
        registry.remove_head(&name);
    }

    // A single click replaces the focused head with the selected one.
    if let Some(new_head) = response.graph_replaced() {
        cmds.trigger(head::ReplaceEvent(new_head.clone()));
    }

    // Open the head as a new tab, or focus it if already open.
    if let Some(new_head) = response.graph_opened() {
        cmds.trigger(head::OpenEvent(new_head.clone()));
    }

    if let Some(h) = response.graph_closed() {
        cmds.trigger(head::CloseEvent(h.clone()));
    }

    if response.new_graph() {
        let new_head = registry.init_head(bevy_gantz::timestamp());
        cmds.trigger(head::OpenEvent(new_head));
    }

    // Handle closed heads from tab close buttons.
    for closed_head in &response.closed_heads {
        cmds.trigger(head::CloseEvent(closed_head.clone()));
    }

    // Handle new branch created from tab double-click.
    if let Some((original_head, new_name)) = response.new_branch() {
        cmds.trigger(head::BranchHeadEvent {
            original: original_head.clone(),
            new_name: new_name.clone(),
        });
    }

    // Handle egui-level file drops.
    for drop in &response.file_drops {
        let open_head = drop.target == gantz_egui::widget::gantz::FileDropTarget::GraphScene;
        cmds.trigger(ImportFileEvent {
            bytes: drop.bytes.clone(),
            open_head,
        });
    }

    // Handle a demo graph association change. Keyed by the head's branch name
    // so the association survives later edits, which mint a new commit.
    if let Some((ca::Head::Branch(graph_name), demo_val)) = &response.demo_changed {
        match demo_val {
            Some(demo) => {
                gantz_egui::section::set_demo(&mut registry.0, graph_name.clone(), demo.clone());
            }
            None => {
                gantz_egui::section::remove_demo(&mut registry.0, graph_name);
            }
        }
    }

    // Handle a graph description edit, keyed by the graph's name.
    if let Some((ca::Head::Branch(name), description)) = &response.description_changed {
        gantz_egui::section::set_description(&mut registry.0, name.clone(), description.clone());
    }

    // Re-attribute a graph to a different base source. Durable wherever the
    // per-source write-back runs, as in `update-base`. That rewrites every
    // configured file on the next save, which moves the graph between files.
    if let Some((ca::Head::Branch(name), source)) = &response.base_source_changed {
        match base_sources.0.iter().find(|s| s.name == source.as_str()) {
            Some(s) => {
                base_name_sources.0.insert(name.to_string(), s.name);
            }
            None => log::warn!("base source `{source}` is not registered"),
        }
    }

    // Handle a demo reset. Re-merge the base version of the demo graph.
    if let Some(head) = &response.reset_base_graph {
        if let ca::Head::Branch(name) = head {
            cmds.trigger(ResetBaseGraphEvent(name.clone()));
        }
    }

    // Handle reset all demos. Re-merge every `demo-*` base graph.
    if response.reset_all_demos {
        for name in base_names.0.keys() {
            if name.to_string().starts_with("demo-") {
                cmds.trigger(ResetBaseGraphEvent(name.clone()));
            }
        }
    }

    // Handle a compile config change. The recompile happens next frame in
    // `vm::sync`, which compares each head's compile inputs by value.
    if let Some(cfg) = response.compile_config {
        if compile_config.0 != cfg {
            compile_config.0 = cfg;
        }
    }

    // Toggle change-tracking validation, a debugging aid.
    if let Some(enabled) = response.validate_change_tracking {
        change_validation.0 = enabled;
    }

    // Handle the import button. Open a file dialog if none is in flight.
    if response.import() && import_task.is_none() {
        let ext = gantz_egui::export::FILE_EXTENSION;
        let dialog = rfd::AsyncFileDialog::new()
            .set_title("Import")
            .add_filter("Gantz Export", &[ext]);
        let task = bevy_tasks::AsyncComputeTaskPool::get().spawn(async move {
            let handle = dialog.pick_file().await?;
            Some(handle.read().await)
        });
        cmds.insert_resource(ImportTask(task));
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

/// Downcast a dispatched payload to its concrete type.
///
/// Dispatchers are keyed by the payload's `TypeId`, so the downcast cannot
/// fail for a correctly registered dispatcher. Public so external plugins
/// such as `bevy_gantz_collab` can write custom [`DispatchFn`]s.
pub fn downcast_payload<T: ResponseData>(payload: DynResponse) -> T {
    payload
        .downcast::<T>()
        .expect("dispatcher registered for this payload type")
}

/// Dispatch a head-scoped payload as a [`ForHead`] event.
fn dispatch_for_head<T: ResponseData>(
    entity: Option<Entity>,
    payload: DynResponse,
    cmds: &mut Commands,
) {
    let Some(head) = entity else {
        log::error!(
            "response payload `{}` has no open-head entity",
            payload.type_name()
        );
        return;
    };
    let data = downcast_payload::<T>(payload);
    cmds.trigger(ForHead { head, data });
}

/// Dispatch a [`gantz_egui::EvalEntry`] payload as an [`EvalEntryEvent`].
fn dispatch_eval_entry(entity: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let Some(head) = entity else {
        log::error!("EvalEntry payload has no open-head entity");
        return;
    };
    let gantz_egui::EvalEntry(entrypoint) = downcast_payload(payload);
    cmds.trigger(EvalEntryEvent {
        head,
        entrypoint,
        // A user-driven push fires "now".
        time: None,
    });
}

/// Drop a [`gantz_egui::StateWritten`] payload. Recorded VM-state writes
/// exist for the collaborative-session layer, which overrides this
/// registration to broadcast them. Last registration wins. Without it they
/// are local-only by design, not unhandled.
fn dispatch_state_written(_: Option<Entity>, _payload: DynResponse, _cmds: &mut Commands) {}

/// Dispatch a [`gantz_egui::OpenHead`] payload as a [`head::OpenEvent`].
fn dispatch_open_head(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::OpenHead(target) = downcast_payload(payload);
    cmds.trigger(head::OpenEvent(target));
}

/// Dispatch a [`gantz_egui::ReplaceHead`] payload as a [`head::ReplaceEvent`].
/// This navigates the focused tab to the target head in place, for example
/// when entering a nested graph.
fn dispatch_replace_head(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::ReplaceHead(target) = downcast_payload(payload);
    cmds.trigger(head::ReplaceEvent(target));
}

/// Dispatch a [`gantz_egui::ExportHead`] payload as an [`ExportHeadEvent`].
fn dispatch_export_head(entity: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let Some(head) = entity else {
        log::error!("ExportHead payload has no open-head entity");
        return;
    };
    let gantz_egui::ExportHead = downcast_payload(payload);
    cmds.trigger(ExportHeadEvent { head });
}

/// Dispatch a [`gantz_egui::ExportAllNamed`] payload as an [`ExportAllNamedEvent`].
fn dispatch_export_all_named(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::ExportAllNamed = downcast_payload(payload);
    cmds.trigger(ExportAllNamedEvent);
}

/// Dispatch a [`gantz_egui::ExportStyle`] payload as an [`ExportStyleEvent`].
fn dispatch_export_style(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::ExportStyle = downcast_payload(payload);
    cmds.trigger(ExportStyleEvent);
}

/// Dispatch a [`gantz_egui::ImportStyle`] payload as an [`ImportStyleEvent`].
fn dispatch_import_style(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::ImportStyle = downcast_payload(payload);
    cmds.trigger(ImportStyleEvent);
}

/// Trigger the appropriate event to move a head to a target commit.
///
/// Branch heads use `MoveBranchEvent` for atomic registry and graph updates,
/// which avoids oscillation with `vm::sync`. Commit heads use `ReplaceEvent`.
fn navigate_head(cmds: &mut Commands, entity: Entity, head: &ca::Head, target: ca::CommitAddr) {
    match head {
        ca::Head::Commit(_) => cmds.trigger(head::ReplaceEvent(ca::Head::Commit(target))),
        ca::Head::Branch(name) => cmds.trigger(head::MoveBranchEvent {
            entity,
            name: name.clone(),
            target,
        }),
    }
}
