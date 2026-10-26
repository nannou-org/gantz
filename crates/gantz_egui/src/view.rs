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
///
/// The scene draws at a zoom within [`egui_graph::Graph::DEFAULT_ZOOM_RANGE`]
/// even when its `scene_rect` asks for more, as a center view on a small
/// graph does. The camera takes the same clamped zoom, so that what is drawn
/// from the camera, such as the ground's dots, matches the scene.
pub fn restore_egui(view: &mut SceneView, egui_view: egui_graph::View, viewport: egui::Vec2) {
    let mut camera = Camera::from_scene_rect(egui_view.scene_rect, viewport);
    camera.zoom = egui_graph::Graph::DEFAULT_ZOOM_RANGE.clamp(camera.zoom);
    view.camera = camera;
    view.layout = from_egui_layout(egui_view.layout);
}

/// Convert an `egui_graph` layout, such as an auto-layout result.
pub fn from_egui_layout(layout: egui_graph::Layout) -> Layout {
    layout
        .into_iter()
        .map(|(egui_graph::NodeId(id), pos)| (NodeId(id), pos))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_clamps_the_zoom_to_what_the_scene_draws() {
        let viewport = egui::vec2(800.0, 600.0);
        let mut view = SceneView::default();
        // A center view on one small node asks for a zoom of 8.
        let egui_view = egui_graph::View {
            scene_rect: egui::Rect::from_center_size(egui::pos2(10.0, 20.0), viewport / 8.0),
            layout: Default::default(),
        };
        restore_egui(&mut view, egui_view, viewport);
        assert_eq!(view.camera.zoom, egui_graph::Graph::DEFAULT_ZOOM_RANGE.max);
        assert_eq!(view.camera.center, egui::pos2(10.0, 20.0));
    }
}
