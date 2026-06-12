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
//! (or dispatches on [`Payload::type_id`]) and warns on the rest.

use std::any::{Any, TypeId};

/// A dynamic response payload emitted from within the widget tree.
///
/// Blanket-implemented for any eligible type, so emitting a custom payload
/// requires no trait impl - only `Debug + Send + Sync + 'static`.
pub trait ResponseData: Any + std::fmt::Debug + Send + Sync {
    /// Upcast for downcasting to the concrete payload type.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

/// A type-erased response payload.
///
/// The concrete type's identity is captured at construction, where the type
/// is statically known. This matters because `Box<dyn ResponseData>` itself
/// satisfies the [`ResponseData`] blanket impl: identity queried through a
/// box via method calls would resolve to the *box's* impl and report the
/// box's `TypeId` rather than the payload's, silently breaking dispatch.
#[derive(Debug)]
pub struct Payload {
    type_id: TypeId,
    type_name: &'static str,
    data: Box<dyn ResponseData>,
}

/// Dynamic response data collected during one widget pass.
///
/// Entries are tagged with the head whose UI emitted them (`None` for
/// app-level emissions like [`ExportAllNamed`][crate::ExportAllNamed]).
#[derive(Debug, Default)]
pub struct Responses {
    entries: Vec<(Option<gantz_ca::Head>, Payload)>,
}

impl Payload {
    /// Erase a concrete payload, capturing its type identity.
    pub fn new<T: ResponseData>(data: T) -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
            data: Box::new(data),
        }
    }

    /// The `TypeId` of the concrete payload type.
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// The type name of the concrete payload, for unhandled-payload warnings.
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// Downcast to the concrete payload type.
    pub fn downcast<T: ResponseData>(self) -> Result<T, Self> {
        if self.type_id == TypeId::of::<T>() {
            let data = self
                .data
                .into_any()
                .downcast::<T>()
                .expect("`type_id` matches `T`");
            Ok(*data)
        } else {
            Err(self)
        }
    }
}

impl Responses {
    /// Emit a payload, tagged with the head whose UI emitted it (`None` for
    /// app-level payloads).
    pub fn push<T: ResponseData>(&mut self, head: Option<gantz_ca::Head>, data: T) {
        self.entries.push((head, Payload::new(data)));
    }

    /// Merge payloads emitted by a single head's widgets.
    pub fn extend(
        &mut self,
        head: Option<&gantz_ca::Head>,
        data: impl IntoIterator<Item = Payload>,
    ) {
        self.entries
            .extend(data.into_iter().map(|d| (head.cloned(), d)));
    }

    /// Drain all entries of type `T` in order of emission, leaving the rest.
    pub fn take<T: ResponseData>(&mut self) -> Vec<(Option<gantz_ca::Head>, T)> {
        let mut taken = Vec::new();
        let mut rest = Vec::with_capacity(self.entries.len());
        for (head, payload) in self.entries.drain(..) {
            match payload.downcast::<T>() {
                Ok(data) => taken.push((head, data)),
                Err(payload) => rest.push((head, payload)),
            }
        }
        self.entries = rest;
        taken
    }

    /// Drain all remaining entries in order of emission.
    pub fn drain(&mut self) -> impl Iterator<Item = (Option<gantz_ca::Head>, Payload)> + '_ {
        self.entries.drain(..)
    }

    /// Whether any entries remain.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Type names of the remaining entries, for unhandled-payload warnings.
    pub fn type_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.entries.iter().map(|(_, p)| p.type_name())
    }
}

impl<T> ResponseData for T
where
    T: Any + std::fmt::Debug + Send + Sync,
{
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct A(u32);

    #[derive(Debug, PartialEq)]
    struct B(&'static str);

    /// Identity must be the concrete payload's, never the erased box's (the
    /// box itself satisfies the `ResponseData` blanket impl).
    #[test]
    fn payload_identity_is_the_concrete_type() {
        let p = Payload::new(A(1));
        assert_eq!(p.type_id(), TypeId::of::<A>());
        assert_eq!(p.type_name(), std::any::type_name::<A>());
        assert_eq!(p.downcast::<A>().unwrap(), A(1));
    }

    #[test]
    fn downcast_to_wrong_type_returns_the_payload() {
        let p = Payload::new(A(1));
        let p = p.downcast::<B>().unwrap_err();
        assert_eq!(p.downcast::<A>().unwrap(), A(1));
    }

    #[test]
    fn take_drains_matching_entries_in_order() {
        let mut rs = Responses::default();
        rs.push(None, A(1));
        rs.push(None, B("x"));
        rs.push(None, A(2));
        let taken: Vec<_> = rs.take::<A>().into_iter().map(|(_, a)| a).collect();
        assert_eq!(taken, vec![A(1), A(2)]);
        let names: Vec<_> = rs.type_names().collect();
        assert_eq!(names, vec![std::any::type_name::<B>()]);
        rs.take::<B>();
        assert!(rs.is_empty());
    }

    #[test]
    fn extend_tags_payloads_with_the_head() {
        let head = gantz_ca::Head::Branch("main".to_string());
        let mut rs = Responses::default();
        rs.extend(Some(&head), [Payload::new(A(1))]);
        let mut taken = rs.take::<A>();
        assert_eq!(taken.len(), 1);
        let (tag, a) = taken.pop().unwrap();
        assert_eq!(tag.as_ref(), Some(&head));
        assert_eq!(a, A(1));
    }

    #[test]
    fn drained_payloads_dispatch_by_concrete_type_id() {
        let mut rs = Responses::default();
        rs.push(None, A(1));
        rs.push(None, B("x"));
        let ids: Vec<_> = rs.drain().map(|(_, p)| p.type_id()).collect();
        assert_eq!(ids, vec![TypeId::of::<A>(), TypeId::of::<B>()]);
    }
}
