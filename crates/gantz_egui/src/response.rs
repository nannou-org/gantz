//! Dynamic, typed response data emitted from within the widget tree.
//!
//! Widgets deep within the tree (node UIs via [`NodeCtx`][crate::NodeCtx],
//! the graph scene's context menus, keyboard shortcuts, etc.) cannot mutate
//! application state directly. Instead they emit typed payloads (e.g.
//! [`CreateNode`][crate::CreateNode], [`Paste`][crate::Paste], or any custom
//! type) that are collected into a [`Responses`] and returned from
//! [`Gantz::show`][crate::widget::Gantz::show] as part of its response, for
//! the application to handle after the pass.
//!
//! Payloads are dynamically typed so that nodes defined downstream can emit
//! their own custom types without this crate knowing about them: the
//! application drains the payloads it understands via [`Responses::take`]
//! (or dispatches on [`ResponseData::data_type_id`]) and warns on the rest.

use std::any::{Any, TypeId};

/// A dynamic response payload emitted from within the widget tree.
///
/// Blanket-implemented for any eligible type, so emitting a custom payload
/// requires no trait impl - only `Debug + Send + Sync + 'static`.
pub trait ResponseData: Any + std::fmt::Debug + Send + Sync {
    /// The [`TypeId`] of the concrete payload type.
    fn data_type_id(&self) -> TypeId;

    /// The type name of the concrete payload, for unhandled-payload warnings.
    fn data_type_name(&self) -> &'static str;

    /// Upcast for downcasting to the concrete payload type.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

/// Dynamic response data collected during one widget pass.
///
/// Entries are tagged with the head whose UI emitted them (`None` for
/// app-level emissions like [`ExportAllNamed`][crate::ExportAllNamed]).
#[derive(Debug, Default)]
pub struct Responses {
    entries: Vec<(Option<gantz_ca::Head>, Box<dyn ResponseData>)>,
}

impl<T> ResponseData for T
where
    T: Any + std::fmt::Debug + Send + Sync,
{
    fn data_type_id(&self) -> TypeId {
        TypeId::of::<T>()
    }

    fn data_type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl Responses {
    /// Emit a payload, tagged with the head whose UI emitted it (`None` for
    /// app-level payloads).
    pub fn push<T: ResponseData>(&mut self, head: Option<gantz_ca::Head>, data: T) {
        self.entries.push((head, Box::new(data)));
    }

    /// Merge untagged payloads emitted by a single head's widgets.
    pub fn extend(
        &mut self,
        head: Option<&gantz_ca::Head>,
        data: impl IntoIterator<Item = Box<dyn ResponseData>>,
    ) {
        self.entries
            .extend(data.into_iter().map(|d| (head.cloned(), d)));
    }

    /// Drain all entries of type `T` in order of emission, leaving the rest.
    pub fn take<T: ResponseData>(&mut self) -> Vec<(Option<gantz_ca::Head>, T)> {
        let mut taken = Vec::new();
        let mut rest = Vec::with_capacity(self.entries.len());
        for (head, data) in self.entries.drain(..) {
            if data.data_type_id() == TypeId::of::<T>() {
                let data = data
                    .into_any()
                    .downcast::<T>()
                    .expect("`data_type_id` matched `T`");
                taken.push((head, *data));
            } else {
                rest.push((head, data));
            }
        }
        self.entries = rest;
        taken
    }

    /// Drain all remaining entries in order of emission.
    pub fn drain(
        &mut self,
    ) -> impl Iterator<Item = (Option<gantz_ca::Head>, Box<dyn ResponseData>)> + '_ {
        self.entries.drain(..)
    }

    /// Whether any entries remain.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Type names of the remaining entries, for unhandled-payload warnings.
    pub fn type_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.entries.iter().map(|(_, d)| d.data_type_name())
    }
}
