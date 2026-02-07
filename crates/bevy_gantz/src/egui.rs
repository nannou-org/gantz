//! Egui integration for gantz.
//!
//! This module provides:
//! - [`GantzEguiPlugin`] — Bevy plugin for egui-based UI
//! - GUI state resources and observers
//! - The main `update` system for rendering the gantz GUI

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_egui::egui;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::Node;
use std::marker::PhantomData;

use crate::BuiltinNodes;
use crate::eval::{EvalEvent, EvalKind};
use crate::head::{
    self, BranchCreated, FocusedHead, GraphViews, HeadClosed, HeadCommitted, HeadGuiState,
    HeadOpened, HeadReplaced, HeadTabOrder, HeadVms, OpenEvent, OpenHead, OpenHeadData,
    WorkingGraph,
};
use crate::reg::{Registry, RegistryRef};
use gantz_core::node::{self, graph::Graph};
use std::collections::BTreeMap;
use steel::steel_vm::engine::Engine;

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

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
            // GUI state observers
            .add_observer(on_head_opened)
            .add_observer(on_head_replaced)
            .add_observer(on_head_closed)
            .add_observer(on_branch_created)
            .add_observer(on_head_committed)
            // Node creation/inspection observers
            .add_observer(on_create_node::<N>)
            .add_observer(on_inspect_edge::<N>)
            // Systems
            .add_systems(Update, process_cmds::<N>)
            .add_systems(EguiPrimaryContextPass, update::<N>);
    }
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Captures tracing logs for the TraceView widget.
#[derive(Default, Resource)]
pub struct TraceCapture(pub gantz_egui::widget::trace_view::TraceCapture);

/// Performance capture for VM execution timing.
#[derive(Default, Resource)]
pub struct PerfVm(pub gantz_egui::widget::PerfCapture);

/// Performance capture for GUI frame timing.
#[derive(Default, Resource)]
pub struct PerfGui(pub gantz_egui::widget::PerfCapture);

// ---------------------------------------------------------------------------
// GuiState
// ---------------------------------------------------------------------------

/// The gantz GUI state (open head states, etc.).
#[derive(Resource, Default)]
pub struct GuiState(pub gantz_egui::widget::GantzState);

impl std::ops::Deref for GuiState {
    type Target = gantz_egui::widget::GantzState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for GuiState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Initialize GUI state entry and component for opened head.
pub fn on_head_opened(
    trigger: On<HeadOpened>,
    mut gui_state: ResMut<GuiState>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    gui_state.open_heads.entry(event.head.clone()).or_default();
    cmds.entity(event.entity).insert(HeadGuiState::default());
}

/// Migrate GUI state for replaced head and reset component.
pub fn on_head_replaced(
    trigger: On<HeadReplaced>,
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
    cmds.entity(event.entity).insert(HeadGuiState::default());
}

/// Remove GUI state for closed head.
pub fn on_head_closed(trigger: On<HeadClosed>, mut gui_state: ResMut<GuiState>) {
    gui_state.open_heads.remove(&trigger.event().head);
}

/// Migrate GUI state for branch creation.
pub fn on_branch_created(
    trigger: On<BranchCreated>,
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
    trigger: On<HeadCommitted>,
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

// ---------------------------------------------------------------------------
// Node Creation/Inspection Observers
// ---------------------------------------------------------------------------

/// Handle create node events.
pub fn on_create_node<N>(
    trigger: On<CreateNodeEvent>,
    registry: Res<Registry<N>>,
    builtins: Res<BuiltinNodes<N>>,
    mut vms: NonSendMut<HeadVms>,
    mut heads: Query<OpenHeadData<N>, With<OpenHead>>,
) where
    N: Node
        + From<gantz_egui::node::NamedRef>
        + gantz_egui::widget::graph_scene::ToGraphMut<Node = N>
        + Send
        + Sync
        + 'static,
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

    let node_reg = RegistryRef::new(&*registry, &*builtins);
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
) where
    N: Node
        + From<gantz_egui::node::NamedRef>
        + gantz_egui::widget::graph_scene::ToGraphMut<Node = N>
        + Send
        + Sync
        + 'static,
{
    let event = trigger.event();
    if let Ok(mut data) = heads.get_mut(event.head) {
        if let Some(vm) = vms.get_mut(&event.head) {
            let node_reg = RegistryRef::new(&*registry, &*builtins);
            inspect_edge(
                &node_reg,
                &mut data.working_graph,
                &mut data.views,
                vm,
                event.cmd.clone(),
            );
        }
    }
}

/// Insert an Inspect node on the given edge, replacing the edge with two edges.
fn inspect_edge<N>(
    node_reg: &RegistryRef<N>,
    wg: &mut WorkingGraph<N>,
    gv: &mut GraphViews,
    vm: &mut Engine,
    cmd: gantz_egui::InspectEdge,
) where
    N: Node
        + From<gantz_egui::node::NamedRef>
        + gantz_egui::widget::graph_scene::ToGraphMut<Node = N>
        + Send
        + Sync
        + 'static,
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

// ---------------------------------------------------------------------------
// gantz_egui trait impls for RegistryRef
// ---------------------------------------------------------------------------

impl<N: Node + Send + Sync + 'static> gantz_egui::NodeTypeRegistry for RegistryRef<'_, N> {
    fn node_types(&self) -> Vec<&str> {
        let mut types = vec![];
        types.extend(self.builtins().names());
        types.extend(self.ca_registry().names().keys().map(|s| &s[..]));
        types.sort();
        types
    }
}

impl<N: Node + Send + Sync + 'static> gantz_egui::widget::graph_select::GraphRegistry
    for RegistryRef<'_, N>
{
    fn commits(&self) -> Vec<(&ca::CommitAddr, &ca::Commit)> {
        let mut commits: Vec<_> = self.ca_registry().commits().iter().collect();
        commits.sort_by(|(_, a), (_, b)| b.timestamp.cmp(&a.timestamp));
        commits
    }

    fn names(&self) -> &BTreeMap<String, ca::CommitAddr> {
        self.ca_registry().names()
    }
}

impl<N: Node + Send + Sync + 'static> gantz_egui::node::NameRegistry for RegistryRef<'_, N> {
    fn name_ca(&self, name: &str) -> Option<ca::ContentAddr> {
        if let Some(commit_ca) = self.ca_registry().names().get(name) {
            return Some((*commit_ca).into());
        }
        self.builtins().content_addr(name)
    }

    fn node_exists(&self, ca: &ca::ContentAddr) -> bool {
        self.node(ca).is_some()
    }
}

impl<N: Node + Send + Sync + 'static> gantz_egui::node::FnNodeNames for RegistryRef<'_, N> {
    fn fn_node_names(&self) -> Vec<String> {
        use gantz_egui::node::NameRegistry;

        let builtin_names = self
            .builtins()
            .names()
            .into_iter()
            .filter_map(|name| self.builtins().content_addr(name).map(|_| name.to_string()));
        let registry_names = self.ca_registry().names().keys().cloned();
        let all_names = builtin_names.chain(registry_names);

        let get_node = |ca: &ca::ContentAddr| self.node(ca);
        let mut names: Vec<_> = all_names
            .filter(|name| {
                let meta_ctx = node::MetaCtx::new(&get_node);
                self.name_ca(name)
                    .and_then(|ca| self.node(&ca))
                    .map(|n| {
                        !n.stateful(meta_ctx)
                            && n.branches(meta_ctx).is_empty()
                            && n.n_outputs(meta_ctx) == 1
                    })
                    .unwrap_or(false)
            })
            .collect();

        names.sort();
        names
    }
}

impl<N: Node + Send + Sync + 'static> gantz_egui::Registry for RegistryRef<'_, N> {
    fn node(&self, ca: &ca::ContentAddr) -> Option<&dyn Node> {
        RegistryRef::node(self, ca)
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Process GUI commands from all open heads.
///
/// Handles eval, navigation, and registry commands directly.
/// Emits `InspectEdgeEvent` for edge inspection (requires app-specific handling).
pub fn process_cmds<N: Send + Sync + 'static>(
    mut registry: ResMut<Registry<N>>,
    mut gui_state: ResMut<GuiState>,
    heads: Query<(Entity, &crate::head::HeadRef), With<OpenHead>>,
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

// ---------------------------------------------------------------------------
// Update system
// ---------------------------------------------------------------------------

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
    mut heads_query: Query<OpenHeadData<N>, With<OpenHead>>,
    mut cmds: Commands,
) -> Result
where
    N: Node
        + gantz_ca::CaHash
        + gantz_egui::NodeUi
        + gantz_egui::widget::graph_scene::ToGraphMut<Node = N>
        + Send
        + Sync
        + 'static,
{
    let ctx = ctxs.ctx_mut()?;

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
        let new_head = registry.init_head(crate::reg::timestamp());
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
