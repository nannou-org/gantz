//! Egui integration for bevy_gantz.
//!
//! This crate provides:
//! - [`GantzEguiPlugin`] — Bevy plugin for egui-based UI
//! - GUI state resources and observers
//! - The main `update` system for rendering the gantz GUI

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryData;
use bevy_egui::egui;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};
use bevy_gantz::head;
use bevy_gantz::reg::Registry;
use bevy_gantz::vm::EvalEntryEvent;
use bevy_gantz::{BuiltinNodes, CompileConfig, EvalEntryComplete};
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::Node;
use gantz_core::node::graph::Graph;
pub use gantz_egui::RegistryRef;
use gantz_egui::{DynResponse, HeadDataMut, ResponseData};
use std::any::TypeId;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

pub mod base;
pub mod node;
pub mod storage;

// ----------------------------------------------------------------------------
// Plugin
// ----------------------------------------------------------------------------

/// Plugin providing egui-based UI for gantz.
///
/// Generic over `N`, the node type used in graphs.
///
/// This plugin:
/// - Initializes GUI resources (`GuiState`, `TraceCapture`, `PerfVm`, `PerfGui`)
/// - Registers observers for GUI state management
/// - Registers node creation/inspection observers
/// - Runs the main GUI update system
///
/// **Note:** This plugin requires `GantzPlugin<N>` to be added first.
pub struct GantzEguiPlugin<N> {
    base_immutable: bool,
    _marker: PhantomData<N>,
}

impl<N> Default for GantzEguiPlugin<N> {
    fn default() -> Self {
        Self {
            base_immutable: true,
            _marker: PhantomData,
        }
    }
}

impl<N> GantzEguiPlugin<N> {
    /// Whether base node graphs should be immutable (view-only).
    ///
    /// When `true` (the default), graphs for heads whose branch name
    /// appears in `BaseNames` are shown in immutable mode.
    ///
    /// Set to `false` for developer tools like `update-base` that need
    /// to edit base nodes.
    pub fn base_immutable(mut self, base_immutable: bool) -> Self {
        self.base_immutable = base_immutable;
        self
    }
}

impl<N> Plugin for GantzEguiPlugin<N>
where
    N: Node
        + Clone
        + gantz_ca::CaHash
        + From<gantz_egui::node::NamedRef>
        + gantz_egui::sync::AsNamedRefMut
        + gantz_egui::sync::AsNamedRef
        + gantz_egui::NodeUi
        + node::ToFrameBang
        + serde::Serialize
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static,
{
    fn build(&self, app: &mut App) {
        // Register frame_bang entrypoint provider.
        app.world_mut()
            .resource_mut::<bevy_gantz::EntrypointFns<N>>()
            .0
            .push(Box::new(|get_node, graph| {
                node::frame_bang::entrypoints(get_node, graph)
            }));

        // Builtin GUI response payload dispatchers. Head-scoped payloads
        // arrive at the observers below as `ForHead<T>` events; the rest map
        // onto existing event types via custom dispatch fns. Observers that edit
        // a head's working graph commit it before returning (see the
        // `WorkingGraph` invariant), so no dispatch-side handling is needed.
        app.register_head_response::<gantz_egui::BranchNode>()
            .register_head_response::<gantz_egui::CopyNodes>()
            .register_head_response::<gantz_egui::CreateNode>()
            .register_head_response::<gantz_egui::CreateNestedGraph>()
            .register_head_response::<gantz_egui::InspectEdge>()
            .register_head_response::<gantz_egui::Paste>()
            .register_head_response::<gantz_egui::Redo>()
            .register_head_response::<gantz_egui::Undo>()
            .register_response_with::<gantz_egui::EvalEntry>(dispatch_eval_entry)
            .register_response_with::<gantz_egui::OpenHead>(dispatch_open_head)
            .register_response_with::<gantz_egui::ReplaceHead>(dispatch_replace_head)
            .register_response_with::<gantz_egui::ExportHead>(dispatch_export_head)
            .register_response_with::<gantz_egui::ExportAllNamed>(dispatch_export_all_named);

        app.insert_resource(BaseImmutable(self.base_immutable))
            .init_resource::<BaseNames>()
            .init_resource::<GuiState>()
            .init_resource::<TraceCapture>()
            .init_resource::<PerfVm>()
            .init_resource::<PerfGui>()
            .init_resource::<Views>()
            .init_resource::<Demos>()
            // GUI state observers
            .add_observer(on_head_opened::<N>)
            .add_observer(on_head_changed::<N>)
            .add_observer(on_head_closed)
            .add_observer(on_branch_created)
            .add_observer(on_head_committed)
            .add_observer(on_head_committed_resync::<N>)
            .add_observer(on_branched_head_fork_nested::<N>)
            // VM timing observer
            .add_observer(on_eval_entry_complete)
            // GUI response payload observers
            .add_observer(on_create_node::<N>)
            .add_observer(on_create_nested_graph::<N>)
            .add_observer(on_branch_node::<N>)
            .add_observer(on_inspect_edge::<N>)
            .add_observer(on_copy_nodes::<N>)
            .add_observer(on_paste::<N>)
            .add_observer(on_undo::<N>)
            .add_observer(on_redo)
            .add_observer(on_export_head::<N>)
            .add_observer(on_export_all_named::<N>)
            .add_observer(on_import_file::<N>)
            .add_observer(on_reset_base_graph::<N>)
            // Systems. `drive_frame_bangs` evaluates head VMs, so it must not
            // observe the gap between a head pointing at a new graph and
            // `vm::sync` (re)initializing its VM.
            .add_systems(
                Update,
                (
                    node::frame_bang::drive_frame_bangs::<N>.after(bevy_gantz::VmSet),
                    persist_camera_and_seed::<N>,
                    // On layout settle, fork a layout-only commit. Runs after
                    // `VmSet` (so a graph edit commits first and its baseline is
                    // already seeded - no spurious layout commit) and after the
                    // camera/seed pass (so the head's baseline exists).
                    settle_layout::<N>
                        .after(bevy_gantz::VmSet)
                        .after(persist_camera_and_seed::<N>)
                        .run_if(on_message::<bevy_gantz::debounced_input::DebouncedInputEvent>),
                    poll_import_task,
                ),
            )
            .add_systems(EguiPrimaryContextPass, update::<N>);
    }
}

// ----------------------------------------------------------------------------
// Components
// ----------------------------------------------------------------------------

/// Per-head GUI state component.
///
/// This component wraps `gantz_egui::widget::gantz::OpenHeadState` to store
/// GUI-related state for each open head entity.
#[derive(Component, Default)]
pub struct HeadGuiState(pub gantz_egui::widget::gantz::OpenHeadState);

/// Views for a single head's graphs (keyed by subgraph path).
#[derive(Component, Default, Clone)]
pub struct GraphView(pub egui_graph::View);

// ----------------------------------------------------------------------------
// Resources
// ----------------------------------------------------------------------------

/// Captures tracing logs for the TraceView widget.
#[derive(Default, Resource)]
pub struct TraceCapture(pub gantz_egui::widget::trace_view::TraceCapture);

/// Performance capture for VM execution timing.
#[derive(Default, Resource)]
pub struct PerfVm(pub gantz_egui::widget::PerfCapture);

/// Performance capture for GUI frame timing.
#[derive(Default, Resource)]
pub struct PerfGui(pub gantz_egui::widget::PerfCapture);

/// The gantz GUI state (open head states, etc.).
#[derive(Resource, Default)]
pub struct GuiState(pub gantz_egui::widget::GantzState);

/// Views (layout + camera) for all known commits.
#[derive(Resource, Default)]
pub struct Views(pub HashMap<ca::CommitAddr, egui_graph::View>);

/// Demo graph associations: maps a graph *name* to its associated `demo-*`
/// graph name. Keyed by name (not commit) so the association survives edits,
/// which mint a new commit but keep the name.
#[derive(Resource, Default)]
pub struct Demos(pub HashMap<String, String>);

/// Names of base nodes baked into the binary.
///
/// When present, these names are displayed with a `[base]` prefix and
/// cannot be deleted from the Graphs pane.
#[derive(Resource, Default)]
pub struct BaseNames(pub gantz_ca::registry::Names);

/// Whether base node graphs are immutable (view-only) in the GUI.
///
/// Inserted by [`GantzEguiPlugin`] based on its `base_immutable` setting.
#[derive(Resource)]
pub struct BaseImmutable(pub bool);

/// In-flight import file dialog task.
#[derive(Resource)]
pub struct ImportTask(bevy_tasks::Task<Option<Vec<u8>>>);

// ----------------------------------------------------------------------------
// Events
// ----------------------------------------------------------------------------

/// A GUI response payload targeting an open-head entity.
///
/// The `update` system drains the payloads emitted during the GUI pass and
/// dispatches each via [`ResponseDispatchers`]; head-scoped payloads arrive
/// as this event. Observers take the mutable per-head queries that the GUI
/// system itself cannot (ECS borrow rules).
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

/// Event emitted when a `.gantz` file is dropped onto a pane.
#[derive(Event)]
pub struct ImportFileEvent {
    /// The raw bytes of the dropped file.
    pub bytes: Vec<u8>,
    /// Whether to open the root head after merging (GraphScene target).
    pub open_head: bool,
}

/// Event emitted when a base graph should be reset to its original state.
#[derive(Event)]
pub struct ResetBaseGraphEvent(pub String);

// ----------------------------------------------------------------------------
// Response dispatch
// ----------------------------------------------------------------------------

/// The signature of dispatch fns stored in [`ResponseDispatchers`].
///
/// The `Option<Entity>` is the open-head entity resolved from the payload's
/// head tag (`None` for app-level payloads).
pub type DispatchFn = fn(Option<Entity>, DynResponse, &mut Commands);

/// `TypeId`-keyed dispatchers turning the dynamic GUI response payloads into
/// typed events. Register payload types via [`RegisterResponseExt`].
#[derive(Default, Resource)]
pub struct ResponseDispatchers(pub HashMap<TypeId, DispatchFn>);

/// App extension for registering GUI response payload handlers.
///
/// This is how nodes declared in independent plugins receive custom payloads
/// emitted from their UI (via the `emit` helper on the node's returned
/// [`gantz_egui::NodeUiResponse`] and friends): register the payload type here
/// and add an observer for [`ForHead<T>`]:
///
/// ```ignore
/// app.register_head_response::<MyPayload>()
///     .add_observer(|t: On<ForHead<MyPayload>>, /* any system params */| { .. });
/// ```
pub trait RegisterResponseExt {
    /// Dispatch payloads of type `T` as [`ForHead<T>`] events targeting the
    /// emitting head's entity. Pair with an observer for `On<ForHead<T>>`.
    fn register_head_response<T: ResponseData>(&mut self) -> &mut Self;

    /// Dispatch payloads of type `T` with a custom fn, e.g. to map onto an
    /// existing event type or to handle payloads with no associated head.
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

// ----------------------------------------------------------------------------
// QueryData
// ----------------------------------------------------------------------------

/// Bundled query data for open heads (core data + views).
#[derive(QueryData)]
#[query_data(mutable)]
pub struct OpenHeadViews<N: 'static + Send + Sync> {
    pub core: head::OpenHeadData<N>,
    pub view: &'static mut GraphView,
}

// ----------------------------------------------------------------------------
// HeadAccess adapter
// ----------------------------------------------------------------------------

/// Provides [`gantz_egui::HeadAccess`] implementation for Bevy ECS.
///
/// This struct wraps the necessary Bevy queries and resources to implement
/// the `HeadAccess` trait, allowing the gantz_egui widget to access head data
/// without knowing about Bevy's ECS.
pub struct HeadAccess<'q, 'w, 's, N: 'static + Send + Sync> {
    /// Heads in tab order, pre-collected.
    heads: Vec<ca::Head>,
    /// Map from head to entity for lookup.
    head_to_entity: HashMap<ca::Head, Entity>,
    /// Query for accessing head data + views mutably.
    query: &'q mut Query<'w, 's, OpenHeadViews<N>, With<head::OpenHead>>,
    /// The VMs keyed by entity.
    vms: &'q mut head::HeadVms,
}

impl<'q, 'w, 's, N: 'static + Send + Sync> HeadAccess<'q, 'w, 's, N> {
    pub fn new(
        tab_order: &head::HeadTabOrder,
        query: &'q mut Query<'w, 's, OpenHeadViews<N>, With<head::OpenHead>>,
        vms: &'q mut head::HeadVms,
    ) -> Self {
        // Pre-collect heads in tab order and build entity lookup.
        let mut heads = Vec::new();
        let mut head_to_entity = HashMap::new();

        for &entity in tab_order.iter() {
            if let Ok(data) = query.get(entity) {
                let head: ca::Head = (**data.core.head_ref).clone();
                heads.push(head.clone());
                head_to_entity.insert(head, entity);
            }
        }

        Self {
            heads,
            head_to_entity,
            query,
            vms,
        }
    }

    /// Iterate over all heads mutably (for post-GUI updates).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = OpenHeadViewsItem<'_, '_, N>> + '_ {
        self.query.iter_mut()
    }
}

impl<N: 'static + Send + Sync> gantz_egui::HeadAccess for HeadAccess<'_, '_, '_, N> {
    type Node = N;

    fn heads(&self) -> &[ca::Head] {
        &self.heads
    }

    fn with_head_mut<R>(
        &mut self,
        head: &ca::Head,
        f: impl FnOnce(HeadDataMut<'_, Self::Node>) -> R,
    ) -> Option<R> {
        let entity = *self.head_to_entity.get(head)?;
        let mut data = self.query.get_mut(entity).ok()?;
        let vm = self.vms.get_mut(&entity)?;
        Some(f(HeadDataMut {
            graph: &mut *data.core.working_graph,
            view: &mut *data.view,
            vm,
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

// ----------------------------------------------------------------------------
// Deref impls
// ----------------------------------------------------------------------------

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

impl Deref for Views {
    type Target = HashMap<ca::CommitAddr, egui_graph::View>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Views {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for Demos {
    type Target = HashMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Demos {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for GraphView {
    type Target = egui_graph::View;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GraphView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Construct a `RegistryRef` from Bevy resources.
///
/// This is a convenience function that extracts the underlying references
/// from the Bevy resource wrappers.
pub fn registry_ref<'a, N: 'static + Send + Sync>(
    registry: &'a Registry<N>,
    builtins: &'a BuiltinNodes<N>,
    demos: &'a Demos,
) -> RegistryRef<'a, N> {
    RegistryRef::new(&**registry, &*builtins.0, &demos.0)
}

// ----------------------------------------------------------------------------
// Observers
// ----------------------------------------------------------------------------

/// Record VM execution timing from EvalEntryComplete events.
///
/// This observer receives timing events from `bevy_gantz::vm` and records
/// them to `PerfVm` for the performance widget.
fn on_eval_entry_complete(trigger: On<EvalEntryComplete>, mut perf_vm: ResMut<PerfVm>) {
    perf_vm.0.record(trigger.event().duration);
}

/// Initialize GUI state entry and components for opened head.
///
/// Loads views from the `Views` resource and spawns `GraphView` + `HeadGuiState` components.
pub fn on_head_opened<N: 'static + Send + Sync>(
    trigger: On<head::OpenedEvent>,
    registry: Res<Registry<N>>,
    views: Res<Views>,
    mut gui_state: ResMut<GuiState>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    gui_state.open_heads.entry(event.head.clone()).or_default();

    // Load the view for this head's commit.
    let head_view = registry
        .head_commit_ca(&event.head)
        .and_then(|ca| views.get(ca).cloned())
        .unwrap_or_default();

    cmds.entity(event.entity)
        .insert(HeadGuiState::default())
        .insert(GraphView(head_view));
}

/// Migrate GUI state for changed head and reset components.
///
/// Loads views for the new head and updates `GraphView` + `HeadGuiState` components.
pub fn on_head_changed<N: 'static + Send + Sync>(
    trigger: On<head::ChangedEvent>,
    registry: Res<Registry<N>>,
    views: Res<Views>,
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

    // Load the view for the new head's commit.
    let mut head_view = registry
        .head_commit_ca(&event.new_head)
        .and_then(|ca| views.get(ca).cloned())
        .unwrap_or_default();

    // Camera (`scene_rect`) is excluded from undo: on a same-graph navigation
    // (a layout undo/redo) keep the live camera rather than restoring the target
    // commit's stored camera.
    if event.same_graph {
        if let Ok(current) = graph_views.get(event.entity) {
            head_view.scene_rect = current.scene_rect;
        }
    }

    cmds.entity(event.entity)
        .insert(HeadGuiState::default())
        .insert(GraphView(head_view));
}

/// Remove GUI state for closed head.
pub fn on_head_closed(trigger: On<head::ClosedEvent>, mut gui_state: ResMut<GuiState>) {
    let head = &trigger.event().head;
    gui_state.open_heads.remove(head);
    gui_state.redo_stacks.remove(head);
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

/// Handle graph commit by updating egui state.
///
/// This observer is triggered by `vm::sync` when a graph change is committed.
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

/// On any head commit, propagate the change to referrers: bring all
/// sync-enabled `NamedRef`s up to date and refresh any open head whose commit
/// moved (e.g. a nested graph edit propagating up to its open parent).
pub fn on_head_committed_resync<N>(
    _trigger: On<head::CommittedEvent>,
    mut registry: ResMut<Registry<N>>,
    mut heads: Query<head::OpenHeadData<N>, With<head::OpenHead>>,
    mut views: ResMut<Views>,
) where
    N: 'static + Clone + ca::CaHash + gantz_egui::sync::AsNamedRefMut + Send + Sync,
{
    let moves = gantz_egui::sync::resync(&mut registry, bevy_gantz::reg::timestamp());
    refresh_moved_heads(&moves, &registry, &mut heads, &mut views);
}

/// On a fork (branch from a head), give the fork independent nested children:
/// copy the original's `parent:*` subtree to the fork and rewrite its
/// references, then refresh the open fork.
pub fn on_branched_head_fork_nested<N>(
    trigger: On<head::BranchedHeadEvent>,
    mut registry: ResMut<Registry<N>>,
    mut heads: Query<head::OpenHeadData<N>, With<head::OpenHead>>,
    mut views: ResMut<Views>,
) where
    N: 'static + Clone + ca::CaHash + gantz_egui::sync::AsNamedRefMut + Send + Sync,
{
    let event = trigger.event();
    let (ca::Head::Branch(old), ca::Head::Branch(new)) = (&event.old_head, &event.new_head) else {
        return;
    };
    let ts = bevy_gantz::reg::timestamp();
    // Give the fork independent nested children, then (when the fork renamed a
    // *nested* graph to a root name) repoint the parent's references to it.
    let mut moves = gantz_egui::sync::fork_nested(&mut registry, ts, old, new);
    moves.extend(gantz_egui::sync::promote_nested(
        &mut registry,
        ts,
        old,
        new,
    ));
    refresh_moved_heads(&moves, &registry, &mut heads, &mut views);
}

/// Carry moved graphs' views forward to their new commits, and refresh any open
/// head whose commit moved: reload its working graph to the new version and
/// clear its compile memo so `vm::sync` recompiles it (without re-committing,
/// since the registry already holds this graph).
fn refresh_moved_heads<N>(
    moves: &[gantz_egui::sync::Moved],
    registry: &Registry<N>,
    heads: &mut Query<head::OpenHeadData<N>, With<head::OpenHead>>,
    views: &mut Views,
) where
    N: 'static + Clone + Send + Sync,
{
    if moves.is_empty() {
        return;
    }
    for m in moves {
        if let Some(gv) = views.0.get(&m.old_commit).cloned() {
            views.0.entry(m.new_commit).or_insert(gv);
        }
    }
    for mut data in heads.iter_mut() {
        let ca::Head::Branch(name) = data.head_ref.0.clone() else {
            continue;
        };
        let Some(m) = moves.iter().find(|m| m.name == name) else {
            continue;
        };
        if let Some(graph) = registry.commit_graph_ref(&m.new_commit) {
            data.working_graph.0 = graph.clone();
            *data.compiled_inputs = bevy_gantz::vm::CompiledInputs::default();
        }
    }
}

/// Handle create node payloads.
pub fn on_create_node<N>(
    trigger: On<ForHead<gantz_egui::CreateNode>>,
    mut registry: ResMut<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    demos: Res<Demos>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData<N>, With<head::OpenHead>>,
    mut views_query: Query<&mut GraphView, With<head::OpenHead>>,
) where
    N: 'static
        + Node
        + Clone
        + ca::CaHash
        + From<gantz_egui::node::NamedRef>
        + gantz_egui::sync::AsNamedRef
        + Send
        + Sync,
{
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!("CreateNode: head not found for entity {:?}", event.head);
        return;
    };
    let editing = match &**data.head_ref {
        ca::Head::Branch(name) => Some(name.clone()),
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

    let node_reg = registry_ref(&registry, &builtins, &demos);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    gantz_egui::ops::create_node(
        node_reg.ca_registry(),
        editing.as_deref(),
        &get_node,
        |node_type| node_reg.create_node(node_type),
        &mut data.working_graph,
        &mut views,
        head_state,
        vm,
        event.data.clone(),
    );
    // Uphold the `WorkingGraph` invariant: commit the in-place edit.
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
}

/// Handle branch node payloads.
///
/// Creates a new commit (same graph content, new timestamp, original as parent),
/// inserts the new name, and replaces the NamedRef node in the working graph.
pub fn on_branch_node<N>(
    trigger: On<ForHead<gantz_egui::BranchNode>>,
    mut registry: ResMut<Registry<N>>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData<N>, With<head::OpenHead>>,
) where
    N: 'static + Clone + ca::CaHash + From<gantz_egui::node::NamedRef> + Send + Sync,
{
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
    // Uphold the `WorkingGraph` invariant: commit the in-place edit.
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
}

/// Handle create nested graph payloads.
///
/// Commits a fresh empty graph named `<parent>:<n>` (where `<parent>` is the
/// head's branch name) and inserts a synced `NamedRef` to it in the head's
/// working graph. Requires the head to be named.
pub fn on_create_nested_graph<N>(
    trigger: On<ForHead<gantz_egui::CreateNestedGraph>>,
    mut registry: ResMut<Registry<N>>,
    mut gui_state: ResMut<GuiState>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData<N>, With<head::OpenHead>>,
    mut views_query: Query<&mut GraphView, With<head::OpenHead>>,
) where
    N: 'static + Node + Clone + From<gantz_egui::node::NamedRef> + ca::CaHash + Send + Sync,
{
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
    // Uphold the `WorkingGraph` invariant: commit the in-place edit.
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
}

/// Handle inspect edge payloads.
pub fn on_inspect_edge<N>(
    trigger: On<ForHead<gantz_egui::InspectEdge>>,
    mut registry: ResMut<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    demos: Res<Demos>,
    mut vms: NonSendMut<head::HeadVms>,
    mut cmds: Commands,
    mut heads: Query<head::OpenHeadData<N>, With<head::OpenHead>>,
    mut views_query: Query<&mut GraphView, With<head::OpenHead>>,
) where
    N: 'static + Node + Clone + ca::CaHash + From<gantz_egui::node::NamedRef> + Send + Sync,
{
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

    let node_reg = registry_ref(&registry, &builtins, &demos);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    gantz_egui::ops::inspect_edge(
        &get_node,
        || node_reg.create_node("inspect"),
        &mut data.working_graph,
        &mut views,
        vm,
        event.data.clone(),
    );
    // Uphold the `WorkingGraph` invariant: commit the in-place edit.
    bevy_gantz::commit_working_graph(
        &mut registry,
        &mut cmds,
        event.head,
        &mut data.head_ref.0,
        &data.working_graph.0,
    );
}

/// Handle copy selection payloads.
///
/// Serializes the selected nodes (and their registry dependencies) to a `.gantz`
/// document and writes the result directly to the system clipboard via
/// [`bevy_egui::EguiClipboard`].
pub fn on_copy_nodes<N>(
    trigger: On<ForHead<gantz_egui::CopyNodes>>,
    registry: Res<Registry<N>>,
    views: Res<Views>,
    mut clipboard: ResMut<bevy_egui::EguiClipboard>,
    mut heads: Query<(&mut head::WorkingGraph<N>, &GraphView), With<head::OpenHead>>,
) where
    N: 'static
        + Node
        + Clone
        + serde::Serialize
        + serde::de::DeserializeOwned
        + ca::CaHash
        + Send
        + Sync,
{
    let event = trigger.event();
    let Ok((wg, gv)) = heads.get_mut(event.head) else {
        log::error!("CopySelection: head not found for entity {:?}", event.head);
        return;
    };

    let text = gantz_egui::ops::copy_nodes(&registry, &views, &wg, gv, &event.data.0);
    if let Some(text) = text {
        clipboard.set_text(&text);
    }
}

/// Handle paste selection payloads.
///
/// Resolves the clipboard text (via [`bevy_egui::EguiClipboard`] when the
/// payload doesn't carry it), parses it into a [`gantz_egui::export::Copied`],
/// merges registry dependencies, adds the subgraph, maps positions, and updates
/// the selection to the newly pasted nodes.
pub fn on_paste<N>(
    trigger: On<ForHead<gantz_egui::Paste>>,
    mut registry: ResMut<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut gui_state: ResMut<GuiState>,
    mut views: ResMut<Views>,
    mut demos: ResMut<Demos>,
    mut vms: NonSendMut<head::HeadVms>,
    mut clipboard: ResMut<bevy_egui::EguiClipboard>,
    mut cmds: Commands,
    mut heads: Query<
        (
            &mut head::HeadRef,
            &mut head::WorkingGraph<N>,
            &mut GraphView,
        ),
        With<head::OpenHead>,
    >,
) where
    N: 'static
        + Node
        + Clone
        + serde::Serialize
        + serde::de::DeserializeOwned
        + ca::CaHash
        + gantz_egui::sync::AsNamedRef
        + Send
        + Sync,
{
    let event = trigger.event();
    let Some(text) = event.data.text.clone().or_else(|| clipboard.get_text()) else {
        return;
    };
    let Ok((mut head_ref, mut wg, mut gv)) = heads.get_mut(event.head) else {
        log::error!("PasteSelection: head not found for entity {:?}", event.head);
        return;
    };
    let editing = match &**head_ref {
        ca::Head::Branch(name) => Some(name.clone()),
        ca::Head::Commit(_) => None,
    };
    let Some(head_state) = gui_state.open_heads.get_mut(&**head_ref) else {
        log::error!("PasteSelection: GUI state not found for head");
        return;
    };

    let pasted = gantz_egui::ops::paste(
        &mut registry,
        editing.as_deref(),
        &mut views,
        &mut demos,
        &mut wg,
        &mut gv,
        head_state,
        &text,
        &event.data.pos,
    );

    // Re-register the full root graph so pasted nodes get their state
    // initialized with the correct nested hashmap structure. Idempotent
    // for existing nodes.
    if pasted {
        if let Some(vm) = vms.get_mut(&event.head) {
            let node_reg = registry_ref(&registry, &builtins, &demos);
            let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
            gantz_core::graph::register(&get_node, &**wg, &[], vm);
        }
    }

    // Uphold the `WorkingGraph` invariant: commit the in-place edit.
    bevy_gantz::commit_working_graph(&mut registry, &mut cmds, event.head, &mut head_ref.0, &wg.0);
}

/// Handle undo payloads: move the head back to its parent commit.
pub fn on_undo<N>(
    trigger: On<ForHead<gantz_egui::Undo>>,
    registry: Res<Registry<N>>,
    mut gui_state: ResMut<GuiState>,
    heads: Query<&head::HeadRef, With<head::OpenHead>>,
    mut cmds: Commands,
) where
    N: 'static + Send + Sync,
{
    let entity = trigger.event().head;
    let Ok(head_ref) = heads.get(entity) else {
        log::error!("Undo: head not found for entity {entity:?}");
        return;
    };
    let head = (**head_ref).clone();
    let parent = gantz_egui::ops::undo(&registry, &mut gui_state.redo_stacks, &head);
    if let Some(parent) = parent {
        navigate_head(&mut cmds, entity, &head, parent);
    }
}

/// Handle redo payloads: move the head forward to a previously undone commit.
pub fn on_redo(
    trigger: On<ForHead<gantz_egui::Redo>>,
    mut gui_state: ResMut<GuiState>,
    heads: Query<&head::HeadRef, With<head::OpenHead>>,
    mut cmds: Commands,
) {
    let entity = trigger.event().head;
    let Ok(head_ref) = heads.get(entity) else {
        log::error!("Redo: head not found for entity {entity:?}");
        return;
    };
    let head = (**head_ref).clone();
    if let Some(redo_ca) = gantz_egui::ops::redo(&mut gui_state.redo_stacks, &head) {
        navigate_head(&mut cmds, entity, &head, redo_ca);
    }
}

/// Handle export head events.
///
/// Exports the head's graph (with transitive dependencies and views) to a
/// `.gantz` file chosen via an `rfd` file dialog. The export is serialized as
/// `.gantz` text using the [`gantz_egui::export`] infrastructure.
pub fn on_export_head<N>(
    trigger: On<ExportHeadEvent>,
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    views: Res<Views>,
    demos: Res<Demos>,
    heads: Query<&head::HeadRef, With<head::OpenHead>>,
) where
    N: 'static + serde::Serialize + serde::de::DeserializeOwned + Node + Clone + Send + Sync,
{
    let event = trigger.event();
    let Ok(head_ref) = heads.get(event.head) else {
        log::error!("ExportHead: head not found for entity {:?}", event.head);
        return;
    };
    let head: &ca::Head = &**head_ref;

    let node_reg = registry_ref(&registry, &builtins, &demos);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);

    let text = match gantz_egui::export::export_heads_sexpr(
        &get_node,
        &registry,
        &views,
        &demos,
        [head],
    ) {
        Ok(s) => s,
        Err(e) => {
            log::error!("ExportHead: failed to serialize: {e}");
            return;
        }
    };

    // Derive a default filename from the head.
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
/// Exports every named graph (with transitive dependencies and views) to a
/// single `.gantz` file chosen via an `rfd` file dialog.
pub fn on_export_all_named<N>(
    _trigger: On<ExportAllNamedEvent>,
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    views: Res<Views>,
    demos: Res<Demos>,
) where
    N: 'static + serde::Serialize + serde::de::DeserializeOwned + Node + Clone + Send + Sync,
{
    let node_reg = registry_ref(&registry, &builtins, &demos);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);

    let named_heads: Vec<ca::Head> = registry
        .names()
        .keys()
        .map(|name| ca::Head::Branch(name.clone()))
        .collect();

    if named_heads.is_empty() {
        log::info!("ExportAllNamed: no named graphs to export");
        return;
    }

    let text = match gantz_egui::export::export_heads_sexpr(
        &get_node,
        &registry,
        &views,
        &demos,
        named_heads.iter(),
    ) {
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

/// Handle import file events (dropped `.gantz` files).
///
/// Deserializes the export, optionally computes root names, merges into the
/// registry, and opens the unique root head if requested.
pub fn on_import_file<N>(
    trigger: On<ImportFileEvent>,
    mut registry: ResMut<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut views: ResMut<Views>,
    mut demos: ResMut<Demos>,
    mut cmds: Commands,
) where
    N: 'static
        + serde::Serialize
        + serde::de::DeserializeOwned
        + ca::CaHash
        + Node
        + Clone
        + Send
        + Sync,
{
    let event = trigger.event();
    let export = match gantz_egui::export::parse_export::<N>(&event.bytes) {
        Ok(e) => e,
        Err(e) => {
            log::error!("ImportFile: {e}");
            return;
        }
    };

    // Compute the root name before merge if we might open a head.
    let root_name = if event.open_head {
        let node_reg = registry_ref(&registry, &builtins, &demos);
        let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
        gantz_egui::export::unique_root_name(&get_node, &export)
    } else {
        None
    };

    let result = gantz_egui::export::merge_with(&mut registry, &mut views, &mut demos, export);
    log::info!(
        "Imported: {} names added, {} replaced",
        result.names_added.len(),
        result.names_replaced.len(),
    );

    if let Some(name) = root_name {
        cmds.trigger(head::OpenEvent(ca::Head::Branch(name)));
    }
}

/// Reset a base graph to its original state by re-merging from the base export.
pub fn on_reset_base_graph<N>(trigger: On<ResetBaseGraphEvent>, mut registry: ResMut<Registry<N>>)
where
    N: 'static + Clone + serde::Serialize + serde::de::DeserializeOwned + ca::CaHash + Send + Sync,
{
    let name = &trigger.event().0;
    let export: gantz_egui::export::Export<Graph<N>> = match gantz_egui::export::parse_export_at::<N>(
        gantz_base::BYTES,
        crate::base::BASE_TIMESTAMP,
    ) {
        Ok(e) => e,
        Err(e) => {
            log::error!("ResetBaseGraph: failed to parse base: {e}");
            return;
        }
    };
    // Extract just the commits reachable from the target name.
    if let Some(&base_commit_ca) = export.registry.names().get(name) {
        let mut required = std::collections::HashSet::new();
        let mut ca = base_commit_ca;
        loop {
            required.insert(ca);
            match export.registry.commits().get(&ca).and_then(|c| c.parent) {
                Some(parent) => ca = parent,
                None => break,
            }
        }
        let mut subset = export.registry.export(&required);
        subset.insert_name(name.clone(), base_commit_ca);
        registry.merge(subset);
        log::info!("Reset base graph '{name}' to original version");
    } else {
        log::warn!("ResetBaseGraph: name '{name}' not found in base export");
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Keep each open head's camera current and seed its commit's layout baseline.
///
/// Runs every frame. Unlike a blind per-frame copy, this keeps each commit's
/// stored `layout` (node positions) *frozen* as an undo baseline: it is written
/// exactly once - when a commit first has a populated live layout - and is never
/// overwritten in place. Node-position changes only ever produce a *new* commit
/// (see [`settle_layout`]). The camera (`scene_rect`) is excluded from undo, so
/// it tracks the live view in place every frame.
pub fn persist_camera_and_seed<N: 'static + Send + Sync>(
    registry: Res<Registry<N>>,
    mut views: ResMut<Views>,
    heads: Query<(&head::HeadRef, &GraphView), With<head::OpenHead>>,
) {
    for (head_ref, head_view) in heads.iter() {
        let Some(commit_addr) = registry.head_commit_ca(&**head_ref).copied() else {
            continue;
        };
        match views.get_mut(&commit_addr) {
            // Layout frozen as the undo baseline; only the camera tracks live.
            Some(view) => view.scene_rect = head_view.scene_rect,
            // Seed the baseline once the scene has laid the graph out. Guarding
            // on a non-empty layout avoids capturing an empty pre-layout frame.
            None if !head_view.layout.is_empty() => {
                views.insert(commit_addr, (**head_view).clone());
            }
            None => {}
        }
    }
}

/// On layout settle ([`DebouncedInputEvent`]), fork a layout-only commit for any
/// open head whose node positions changed since its frozen baseline view.
///
/// Mirrors the graph-commit path's GUI bookkeeping (migrate per-head GUI state,
/// clear redo, migrate the graph pane) but deliberately does *not* fire
/// [`head::CommittedEvent`]: the graph content is unchanged, so there is nothing
/// to resync, and firing it would churn every sync-enabled referrer's history
/// for a pure layout move. `GraphView` is left untouched - it already holds the
/// settled layout, which now matches the new commit's seeded baseline.
///
/// [`DebouncedInputEvent`]: bevy_gantz::debounced_input::DebouncedInputEvent
pub fn settle_layout<N>(
    mut registry: ResMut<Registry<N>>,
    mut views: ResMut<Views>,
    mut gui_state: ResMut<GuiState>,
    mut ctxs: EguiContexts,
    mut heads: Query<(&mut head::HeadRef, &GraphView), With<head::OpenHead>>,
) where
    N: 'static + Send + Sync,
{
    for (mut head_ref, head_view) in heads.iter_mut() {
        let old_head = (**head_ref).clone();
        let Some(new_commit) = gantz_egui::ops::commit_layout(
            &mut registry,
            &views,
            bevy_gantz::reg::timestamp(),
            &mut head_ref,
            head_view,
        ) else {
            continue;
        };
        // Freeze the new commit's layout baseline this frame, before any
        // debounce-gated `save_views`/export reads the `Views` resource.
        views.insert(new_commit, (**head_view).clone());
        // Clear redo (a new commit invalidates it) and migrate GUI state.
        let new_head = (**head_ref).clone();
        gui_state.migrate_head(&old_head, &new_head, true);
        if let Ok(ctx) = ctxs.ctx_mut() {
            gantz_egui::widget::update_graph_pane_head(ctx, &old_head, &new_head);
        }
    }
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

/// Prune views for unreachable commits.
///
/// This should run after `reg::prune_unused` to clean up views for commits
/// that are no longer reachable from any open head.
pub fn prune_views<N: 'static + Node + Send + Sync>(
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    demos: Res<Demos>,
    mut views: ResMut<Views>,
    heads: Query<&head::HeadRef, With<head::OpenHead>>,
) {
    let node_reg = registry_ref(&registry, &builtins, &demos);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    let head_iter = heads.iter().map(|h| &**h);
    let required = gantz_core::reg::required_commits(&get_node, &registry, head_iter);
    views.retain(|ca, _| required.contains(ca));
}

/// Update the Gantz GUI and process widget responses.
///
/// This system:
/// - Shows the Gantz widget in an egui CentralPanel
/// - Processes GUI responses (head open/close/replace, branch creation, etc.)
/// - Dispatches dynamic response payloads via [`ResponseDispatchers`]
/// - Uses TraceCapture for tracing and PerfVm/PerfGui for performance capture
pub fn update<N>(
    trace_capture: Res<TraceCapture>,
    mut perf_vm: ResMut<PerfVm>,
    mut perf_gui: ResMut<PerfGui>,
    mut ctxs: EguiContexts,
    mut registry: ResMut<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<head::HeadVms>,
    tab_order: Res<head::HeadTabOrder>,
    mut focused: ResMut<head::FocusedHead>,
    mut heads_query: Query<OpenHeadViews<N>, With<head::OpenHead>>,
    import_task: Option<Res<ImportTask>>,
    (base_names, base_immutable, mut compile_config, mut change_validation): (
        Res<BaseNames>,
        Res<BaseImmutable>,
        ResMut<CompileConfig>,
        ResMut<bevy_gantz::ValidateCommitted>,
    ),
    mut demos: ResMut<Demos>,
    dispatchers: Res<ResponseDispatchers>,
    mut cmds: Commands,
) -> Result
where
    N: 'static
        + Node
        + Clone
        + gantz_ca::CaHash
        + gantz_egui::NodeUi
        + gantz_egui::sync::AsNamedRef
        + Send
        + Sync,
{
    let ctx = ctxs.ctx_mut()?;

    // Measure GUI frame time.
    let gui_start = web_time::Instant::now();

    // Determine the focused head index from the focused entity.
    let focused_ix = (**focused)
        .and_then(|e| tab_order.iter().position(|&x| x == e))
        .unwrap_or(0);

    // Map heads to entities for response payload dispatch (after `show`).
    let head_to_entity: HashMap<ca::Head, Entity> = tab_order
        .iter()
        .filter_map(|&e| {
            let data = heads_query.get(e).ok()?;
            Some(((**data.core.head_ref).clone(), e))
        })
        .collect();

    // Create the head access adapter.
    let mut access = HeadAccess::new(&tab_order, &mut heads_query, &mut vms);

    // Construct node registry on-demand for the widget (with demo + doc lookup).
    let node_reg = registry_ref(&registry, &builtins, &demos);

    let level = bevy_log::tracing_subscriber::filter::LevelFilter::current();

    // Build and show the Gantz widget.
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

    let mut response = egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show_inside(&mut panel_ui, |ui| {
            gantz_egui::widget::Gantz::new(&node_reg, &base_names.0)
                .base_immutable(base_immutable.0)
                .demos(&demos.0)
                .compile_config(current_compile_config)
                .validate_change_tracking(current_validate_change_tracking)
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
        for mut data in access.iter_mut() {
            if let ca::Head::Branch(head_name) = &**data.core.head_ref {
                if *head_name == name {
                    let commit_ca = *registry.head_commit_ca(&*data.core.head_ref).unwrap();
                    **data.core.head_ref = ca::Head::Commit(commit_ca);
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

    // Handle file drops (egui-level DnD).
    for drop in &response.file_drops {
        let open_head = drop.target == gantz_egui::widget::gantz::FileDropTarget::GraphScene;
        cmds.trigger(ImportFileEvent {
            bytes: drop.bytes.clone(),
            open_head,
        });
    }

    // Handle demo graph association change. Keyed by the head's branch name so
    // the association survives later edits (which mint a new commit).
    if let Some((ca::Head::Branch(graph_name), demo_val)) = &response.demo_changed {
        match demo_val {
            Some(demo) => {
                demos.insert(graph_name.clone(), demo.clone());
            }
            None => {
                demos.remove(graph_name);
            }
        }
    }

    // Handle a graph description edit (keyed by the graph's name).
    if let Some((ca::Head::Branch(name), description)) = &response.description_changed {
        registry.set_description(name.clone(), description.clone());
    }

    // Handle demo reset - re-merge the base version of the demo graph.
    if let Some(head) = &response.reset_base_graph {
        if let ca::Head::Branch(name) = head {
            cmds.trigger(ResetBaseGraphEvent(name.clone()));
        }
    }

    // Handle "reset all demos" - re-merge every `demo-*` base graph.
    if response.reset_all_demos {
        for name in base_names.0.keys() {
            if name.starts_with("demo-") {
                cmds.trigger(ResetBaseGraphEvent(name.clone()));
            }
        }
    }

    // Handle compile config change. The recompile happens next frame via
    // `bevy_gantz::vm::sync`, which compares each head's compile inputs
    // (graph content address + config) by value.
    if let Some(cfg) = response.compile_config {
        if compile_config.0 != cfg {
            compile_config.0 = cfg;
        }
    }

    // Toggle change-tracking validation (a debugging aid; see `vm::sync`).
    if let Some(enabled) = response.validate_change_tracking {
        change_validation.0 = enabled;
    }

    // Handle import button - open a file dialog (only if none already in flight).
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

    // Commit each head whose graph the GUI edited in place this pass (node UI
    // edits + structural scene edits) before returning, upholding the
    // `WorkingGraph` invariant so `vm::sync` recompiles it from the committed
    // address without re-hashing every open graph (#159). Graph ops applied via
    // response observers commit themselves.
    for head in &response.changed_heads {
        let Some(&entity) = head_to_entity.get(head) else {
            continue;
        };
        let Ok(mut data) = heads_query.get_mut(entity) else {
            continue;
        };
        bevy_gantz::commit_working_graph(
            &mut registry,
            &mut cmds,
            entity,
            &mut data.core.head_ref.0,
            &data.core.working_graph.0,
        );
    }

    // Dispatch the dynamic response payloads emitted during the GUI pass.
    // DynResponse types are registered in `ResponseDispatchers` (see
    // `RegisterResponseExt`); unregistered payloads are reported.
    for (head, payload) in response.responses.drain() {
        log::debug!("{payload:?}");
        let entity = head.and_then(|h| head_to_entity.get(&h).copied());
        match dispatchers.0.get(&payload.type_id()) {
            Some(dispatch) => dispatch(entity, payload, &mut cmds),
            None => log::warn!("unhandled response payload: {}", payload.type_name()),
        }
    }

    // Record GUI frame time.
    perf_gui.0.record(gui_start.elapsed());

    Ok(())
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

/// Downcast a dispatched payload to its concrete type.
///
/// Dispatchers are keyed by the payload's `TypeId`, so the downcast cannot
/// fail for a correctly registered dispatcher.
fn downcast_payload<T: ResponseData>(payload: DynResponse) -> T {
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
    cmds.trigger(EvalEntryEvent { head, entrypoint });
}

/// Dispatch a [`gantz_egui::OpenHead`] payload as a [`head::OpenEvent`].
fn dispatch_open_head(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::OpenHead(target) = downcast_payload(payload);
    cmds.trigger(head::OpenEvent(target));
}

/// Dispatch a [`gantz_egui::ReplaceHead`] payload as a [`head::ReplaceEvent`],
/// navigating the focused tab to the target head in place (e.g. entering a
/// nested graph or breadcrumb navigation).
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

/// Trigger the appropriate event to move a head to a target commit.
///
/// Branch heads use `MoveBranchEvent` for atomic registry+graph updates
/// (avoids oscillation with `vm::sync`). Commit heads use `ReplaceEvent`.
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
