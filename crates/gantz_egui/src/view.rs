//! Viewport-independent camera + layout state for a graph scene.
//!
//! The persisted/runtime currency for a graph's view is [`SceneView`] (a
//! [`Camera`] plus a node [`Layout`](egui_graph::Layout)). It is deliberately
//! *viewport independent*: the camera stores a centre and a zoom factor (screen
//! points per graph unit) rather than a visible rectangle, so the zoom level is
//! preserved across sessions and window sizes. An [`egui_graph::View`] (whose
//! `scene_rect` *is* viewport dependent) is materialised from a `SceneView` only
//! at the `egui_graph` API boundary, where the live viewport size is known - see
//! [`SceneView::take_egui`] / [`SceneView::restore_egui`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A graph scene camera, independent of the viewport size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// The point in graph space at the centre of the view.
    pub center: egui::Pos2,
    /// The zoom factor in screen points per graph unit. `1.0` shows one graph
    /// unit per point (i.e. no scaling).
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: egui::Pos2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera {
    /// The visible [`egui::Rect`] (in graph space) this camera maps to for a
    /// viewport of the given size: a `viewport / zoom` sized rect centred on
    /// [`Camera::center`]. `egui`'s `Scene` fits this rect back into the
    /// viewport, reproducing exactly `zoom` points per graph unit.
    pub fn to_scene_rect(self, viewport: egui::Vec2) -> egui::Rect {
        let zoom = if self.zoom > 0.0 { self.zoom } else { 1.0 };
        egui::Rect::from_center_size(self.center, viewport / zoom)
    }

    /// Recover the camera from a `scene_rect` and the viewport it was fit into.
    /// The inverse of [`Camera::to_scene_rect`] (exact when the rect shares the
    /// viewport's aspect ratio, which holds once `egui`'s `Scene` has fit it).
    pub fn from_scene_rect(rect: egui::Rect, viewport: egui::Vec2) -> Self {
        let size = rect.size();
        let zoom = if size.x > 0.0 && size.y > 0.0 {
            (viewport / size).min_elem()
        } else {
            1.0
        };
        Self {
            center: rect.center(),
            zoom,
        }
    }
}

/// A graph scene's persisted view: a [`Camera`] plus the node
/// [`Layout`](egui_graph::Layout).
///
/// This is the viewport-independent currency stored in memory and on disk. See
/// the [module docs](self).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneView {
    #[serde(default)]
    pub camera: Camera,
    /// Node positions in graph space. Serialised sorted for deterministic output.
    #[serde(default, serialize_with = "serialize_sorted_layout")]
    pub layout: egui_graph::Layout,
}

impl SceneView {
    /// Materialise an [`egui_graph::View`] for a viewport of `viewport` size,
    /// *taking* the layout (left empty) to avoid a per-frame clone. Pair with
    /// [`SceneView::restore_egui`] to write the (mutated) view back.
    pub fn take_egui(&mut self, viewport: egui::Vec2) -> egui_graph::View {
        egui_graph::View {
            scene_rect: self.camera.to_scene_rect(viewport),
            layout: std::mem::take(&mut self.layout),
        }
    }

    /// Write back an [`egui_graph::View`] mutated by the `egui_graph` pass,
    /// recovering the viewport-independent [`Camera`] from its `scene_rect`.
    pub fn restore_egui(&mut self, view: egui_graph::View, viewport: egui::Vec2) {
        self.camera = Camera::from_scene_rect(view.scene_rect, viewport);
        self.layout = view.layout;
    }
}

/// Serialise a [`Layout`](egui_graph::Layout) with deterministically sorted keys
/// (mirrors `egui_graph`'s own sorted-layout serialisation).
fn serialize_sorted_layout<S: serde::Serializer>(
    layout: &egui_graph::Layout,
    s: S,
) -> Result<S::Ok, S::Error> {
    let sorted: BTreeMap<_, _> = layout.iter().collect();
    sorted.serialize(s)
}
