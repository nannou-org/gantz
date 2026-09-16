//! Bevy plugin for gantz, an environment for creative systems.
//!
//! This crate provides core Bevy integration for gantz. For egui-based UI,
//! see the `bevy_gantz_egui` crate.
//!
//! # Events vs Messages
//!
//! Observer events use `Event` and `On<T>`. They carry discrete, low-frequency
//! intents and hooks that need immediate handling. They come in two layers.
//!
//! - Request events ask for an operation. They are [`head::OpenEvent`],
//!   [`head::CloseEvent`], [`head::ReplaceEvent`], [`head::BranchHeadEvent`],
//!   [`head::MoveBranchEvent`] and [`vm::EvalEntryEvent`].
//! - Hook events announce that one happened. They decouple this crate from
//!   downstream UI crates. They are [`head::OpenedEvent`],
//!   [`head::ClosedEvent`], [`head::ChangedEvent`],
//!   [`head::BranchedHeadEvent`], [`head::CommittedEvent`] and
//!   [`vm::EvalEntryComplete`].
//!
//! Buffered messages use `Message` and `MessageReader`. They carry per-frame
//! streams that polling systems consume. [`debounced_input::DebouncedInputEvent`]
//! is the one case.

pub mod debounced_input;
pub mod head;
pub mod reg;
pub mod storage;
pub mod task;
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

/// The `Update` system set that runs the UI layer's VM synchronisation system
/// `bevy_gantz_egui::vm::sync`.
///
/// Systems that evaluate head VMs each frame should run `.after(VmSet)`. Then
/// they never observe a head that points at a new graph before its VM is
/// initialized.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub struct VmSet;

/// The `Update` system set that groups the entrypoint drivers for timed
/// evaluations such as `tick!` and `update!`.
///
/// Consumers that read state written by those evaluations should run
/// `.after(EntrypointSet)`. The dsp driver is one such consumer. It drains the
/// per-tick control values that an evaluation queues. The auto-inserted
/// `apply_deferred` at that boundary flushes the [`vm::on_eval_entry`]
/// observers the drivers trigger, so the queued values are visible when the
/// consumer runs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub struct EntrypointSet;

/// A monotonic clock epoch shared across the app, captured once at startup.
///
/// It is the single time base for entrypoint firing times and the dsp engine's
/// scheduling clock. Firing times are written into the `time` field of `%args`.
/// A `tick!`'s exact firing time and the audio thread's buffer time therefore
/// share one timeline with no cross-clock mapping. The clock is monotonic, so
/// NTP steps cannot glitch audio timing.
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
/// It initializes the core resources and registers the head and eval event
/// observers.
///
/// Apps should also add `GantzEguiPlugin` for egui integration. That plugin
/// owns the typed side. It holds the reified-graph cache, the builtin
/// instances and the input-addressed VM synchronisation system in [`VmSet`].
///
/// # Assembly
///
/// Plugin order does not matter. The gantz plugins contribute to shared
/// collections via `get_resource_or_init`, as `bevy_gantz_egui::EntrypointFns`
/// does. They perform cross-plugin resource reads in `Plugin::finish`, as
/// the reference domain plugin `bevy_gantz_plyphon::PlyphonPlugin` does.
#[derive(Default)]
pub struct GantzPlugin;

impl Plugin for GantzPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FocusedHead>()
            .init_resource::<HeadTabOrder>()
            .init_resource::<Registry>()
            .init_resource::<vm::CompileConfig>()
            .init_resource::<vm::SteelModules>()
            .init_resource::<vm::ValidateCommitted>()
            .insert_resource(EvalEpoch(web_time::Instant::now()))
            .init_non_send::<HeadVms>()
            .add_observer(head::on_open)
            .add_observer(head::on_replace)
            .add_observer(head::on_close)
            .add_observer(head::on_branch_head)
            .add_observer(head::on_move_branch)
            .add_observer(vm::on_eval_entry)
            .add_systems(Update, vm::validate_committed.after(VmSet));
    }
}
