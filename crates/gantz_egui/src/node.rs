//! Provides custom nodes that are commonly useful to egui applications of
//! gantz.
//!
//! Provides new node items, while re-exporting some of the `gantz_core::node`
//! items for convenience.

#[doc(inline)]
pub use gantz_core::node::{Id, state};
pub use graph::NamedGraph;

pub mod graph;
