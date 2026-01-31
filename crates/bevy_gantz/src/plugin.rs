//! The GantzPlugin for Bevy applications.

use crate::egui::{self, GuiState};
use crate::eval::on_eval_event;
use crate::head::{FocusedHead, HeadTabOrder, HeadVms};
use crate::reg::Registry;
use crate::view::Views;
use bevy_app::prelude::*;
use std::marker::PhantomData;

/// Plugin providing core gantz functionality.
///
/// Generic over `N`, the node type used in graphs.
///
/// This plugin:
/// - Initializes core resources (Registry, Views, HeadVms, GuiState, etc.)
/// - Registers event observers for head operations
/// - Registers the eval event observer
/// - Handles GUI state management automatically
///
/// Apps should also:
/// - Insert a `BuiltinNodes<N>` resource with their builtin nodes
/// - Add observers for HeadOpened/HeadReplaced if VM initialization is needed
pub struct GantzPlugin<N>(PhantomData<N>);

impl<N> Default for GantzPlugin<N> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<N: Clone + Send + Sync + 'static> Plugin for GantzPlugin<N> {
    fn build(&self, app: &mut App) {
        use crate::head::{on_close_head, on_create_branch, on_open_head, on_replace_head};

        app.init_resource::<FocusedHead>()
            .init_resource::<HeadTabOrder>()
            .init_resource::<Registry<N>>()
            .init_resource::<Views>()
            .init_resource::<GuiState>()
            .init_non_send_resource::<HeadVms>()
            // Register head event handlers.
            .add_observer(on_open_head::<N>)
            .add_observer(on_replace_head::<N>)
            .add_observer(on_close_head::<N>)
            .add_observer(on_create_branch::<N>)
            // Register eval event handler.
            .add_observer(on_eval_event)
            // Register GUI state handlers.
            .add_observer(egui::on_head_opened)
            .add_observer(egui::on_head_replaced)
            .add_observer(egui::on_head_closed)
            .add_observer(egui::on_branch_created);
    }
}
