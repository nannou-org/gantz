//! Bevy plugin for gantz - an environment for creative systems.
//!
//! This crate provides core Bevy integration for gantz. For egui-based UI,
//! see the `bevy_gantz_egui` crate.
//!
//! # Events vs Messages
//!
//! Observer events (`Event` + `On<T>`) are used for discrete, low-frequency
//! intents and hooks where immediate, possibly-cascading handling matters.
//! These come in two layers:
//!
//! - *Request* events ask for an operation: [`head::OpenEvent`],
//!   [`head::CloseEvent`], [`head::ReplaceEvent`], [`head::BranchHeadEvent`],
//!   [`head::MoveBranchEvent`], [`vm::EvalEntryEvent`].
//! - *Hook* events announce that one happened, decoupling this crate from
//!   downstream UI crates: [`head::OpenedEvent`], [`head::ClosedEvent`],
//!   [`head::ChangedEvent`], [`head::BranchedHeadEvent`],
//!   [`head::CommittedEvent`], [`vm::EvalEntryComplete`].
//!
//! Buffered messages (`Message` + `MessageReader`) are reserved for
//! per-frame streams consumed by polling systems -
//! [`debounced_input::DebouncedInputEvent`] is the one case.

pub mod debounced_input;
pub mod head;
pub mod reg;
pub mod storage;
pub mod vm;

use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::{IntoScheduleConfigs, Resource, SystemSet};
pub use head::{
    FocusedHead, HeadRef, HeadTabOrder, HeadVms, OpenHead, OpenHeadData, OpenHeadDataReadOnly,
    WorkingGraph,
};
pub use reg::{Registry, timestamp};
pub use vm::{
    CompileConfig, CompiledInputs, EvalEntryComplete, EvalEntryEvent, ValidateCommitted,
    commit_working_graph,
};

/// The system set in which the UI layer's VM synchronisation system
/// (`bevy_gantz_egui::vm::sync`) runs (in the `Update` schedule).
///
/// Systems that evaluate head VMs each frame should run `.after(VmSet)` so
/// they never observe the gap between a head pointing at a new graph and its
/// VM being (re)initialized.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub struct VmSet;

/// The system set grouping the entrypoint drivers that fire timed evaluations
/// (`tick!`, `update!`), in the `Update` schedule.
///
/// Consumers that read state written by those evaluations - notably the dsp
/// driver, which drains the per-tick control values an evaluation queues - should
/// run `.after(EntrypointSet)`. The auto-inserted `apply_deferred` at that
/// boundary flushes the drivers' `cmds.trigger`ed [`vm::on_eval_entry`] observers,
/// so the queued values are visible by the time the consumer runs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub struct EntrypointSet;

/// A monotonic clock epoch shared across the app, captured once at startup.
///
/// It is the single time base for entrypoint firing times (written into
/// `%args`'s `time`) and the dsp engine's scheduling clock, so a `tick!`'s
/// exact firing time and the audio thread's buffer time live on one timeline -
/// no cross-clock mapping, and monotonic so NTP steps can't glitch audio timing.
#[derive(Clone, Copy, Debug, Resource)]
pub struct EvalEpoch(pub web_time::Instant);

impl EvalEpoch {
    /// Monotonic seconds elapsed since the epoch was captured.
    pub fn now_secs(&self) -> f64 {
        self.0.elapsed().as_secs_f64()
    }
}

/// Plugin providing core gantz functionality.
///
/// This plugin:
/// - Initializes core resources (Registry, HeadVms, etc.)
/// - Registers event observers for head operations
/// - Registers the eval event observer
///
/// Apps should also add `GantzEguiPlugin` for egui integration, which owns
/// the typed side (the reified-graph cache, builtin instances and the
/// input-addressed VM synchronisation system running in [`VmSet`]).
///
/// # Assembly
///
/// Plugin order does not matter: the gantz plugins contribute to shared
/// collections via `get_resource_or_init` (see
/// `bevy_gantz_egui::EntrypointFns`) and perform cross-plugin resource reads
/// in `Plugin::finish` (see `bevy_gantz_plyphon::PlyphonPlugin`, the
/// reference domain plugin), so they may be added in any order relative to
/// each other.
#[derive(Default)]
pub struct GantzPlugin;

impl Plugin for GantzPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FocusedHead>()
            .init_resource::<HeadTabOrder>()
            .init_resource::<Registry>()
            .init_resource::<vm::CompileConfig>()
            .init_resource::<vm::ValidateCommitted>()
            .insert_resource(EvalEpoch(web_time::Instant::now()))
            .init_non_send::<HeadVms>()
            // Register head event handlers.
            .add_observer(head::on_open)
            .add_observer(head::on_replace)
            .add_observer(head::on_close)
            .add_observer(head::on_branch_head)
            .add_observer(head::on_move_branch)
            // Register eval entry event handler.
            .add_observer(vm::on_eval_entry)
            // Debug check for the WorkingGraph commit-before-return invariant
            // (no-op unless `ValidateCommitted` is enabled).
            .add_systems(Update, vm::validate_committed.after(VmSet));
    }
}
