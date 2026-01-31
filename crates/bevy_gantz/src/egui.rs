//! Egui integration for gantz.

use bevy_ecs::prelude::*;
use bevy_egui::EguiContexts;

use crate::head::{BranchCreated, HeadClosed, HeadOpened, HeadReplaced};

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
