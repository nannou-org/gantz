//! Egui integration for bevy_gantz.
//!
//! This crate provides:
//! - [`GantzEguiPlugin`] — Bevy plugin for egui-based UI
//! - GUI state resources and observers
//! - The main `update` system for rendering the gantz GUI

pub mod storage;

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryData;
use bevy_egui::egui;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};
use bevy_gantz::eval::{EvalEvent, EvalKind};
use bevy_gantz::head::{
    self, BranchedEvent, ClosedEvent, CommittedEvent, FocusedHead, HeadRef, HeadTabOrder, HeadVms,
    OpenEvent, OpenHead, OpenHeadData, OpenedEvent, ReplacedEvent, WorkingGraph,
};
use bevy_gantz::reg::Registry;
use bevy_gantz::{BuiltinNodes, VmExecCompleted};
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::Node;
use gantz_core::node::graph::Graph;
use gantz_egui::HeadDataMut;
pub use gantz_egui::RegistryRef;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use steel::steel_vm::engine::Engine;

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
pub struct GantzEguiPlugin<N>(PhantomData<N>);

impl<N> Default for GantzEguiPlugin<N> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<N> Plugin for GantzEguiPlugin<N>
where
    N: Node
        + Clone
        + gantz_ca::CaHash
        + From<gantz_egui::node::NamedRef>
        + gantz_egui::NodeUi
        + gantz_egui::widget::graph_scene::ToGraphMut<Node = N>
        + Send
        + Sync
        + 'static,
{
    fn build(&self, app: &mut App) {
        app.init_resource::<GuiState>()
            .init_resource::<TraceCapture>()
            .init_resource::<PerfVm>()
            .init_resource::<PerfGui>()
            .init_resource::<Views>()
            // GUI state observers
            .add_observer(on_head_opened::<N>)
            .add_observer(on_head_replaced::<N>)
            .add_observer(on_head_closed)
            .add_observer(on_branch_created)
            .add_observer(on_head_committed)
            // VM timing observer
            .add_observer(on_vm_exec_completed)
            // Node creation/inspection observers
            .add_observer(on_create_node::<N>)
            .add_observer(on_inspect_edge::<N>)
            // Systems
            .add_systems(Update, (process_cmds::<N>, persist_views::<N>))
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
pub struct GraphViews(pub gantz_egui::GraphViews);

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
pub struct Views(pub HashMap<ca::CommitAddr, gantz_egui::GraphViews>);

// ----------------------------------------------------------------------------
// Events
// ----------------------------------------------------------------------------

/// Event emitted when an edge inspection is requested.
///
/// Apps should handle this with an observer that has access to the
/// app-specific Environment for creating and registering the inspect node.
#[derive(Event)]
pub struct InspectEdgeEvent {
    /// The head entity on which the edge exists.
    pub head: Entity,
    /// The inspect edge command from the GUI.
    pub cmd: gantz_egui::InspectEdge,
}

/// Event emitted when node creation is requested.
///
/// Apps should handle this with an observer that has access to the
/// app-specific Builtins for creating nodes.
#[derive(Event)]
pub struct CreateNodeEvent {
    /// The head entity where the node should be created.
    pub head: Entity,
    /// The create node command from the GUI.
    pub cmd: gantz_egui::CreateNode,
}

// ----------------------------------------------------------------------------
// QueryData
// ----------------------------------------------------------------------------

/// Bundled query data for open heads (core data + views).
#[derive(QueryData)]
#[query_data(mutable)]
pub struct OpenHeadViews<N: 'static + Send + Sync> {
    pub core: OpenHeadData<N>,
    pub views: &'static mut GraphViews,
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
    query: &'q mut Query<'w, 's, OpenHeadViews<N>, With<OpenHead>>,
    /// The VMs keyed by entity.
    vms: &'q mut HeadVms,
}

impl<'q, 'w, 's, N: 'static + Send + Sync> HeadAccess<'q, 'w, 's, N> {
    pub fn new(
        tab_order: &HeadTabOrder,
        query: &'q mut Query<'w, 's, OpenHeadViews<N>, With<OpenHead>>,
        vms: &'q mut HeadVms,
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
            views: &mut *data.views,
            vm,
        }))
    }

    fn compiled_module(&self, head: &ca::Head) -> Option<&str> {
        let entity = *self.head_to_entity.get(head)?;
        let data = self.query.get(entity).ok()?;
        Some(&*data.core.compiled)
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
    type Target = HashMap<ca::CommitAddr, gantz_egui::GraphViews>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Views {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for GraphViews {
    type Target = gantz_egui::GraphViews;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GraphViews {
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
) -> RegistryRef<'a, N> {
    RegistryRef::new(&**registry, &*builtins.0)
}

// ----------------------------------------------------------------------------
// Observers
// ----------------------------------------------------------------------------

/// Record VM execution timing from VmExecCompleted events.
///
/// This observer receives timing events from `bevy_gantz::eval` and records
/// them to `PerfVm` for the performance widget.
fn on_vm_exec_completed(trigger: On<VmExecCompleted>, mut perf_vm: ResMut<PerfVm>) {
    perf_vm.0.record(trigger.event().duration);
}

/// Initialize GUI state entry and components for opened head.
///
/// Loads views from the `Views` resource and spawns `GraphViews` + `HeadGuiState` components.
pub fn on_head_opened<N: 'static + Send + Sync>(
    trigger: On<OpenedEvent>,
    registry: Res<Registry<N>>,
    views: Res<Views>,
    mut gui_state: ResMut<GuiState>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    gui_state.open_heads.entry(event.head.clone()).or_default();

    // Load views for this head's commit.
    let head_views = registry
        .head_commit_ca(&event.head)
        .and_then(|ca| views.get(ca).cloned())
        .unwrap_or_default();

    cmds.entity(event.entity)
        .insert(HeadGuiState::default())
        .insert(GraphViews(head_views));
}

/// Migrate GUI state for replaced head and reset components.
///
/// Loads views for the new head and updates `GraphViews` + `HeadGuiState` components.
pub fn on_head_replaced<N: 'static + Send + Sync>(
    trigger: On<ReplacedEvent>,
    registry: Res<Registry<N>>,
    views: Res<Views>,
    mut gui_state: ResMut<GuiState>,
    mut ctxs: EguiContexts,
    mut cmds: Commands,
) {
    let event = trigger.event();
    if let Some(state) = gui_state.open_heads.remove(&event.old_head) {
        gui_state.open_heads.insert(event.new_head.clone(), state);
    }
    if let Ok(ctx) = ctxs.ctx_mut() {
        gantz_egui::widget::update_graph_pane_head(ctx, &event.old_head, &event.new_head);
    }

    // Load views for the new head's commit.
    let head_views = registry
        .head_commit_ca(&event.new_head)
        .and_then(|ca| views.get(ca).cloned())
        .unwrap_or_default();

    cmds.entity(event.entity)
        .insert(HeadGuiState::default())
        .insert(GraphViews(head_views));
}

/// Remove GUI state for closed head.
pub fn on_head_closed(trigger: On<ClosedEvent>, mut gui_state: ResMut<GuiState>) {
    gui_state.open_heads.remove(&trigger.event().head);
}

/// Migrate GUI state for branch creation.
pub fn on_branch_created(
    trigger: On<BranchedEvent>,
    mut gui_state: ResMut<GuiState>,
    mut ctxs: EguiContexts,
) {
    let event = trigger.event();
    if let Some(state) = gui_state.open_heads.remove(&event.old_head) {
        gui_state.open_heads.insert(event.new_head.clone(), state);
    }
    if let Ok(ctx) = ctxs.ctx_mut() {
        gantz_egui::widget::update_graph_pane_head(ctx, &event.old_head, &event.new_head);
    }
}

/// Handle graph commit by updating egui state.
///
/// This observer is triggered by `vm::update` when a graph change is committed.
pub fn on_head_committed(
    trigger: On<CommittedEvent>,
    mut gui_state: ResMut<GuiState>,
    mut ctxs: EguiContexts,
) {
    let event = trigger.event();

    // Migrate GUI state to new head.
    if let Some(state) = gui_state.open_heads.remove(&event.old_head) {
        gui_state.open_heads.insert(event.new_head.clone(), state);
    }

    // Update egui pane ID mapping.
    if let Ok(ctx) = ctxs.ctx_mut() {
        gantz_egui::widget::update_graph_pane_head(ctx, &event.old_head, &event.new_head);
    }
}

/// Handle create node events.
pub fn on_create_node<N>(
    trigger: On<CreateNodeEvent>,
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut vms: NonSendMut<HeadVms>,
    mut heads: Query<OpenHeadData<N>, With<OpenHead>>,
) where
    N: 'static
        + Node
        + From<gantz_egui::node::NamedRef>
        + gantz_egui::widget::graph_scene::ToGraphMut<Node = N>
        + Send
        + Sync,
{
    let event = trigger.event();
    let Ok(mut data) = heads.get_mut(event.head) else {
        log::error!("CreateNode: head not found for entity {:?}", event.head);
        return;
    };
    let Some(vm) = vms.get_mut(&event.head) else {
        log::error!("CreateNode: VM not found for entity {:?}", event.head);
        return;
    };

    let Some(graph) = gantz_egui::widget::graph_scene::index_path_graph_mut(
        &mut data.working_graph,
        &event.cmd.path,
    ) else {
        log::error!(
            "CreateNode: could not find graph at path {:?}",
            event.cmd.path
        );
        return;
    };

    let node_reg = registry_ref(&registry, &builtins);
    let Some(node) = node_reg.create_node(&event.cmd.node_type) else {
        log::error!("CreateNode: unknown node type {:?}", event.cmd.node_type);
        return;
    };

    let id = graph.add_node(node);
    let ix = id.index();

    let node_path: Vec<_> = event.cmd.path.iter().copied().chain(Some(ix)).collect();
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    let reg_ctx = gantz_core::node::RegCtx::new(&get_node, &node_path, vm);
    graph[id].register(reg_ctx);
}

/// Handle inspect edge events.
pub fn on_inspect_edge<N>(
    trigger: On<InspectEdgeEvent>,
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut vms: NonSendMut<HeadVms>,
    mut heads: Query<OpenHeadData<N>, With<OpenHead>>,
    mut views_query: Query<&mut GraphViews, With<OpenHead>>,
) where
    N: 'static
        + Node
        + From<gantz_egui::node::NamedRef>
        + gantz_egui::widget::graph_scene::ToGraphMut<Node = N>
        + Send
        + Sync,
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

    let node_reg = registry_ref(&registry, &builtins);
    inspect_edge(
        &node_reg,
        &mut data.working_graph,
        &mut views,
        vm,
        event.cmd.clone(),
    );
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Persist views for all open heads to the Views resource.
///
/// This runs every frame to capture layout changes (node dragging, etc.)
/// that occur without graph topology changes.
pub fn persist_views<N: 'static + Send + Sync>(
    registry: Res<Registry<N>>,
    mut views: ResMut<Views>,
    heads: Query<(&HeadRef, &GraphViews), With<OpenHead>>,
) {
    for (head_ref, head_views) in heads.iter() {
        if let Some(commit_addr) = registry.head_commit_ca(&**head_ref).copied() {
            views.insert(commit_addr, (**head_views).clone());
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
    mut views: ResMut<Views>,
    heads: Query<&HeadRef, With<OpenHead>>,
) {
    let node_reg = registry_ref(&registry, &builtins);
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    let head_iter = heads.iter().map(|h| &**h);
    let required = gantz_core::reg::required_commits(&get_node, &registry, head_iter);
    views.retain(|ca, _| required.contains(ca));
}

/// Process GUI commands from all open heads.
///
/// Handles eval, navigation, and registry commands directly.
/// Emits `InspectEdgeEvent` for edge inspection (requires app-specific handling).
pub fn process_cmds<N: 'static + Send + Sync>(
    mut registry: ResMut<Registry<N>>,
    mut gui_state: ResMut<GuiState>,
    heads: Query<(Entity, &HeadRef), With<OpenHead>>,
    mut cmds: Commands,
) {
    // Collect heads to process.
    let heads_to_process: Vec<_> = heads
        .iter()
        .map(|(entity, head_ref)| (entity, (**head_ref).clone()))
        .collect();

    for (entity, head) in heads_to_process {
        let head_state = gui_state.open_heads.entry(head.clone()).or_default();
        for cmd in std::mem::take(&mut head_state.scene.cmds) {
            log::debug!("{cmd:?}");
            match cmd {
                gantz_egui::Cmd::PushEval(path) => {
                    cmds.trigger(EvalEvent {
                        head: entity,
                        path,
                        kind: EvalKind::Push,
                    });
                }
                gantz_egui::Cmd::PullEval(path) => {
                    cmds.trigger(EvalEvent {
                        head: entity,
                        path,
                        kind: EvalKind::Pull,
                    });
                }
                gantz_egui::Cmd::OpenGraph(path) => {
                    let head_state = gui_state.open_heads.get_mut(&head).unwrap();
                    head_state.path = path;
                }
                gantz_egui::Cmd::OpenNamedNode(name, content_ca) => {
                    let commit_ca = ca::CommitAddr::from(content_ca);
                    if registry.names().get(&name) == Some(&commit_ca) {
                        cmds.trigger(OpenEvent(ca::Head::Branch(name.to_string())));
                    } else {
                        log::debug!(
                            "Attempted to open named node, but the content address has changed"
                        );
                    }
                }
                gantz_egui::Cmd::ForkNamedNode { new_name, ca } => {
                    let commit_ca = ca::CommitAddr::from(ca);
                    registry.insert_name(new_name.clone(), commit_ca);
                    log::info!("Forked node to new name: {new_name}");
                }
                gantz_egui::Cmd::InspectEdge(cmd) => {
                    cmds.trigger(InspectEdgeEvent { head: entity, cmd });
                }
                gantz_egui::Cmd::CreateNode(cmd) => {
                    cmds.trigger(CreateNodeEvent { head: entity, cmd });
                }
            }
        }
    }
}

/// Update the Gantz GUI and process widget responses.
///
/// This system:
/// - Shows the Gantz widget in an egui CentralPanel
/// - Processes GUI responses (head open/close/replace, branch creation, etc.)
/// - Uses TraceCapture for tracing and PerfVm/PerfGui for performance capture
pub fn update<N>(
    trace_capture: Res<TraceCapture>,
    mut perf_vm: ResMut<PerfVm>,
    mut perf_gui: ResMut<PerfGui>,
    mut ctxs: EguiContexts,
    mut registry: ResMut<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut gui_state: ResMut<GuiState>,
    mut vms: NonSendMut<HeadVms>,
    tab_order: Res<HeadTabOrder>,
    mut focused: ResMut<FocusedHead>,
    mut heads_query: Query<OpenHeadViews<N>, With<OpenHead>>,
    mut cmds: Commands,
) -> Result
where
    N: 'static
        + Node
        + gantz_ca::CaHash
        + gantz_egui::NodeUi
        + gantz_egui::widget::graph_scene::ToGraphMut<Node = N>
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

    // Create the head access adapter.
    let mut access = HeadAccess::new(&tab_order, &mut heads_query, &mut vms);

    // Construct node registry on-demand for the widget.
    let node_reg = registry_ref(&registry, &builtins);

    let level = bevy_log::tracing_subscriber::filter::LevelFilter::current();

    // Build and show the Gantz widget.
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
        cmds.trigger(head::BranchEvent {
            original: original_head.clone(),
            new_name: new_name.clone(),
        });
    }

    // Record GUI frame time.
    perf_gui.0.record(gui_start.elapsed());

    Ok(())
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

/// Insert an Inspect node on the given edge, replacing the edge with two edges.
fn inspect_edge<N>(
    node_reg: &RegistryRef<N>,
    wg: &mut WorkingGraph<N>,
    gv: &mut GraphViews,
    vm: &mut Engine,
    cmd: gantz_egui::InspectEdge,
) where
    N: 'static
        + Node
        + From<gantz_egui::node::NamedRef>
        + gantz_egui::widget::graph_scene::ToGraphMut<Node = N>
        + Send
        + Sync,
{
    let gantz_egui::InspectEdge { path, edge, pos } = cmd;

    let graph: &mut Graph<N> = &mut *wg;
    let views: &mut gantz_egui::GraphViews = &mut *gv;

    let Some(nested) = gantz_egui::widget::graph_scene::index_path_graph_mut(graph, &path) else {
        log::error!("InspectEdge: could not find graph at path");
        return;
    };

    let Some((src_node, dst_node)) = nested.edge_endpoints(edge) else {
        log::error!("InspectEdge: edge not found");
        return;
    };
    let edge_weight = *nested.edge_weight(edge).unwrap();

    nested.remove_edge(edge);

    let Some(inspect_node) = node_reg.create_node("inspect") else {
        log::error!("InspectEdge: could not create inspect node");
        return;
    };
    let inspect_id = nested.add_node(inspect_node);

    let node_path: Vec<_> = path
        .iter()
        .copied()
        .chain(Some(inspect_id.index()))
        .collect();
    let get_node = |ca: &ca::ContentAddr| node_reg.node(ca);
    let reg_ctx = gantz_core::node::RegCtx::new(&get_node, &node_path, vm);
    nested[inspect_id].register(reg_ctx);

    nested.add_edge(
        src_node,
        inspect_id,
        gantz_core::Edge::new(edge_weight.output, gantz_core::node::Input(0)),
    );

    nested.add_edge(
        inspect_id,
        dst_node,
        gantz_core::Edge::new(gantz_core::node::Output(0), edge_weight.input),
    );

    let node_id = egui_graph::NodeId::from_u64(inspect_id.index() as u64);
    let view = views.entry(path).or_default();
    view.layout.insert(node_id, pos);
}
