//! Egui integration for gantz.

use bevy_ecs::prelude::*;
use bevy_egui::EguiContexts;
use bevy_log as log;
use gantz_ca as ca;

use crate::eval::{EvalEvent, EvalKind};
use crate::head::{BranchCreated, HeadClosed, HeadOpened, HeadReplaced, OpenEvent, OpenHead};
use crate::reg::Registry;

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
