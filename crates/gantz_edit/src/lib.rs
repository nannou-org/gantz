//! The GUI-independent editing model of gantz.
//!
//! A GUI and a headless peer share this model, so both edit a registry in
//! the same way.
//!
//! - [`view`] holds the viewport-independent scene view and the layout ops
//!   that carry it between commits.
//! - [`section`] declares the registry metadata sections, such as
//!   descriptions, demos and views.
//! - [`codec`] converts between a node set's typed nodes and their stored
//!   form. [`format`](mod@format) reads and writes the `.gantz` text over a codec,
//!   [`export`] exports and imports named graphs and copied subgraphs, and
//!   [`base`] loads the base sources that domains embed.
//! - [`NamedRef`] references a graph by name. [`resync`] keeps every
//!   reference current, and [`cycle`] refuses references that would form a
//!   cycle.

#[doc(hidden)]
pub use gantz_ca;
#[doc(hidden)]
pub use gantz_core;
#[doc(hidden)]
pub use gantz_format;
#[doc(hidden)]
pub use gantz_nodetag;
pub use named_ref::NamedRef;
pub use view::{Camera, Layout, NodeId, SceneView};

pub mod base;
pub mod codec;
pub mod cycle;
pub mod export;
pub mod format;
pub mod named_ref;
pub mod resync;
pub mod section;
#[cfg(test)]
mod test_node;
pub mod view;
