//! The GantzPlugin for Bevy applications.

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
    N: 'static + Node + Clone + gantz_ca::CaHash + Send + Sync,
{
    fn build(&self, app: &mut App) {
        use crate::head::{on_branch, on_close, on_open, on_replace};

        app.init_resource::<FocusedHead>()
            .init_resource::<HeadTabOrder>()
            .init_resource::<Registry<N>>()
            .init_non_send_resource::<HeadVms>()
            // Register head event handlers.
            .add_observer(on_open::<N>)
            .add_observer(on_replace::<N>)
            .add_observer(on_close::<N>)
            .add_observer(on_branch::<N>)
            // Register eval event handler.
            .add_observer(vm::on_eval)
            // VM init observers.
            .add_observer(vm::on_head_opened::<N>)
            .add_observer(vm::on_head_replaced::<N>)
            // Graph recompilation system.
            .add_systems(Update, vm::update::<N>);
    }
}
