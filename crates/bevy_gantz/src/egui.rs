//! Egui integration for gantz.

use bevy_ecs::prelude::*;
use bevy_egui::EguiContexts;
use bevy_egui::egui;
use bevy_log as log;
use gantz_ca as ca;
use gantz_core::Node;

use crate::BuiltinNodes;
use crate::eval::{EvalEvent, EvalKind};
use crate::head::{
    self, BranchCreated, FocusedHead, HeadClosed, HeadOpened, HeadReplaced, HeadTabOrder, HeadVms,
    OpenEvent, OpenHead, OpenHeadData,
};
use crate::reg::{Registry, RegistryRef};

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

/// Initialize GUI state entry for opened head.
pub fn on_head_opened(trigger: On<HeadOpened>, mut gui_state: ResMut<GuiState>) {
    gui_state
        .open_heads
        .entry(trigger.event().head.clone())
        .or_default();
}

/// Migrate GUI state for replaced head.
pub fn on_head_replaced(
    trigger: On<HeadReplaced>,
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
/// - Optionally uses TraceCapture and PerfVm/PerfGui if inserted as resources
pub fn update<N>(
    trace_capture: Option<Res<TraceCapture>>,
    mut perf_vm: Option<ResMut<PerfVm>>,
    mut perf_gui: Option<ResMut<PerfGui>>,
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
    let mut gantz = gantz_egui::widget::Gantz::new(&node_reg);
    if let Some(ref trace) = trace_capture {
        gantz = gantz.trace_capture(trace.0.clone(), level);
    }
    if let (Some(pv), Some(pg)) = (&mut perf_vm, &mut perf_gui) {
        gantz = gantz.perf_captures(&mut pv.0, &mut pg.0);
    }

    let response = egui::containers::CentralPanel::default()
        .frame(egui::Frame::default())
        .show(ctx, |ui| {
            gantz.show(&mut *gui_state, focused_ix, &mut access, ui)
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
    if let Some(ref mut pg) = perf_gui {
        pg.0.record(gui_start.elapsed());
    }

    Ok(())
}
