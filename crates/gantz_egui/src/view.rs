//! The bridge between a [`SceneView`] and an [`egui_graph::View`].
//!
//! An [`egui_graph::View`] has a viewport-dependent `scene_rect`. It is
//! materialised from a viewport-independent [`SceneView`] only at the
//! `egui_graph` API boundary, where the live viewport size is known.

use gantz_edit::{Camera, Layout, NodeId, SceneView};

/// Materialise an [`egui_graph::View`] for a viewport of `viewport` size.
/// This moves the layout out of `view` and leaves it empty. Pair with
/// [`restore_egui`] to write the mutated view back.
pub fn take_egui(view: &mut SceneView, viewport: egui::Vec2) -> egui_graph::View {
    egui_graph::View {
        scene_rect: view.camera.to_scene_rect(viewport),
        layout: view
            .layout
            .drain()
            .map(|(NodeId(id), pos)| (egui_graph::NodeId(id), pos))
            .collect(),
    }
}

/// Write back an [`egui_graph::View`] mutated by the `egui_graph` pass.
/// Recovers the viewport-independent [`Camera`] from its `scene_rect`.
pub fn restore_egui(view: &mut SceneView, egui_view: egui_graph::View, viewport: egui::Vec2) {
    view.camera = Camera::from_scene_rect(egui_view.scene_rect, viewport);
    view.layout = from_egui_layout(egui_view.layout);
}

/// Convert an `egui_graph` layout, such as an auto-layout result.
pub fn from_egui_layout(layout: egui_graph::Layout) -> Layout {
    layout
        .into_iter()
        .map(|(egui_graph::NodeId(id), pos)| (NodeId(id), pos))
        .collect()
}
