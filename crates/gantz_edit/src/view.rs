//! Viewport-independent camera and layout state for a graph scene.
//!
//! The persisted and runtime currency for a graph's view is [`SceneView`], a
//! [`Camera`] plus a node [`Layout`]. It is viewport independent. The camera
//! stores a centre and a zoom factor in screen points per graph unit rather
//! than a visible rectangle. The zoom level is therefore preserved across
//! sessions and window sizes. A GUI derives the visible rectangle only where
//! the live viewport size is known. See [`Camera::to_scene_rect`].

use emath::{Pos2, Rect, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// The id of a node in a [`Layout`]. It holds the node's index in its graph.
///
/// It serializes as `egui_graph::NodeId` does, so a GUI over `egui_graph`
/// converts between the two with no change to stored views.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize,
)]
pub struct NodeId(pub u64);

/// The position of each node's top-left corner in graph space.
pub type Layout = HashMap<NodeId, Pos2>;

/// A graph scene camera, independent of the viewport size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// The point in graph space at the centre of the view.
    pub center: Pos2,
    /// The zoom factor in screen points per graph unit. `1.0` shows one graph
    /// unit per point, with no scaling.
    pub zoom: f32,
}

/// A graph scene's persisted view, a [`Camera`] plus the node [`Layout`].
///
/// This is the viewport-independent currency stored in memory and on disk. See
/// the [module docs](self).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneView {
    #[serde(default)]
    pub camera: Camera,
    /// Node positions in graph space. Serialised sorted for deterministic output.
    #[serde(default, serialize_with = "serialize_sorted_layout")]
    pub layout: Layout,
}

impl NodeId {
    /// The id of the node at graph index `ix`.
    pub const fn from_index(ix: usize) -> Self {
        Self(ix as u64)
    }
}

impl Camera {
    /// The visible rect in graph space this camera maps to for a viewport of
    /// the given size. It is a `viewport / zoom` sized rect centred on
    /// [`Camera::center`]. Fitting this rect back into the viewport reproduces
    /// exactly `zoom` points per graph unit.
    pub fn to_scene_rect(self, viewport: Vec2) -> Rect {
        let zoom = if self.zoom > 0.0 { self.zoom } else { 1.0 };
        Rect::from_center_size(self.center, viewport / zoom)
    }

    /// Recover the camera from a scene rect and the viewport it was fit into.
    /// This is the inverse of [`Camera::to_scene_rect`]. It is exact when the
    /// rect shares the viewport's aspect ratio, which holds once the rect is
    /// fit to the viewport.
    pub fn from_scene_rect(rect: Rect, viewport: Vec2) -> Self {
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

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: Pos2::ZERO,
            zoom: 1.0,
        }
    }
}

/// The positions in `layout` carried to new node indices. Each pair maps an
/// old node index to a new one. A pair whose old node has no position is
/// skipped.
pub fn remap(
    layout: &Layout,
    pairs: impl IntoIterator<Item = (usize, usize)>,
) -> impl Iterator<Item = (NodeId, Pos2)> {
    pairs.into_iter().filter_map(|(old, new)| {
        let pos = *layout.get(&NodeId::from_index(old))?;
        Some((NodeId::from_index(new), pos))
    })
}

/// Serialise a [`Layout`] with deterministically sorted keys.
fn serialize_sorted_layout<S: serde::Serializer>(layout: &Layout, s: S) -> Result<S::Ok, S::Error> {
    let sorted: BTreeMap<_, _> = layout.iter().collect();
    sorted.serialize(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use emath::pos2;
    use gantz_ca::Datum;

    fn point(x: f64, y: f64) -> Datum {
        Datum::Map(vec![
            ("x".into(), Datum::F64(x)),
            ("y".into(), Datum::F64(y)),
        ])
    }

    // Stored views must keep their encoding. The expected datum is the one
    // that the `egui_graph` layout types produced.
    #[test]
    fn scene_view_datum_is_pinned() {
        let mut view = SceneView {
            camera: Camera {
                center: pos2(1.5, -2.0),
                zoom: 0.75,
            },
            layout: Layout::default(),
        };
        view.layout.insert(NodeId(2), pos2(10.0, 20.5));
        view.layout.insert(NodeId(0), pos2(-3.0, 4.0));
        let expected = Datum::Map(vec![
            (
                "camera".into(),
                Datum::Map(vec![
                    ("center".into(), point(1.5, -2.0)),
                    ("zoom".into(), Datum::F64(0.75)),
                ]),
            ),
            (
                "layout".into(),
                Datum::Map(vec![
                    ("0".into(), point(-3.0, 4.0)),
                    ("2".into(), point(10.0, 20.5)),
                ]),
            ),
        ]);
        let datum = gantz_ca::datum::to_datum(&view).unwrap();
        assert_eq!(datum, expected);
        let decoded: SceneView = gantz_ca::datum::from_datum(datum).unwrap();
        assert_eq!(decoded, view);
    }

    #[test]
    fn camera_scene_rect_round_trips() {
        let camera = Camera {
            center: pos2(3.0, -4.0),
            zoom: 2.0,
        };
        let viewport = emath::vec2(800.0, 600.0);
        let rect = camera.to_scene_rect(viewport);
        assert_eq!(rect.size(), emath::vec2(400.0, 300.0));
        assert_eq!(Camera::from_scene_rect(rect, viewport), camera);
    }

    // Positions follow their pairs. Pairs without an old position drop out.
    #[test]
    fn remap_carries_positions_to_new_indices() {
        let mut layout = Layout::default();
        layout.insert(NodeId(0), pos2(1.0, 1.0));
        layout.insert(NodeId(2), pos2(2.0, 2.0));
        let mut out: Vec<_> = remap(&layout, [(0, 5), (1, 6), (2, 0)]).collect();
        out.sort_by_key(|(id, _)| *id);
        assert_eq!(
            out,
            [(NodeId(0), pos2(2.0, 2.0)), (NodeId(5), pos2(1.0, 1.0))]
        );
    }
}
