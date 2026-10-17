//! The dispatch of the payloads a GUI pass emits, onto events and observers.

use crate::{
    ExportAllNamedEvent, ExportHeadEvent, ExportStyleEvent, ImportFileEvent, ImportStyleEvent,
    ResetBaseGraphEvent,
};
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_gantz::head;
use bevy_gantz::vm::EvalEntryEvent;
use bevy_log as log;
use gantz_egui::{DynResponse, ResponseData};
use std::any::TypeId;
use std::collections::HashMap;

/// A GUI response payload targeting an open-head entity.
///
/// The `update` system drains the payloads emitted during the GUI pass and
/// dispatches each via [`ResponseDispatchers`]. Head-scoped payloads arrive
/// as this event. Observers take the mutable per-head queries that the GUI
/// system itself cannot, due to ECS borrow rules.
#[derive(EntityEvent)]
pub struct ForHead<T: Send + Sync + 'static> {
    /// The open-head entity the payload targets.
    #[event_target]
    pub head: Entity,
    /// The payload emitted from the GUI.
    pub data: T,
}

/// A GUI response payload with no target head, such as a settings change.
///
/// The app-level counterpart of [`ForHead`]. Register the payload type with
/// [`RegisterResponseExt::register_app_response`] and observe
/// `On<ForApp<T>>`.
#[derive(Event)]
pub struct ForApp<T: Send + Sync + 'static>(pub T);

/// The signature of dispatch fns stored in [`ResponseDispatchers`].
///
/// The `Option<Entity>` is the open-head entity resolved from the payload's
/// head tag. It is `None` for app-level payloads.
pub type DispatchFn = fn(Option<Entity>, DynResponse, &mut Commands);

/// `TypeId`-keyed dispatchers turning the dynamic GUI response payloads into
/// typed events. Register payload types via [`RegisterResponseExt`].
#[derive(Default, Resource)]
pub struct ResponseDispatchers(pub HashMap<TypeId, DispatchFn>);

/// App extension for registering GUI response payload handlers.
///
/// Nodes declared in independent plugins receive custom payloads emitted from
/// their UI this way. The node's returned [`gantz_egui::NodeUiResponse`] and
/// friends carry an `emit` helper. Register the payload type here and add an
/// observer for [`ForHead<T>`]:
///
/// ```ignore
/// app.register_head_response::<MyPayload>()
///     .add_observer(|t: On<ForHead<MyPayload>>, /* any system params */| { .. });
/// ```
pub trait RegisterResponseExt {
    /// Dispatch payloads of type `T` as [`ForHead<T>`] events targeting the
    /// emitting head's entity. Pair with an observer for `On<ForHead<T>>`.
    fn register_head_response<T: ResponseData>(&mut self) -> &mut Self;

    /// Dispatch payloads of type `T` as [`ForApp<T>`] events. Pair with an
    /// observer for `On<ForApp<T>>`.
    fn register_app_response<T: ResponseData>(&mut self) -> &mut Self;

    /// Dispatch payloads of type `T` with a custom fn. For example, to map onto
    /// an existing event type.
    fn register_response_with<T: ResponseData>(&mut self, f: DispatchFn) -> &mut Self;
}

impl RegisterResponseExt for App {
    fn register_head_response<T: ResponseData>(&mut self) -> &mut Self {
        self.register_response_with::<T>(dispatch_for_head::<T>)
    }

    fn register_app_response<T: ResponseData>(&mut self) -> &mut Self {
        self.register_response_with::<T>(dispatch_for_app::<T>)
    }

    fn register_response_with<T: ResponseData>(&mut self, f: DispatchFn) -> &mut Self {
        self.world_mut()
            .get_resource_or_init::<ResponseDispatchers>()
            .0
            .insert(TypeId::of::<T>(), f);
        self
    }
}

/// Downcast a dispatched payload to its concrete type.
///
/// Dispatchers are keyed by the payload's `TypeId`, so the downcast cannot
/// fail for a correctly registered dispatcher. Public so external plugins
/// such as `bevy_gantz_collab` can write custom [`DispatchFn`]s.
pub fn downcast_payload<T: ResponseData>(payload: DynResponse) -> T {
    payload
        .downcast::<T>()
        .expect("dispatcher registered for this payload type")
}

/// Dispatch a head-scoped payload as a [`ForHead`] event.
fn dispatch_for_head<T: ResponseData>(
    entity: Option<Entity>,
    payload: DynResponse,
    cmds: &mut Commands,
) {
    let Some(head) = entity else {
        log::error!(
            "response payload `{}` has no open-head entity",
            payload.type_name()
        );
        return;
    };
    let data = downcast_payload::<T>(payload);
    cmds.trigger(ForHead { head, data });
}

/// Dispatch a [`gantz_egui::EvalEntry`] payload as an [`EvalEntryEvent`].
pub(crate) fn dispatch_eval_entry(
    entity: Option<Entity>,
    payload: DynResponse,
    cmds: &mut Commands,
) {
    let Some(head) = entity else {
        log::error!("EvalEntry payload has no open-head entity");
        return;
    };
    let gantz_egui::EvalEntry(entrypoint) = downcast_payload(payload);
    cmds.trigger(EvalEntryEvent {
        head,
        entrypoint,
        // A user-driven push fires "now".
        time: None,
    });
}

/// Drop a [`gantz_egui::StateWritten`] payload. Recorded VM-state writes
/// exist for the collaborative-session layer, which overrides this
/// registration to broadcast them. Last registration wins. Without it they
/// are local-only by design, not unhandled.
pub(crate) fn dispatch_state_written(
    _: Option<Entity>,
    _payload: DynResponse,
    _cmds: &mut Commands,
) {
}

/// Dispatch a [`gantz_egui::OpenHead`] payload as a [`head::OpenEvent`].
pub(crate) fn dispatch_open_head(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::OpenHead(target) = downcast_payload(payload);
    cmds.trigger(head::OpenEvent(target));
}

/// Dispatch a [`gantz_egui::ReplaceHead`] payload as a [`head::ReplaceEvent`].
/// This navigates the focused tab to the target head in place, for example
/// when entering a nested graph.
pub(crate) fn dispatch_replace_head(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::ReplaceHead(target) = downcast_payload(payload);
    cmds.trigger(head::ReplaceEvent(target));
}

/// Dispatch a [`gantz_egui::ExportHead`] payload as an [`ExportHeadEvent`].
pub(crate) fn dispatch_export_head(
    entity: Option<Entity>,
    payload: DynResponse,
    cmds: &mut Commands,
) {
    let Some(head) = entity else {
        log::error!("ExportHead payload has no open-head entity");
        return;
    };
    let gantz_egui::ExportHead = downcast_payload(payload);
    cmds.trigger(ExportHeadEvent { head });
}

/// Dispatch a [`gantz_egui::ExportAllNamed`] payload as an [`ExportAllNamedEvent`].
pub(crate) fn dispatch_export_all_named(
    _: Option<Entity>,
    payload: DynResponse,
    cmds: &mut Commands,
) {
    let gantz_egui::ExportAllNamed = downcast_payload(payload);
    cmds.trigger(ExportAllNamedEvent);
}

/// Dispatch a [`gantz_egui::ExportStyle`] payload as an [`ExportStyleEvent`].
pub(crate) fn dispatch_export_style(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::ExportStyle = downcast_payload(payload);
    cmds.trigger(ExportStyleEvent);
}

/// Dispatch a [`gantz_egui::ImportStyle`] payload as an [`ImportStyleEvent`].
pub(crate) fn dispatch_import_style(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::ImportStyle = downcast_payload(payload);
    cmds.trigger(ImportStyleEvent);
}

/// Dispatch a [`gantz_egui::CloseHead`] payload as a [`head::CloseEvent`].
pub(crate) fn dispatch_close_head(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::CloseHead(target) = downcast_payload(payload);
    cmds.trigger(head::CloseEvent(target));
}

/// Dispatch a [`gantz_egui::BranchHead`] payload as a
/// [`head::BranchHeadEvent`].
pub(crate) fn dispatch_branch_head(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    let gantz_egui::BranchHead { original, new_name } = downcast_payload(payload);
    cmds.trigger(head::BranchHeadEvent { original, new_name });
}

/// Dispatch a dropped `.gantz` file as an [`ImportFileEvent`]. A drop onto
/// the graph scene also opens the imported root graph.
pub(crate) fn dispatch_file_drop(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    use gantz_egui::widget::gantz::{FileDrop, FileDropTarget};
    let FileDrop { bytes, target } = downcast_payload(payload);
    let open_head = target == FileDropTarget::GraphScene;
    cmds.trigger(ImportFileEvent { bytes, open_head });
}

/// Dispatch a [`gantz_egui::ResetBaseGraph`] payload as a
/// [`ResetBaseGraphEvent`].
pub(crate) fn dispatch_reset_base_graph(
    _: Option<Entity>,
    payload: DynResponse,
    cmds: &mut Commands,
) {
    let gantz_egui::ResetBaseGraph(name) = downcast_payload(payload);
    cmds.trigger(ResetBaseGraphEvent(name));
}

/// Dispatch an app-level payload as a [`ForApp`] event.
fn dispatch_for_app<T: ResponseData>(_: Option<Entity>, payload: DynResponse, cmds: &mut Commands) {
    cmds.trigger(ForApp(downcast_payload::<T>(payload)));
}
