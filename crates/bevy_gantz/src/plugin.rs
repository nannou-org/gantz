//! The GantzPlugin for Bevy applications.

use crate::eval::on_eval_event;
use crate::head::{FocusedHead, HeadTabOrder, HeadVms};
use crate::reg::Registry;
use crate::vm;
use bevy_app::prelude::*;
use gantz_core::Node;
use std::marker::PhantomData;

/// Plugin providing core gantz functionality.
///
/// Generic over `N`, the node type used in graphs.
///
/// This plugin:
/// - Initializes core resources (Registry, HeadVms, etc.)
/// - Registers event observers for head operations
/// - Registers the eval event observer
/// - Handles VM initialization for opened/replaced heads
/// - Detects graph changes and recompiles VMs
///
/// Apps should also:
/// - Insert a `BuiltinNodes<N>` resource with their builtin nodes
/// - Add `GantzEguiPlugin` for egui integration (Views, GraphViews, etc.)
pub struct GantzPlugin<N>(PhantomData<N>);

impl<N> Default for GantzPlugin<N> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<N> Plugin for GantzPlugin<N>
where
    N: Node + Clone + gantz_ca::CaHash + Send + Sync + 'static,
{
    fn build(&self, app: &mut App) {
        use crate::head::{on_close_head, on_create_branch, on_open_head, on_replace_head};

        app.init_resource::<FocusedHead>()
            .init_resource::<HeadTabOrder>()
            .init_resource::<Registry<N>>()
            .init_non_send_resource::<HeadVms>()
            // Register head event handlers.
            .add_observer(on_open_head::<N>)
            .add_observer(on_replace_head::<N>)
            .add_observer(on_close_head::<N>)
            .add_observer(on_create_branch::<N>)
            // Register eval event handler.
            .add_observer(on_eval_event)
            // VM init observers.
            .add_observer(vm::on_head_opened::<N>)
            .add_observer(vm::on_head_replaced::<N>)
            // Graph recompilation system.
            .add_systems(Update, vm::update::<N>);
    }
}
