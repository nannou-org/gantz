//! The GUI-independent editing model of gantz.
//!
//! A GUI and a headless peer share this model, so both edit a registry in
//! the same way.
//!
//! - [`view`] holds the viewport-independent scene view and the layout ops
//!   that carry it between commits.
//! - [`section`] declares the registry metadata sections, such as
//!   descriptions, demos and views.
//! - [`NamedRef`] references a graph by name. [`resync`] keeps every
//!   reference current, and [`cycle`] refuses references that would form a
//!   cycle.

pub use named_ref::NamedRef;
pub use view::{Camera, Layout, NodeId, SceneView};

pub mod cycle;
pub mod named_ref;
pub mod resync;
pub mod section;
pub mod view;
