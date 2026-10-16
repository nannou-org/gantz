//! The GUI-independent editing model of gantz.
//!
//! A GUI and a headless peer share this model, so both edit a registry in
//! the same way. The [`view`] module holds the viewport-independent scene
//! view that rides the registry with each commit.

pub use view::{Camera, Layout, NodeId, SceneView};

pub mod view;
