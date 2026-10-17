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

/// The fallback position for the `i`th node a view migration could not map.
/// Cascades down-right from the camera `center` so unplaced nodes stay
/// visible without overlapping.
pub fn cascade_pos(center: Pos2, i: usize) -> Pos2 {
    center + emath::vec2(20.0, 20.0) * i as f32
}

/// Build a view for a navigation target commit that has no stored view.
/// Carries the live view's node positions forward through the navigation
/// node-identity `matching` from old index to new index. Keeps the live
/// camera.
///
/// Nodes of the new graph absent from the matching are placed in a cascade
/// from the camera centre. Nodes merged in from a peer are one example. An
/// empty live layout, or a matching that carries no positions at all, yields
/// an empty result. The scene's one-shot auto-layout is the right treatment
/// for a genuinely layoutless graph such as a join placeholder, not a cascade
/// of every node.
pub fn carry_layout(
    live: &SceneView,
    matching: &gantz_ca::Matching,
    new_node_count: usize,
) -> SceneView {
    let pairs = matching.iter().map(|(&old, &new)| (old, new));
    let mut view = SceneView {
        camera: live.camera,
        layout: remap(&live.layout, pairs).collect(),
    };
    cascade_unplaced(&mut view, new_node_count);
    view
}

/// Build a view for a headlessly minted merge commit from the two parent
/// tips' stored views. Each merged node takes its position from the first
/// tip's view where it survives there. Otherwise it falls back to the second
/// tip's view, then to a cascade from the camera centre. This is the same
/// fallback as `apply_merge_migration`. The first tip is ours and the second
/// is theirs. The camera comes from whichever side has a view, preferring
/// the first.
///
/// When neither side has a view, or no position carries over at all, the
/// result is empty. Callers must not store empty-layout views, as an
/// adopting peer would destructively auto-layout them.
pub fn merged_view(
    node_srcs: &[gantz_ca::merge::NodeSrc],
    first_view: Option<&SceneView>,
    second_view: Option<&SceneView>,
) -> SceneView {
    let camera = first_view
        .or(second_view)
        .map(|v| v.camera)
        .unwrap_or_default();
    let mut view = SceneView {
        camera,
        layout: Default::default(),
    };
    // Theirs first, so that ours replaces it where both sides hold the node.
    let srcs = node_srcs.iter().enumerate();
    if let Some(v) = second_view {
        let pairs = srcs.clone().filter_map(|(m, src)| Some((src.theirs?, m)));
        view.layout.extend(remap(&v.layout, pairs));
    }
    if let Some(v) = first_view {
        let pairs = srcs.filter_map(|(m, src)| Some((src.ours?, m)));
        view.layout.extend(remap(&v.layout, pairs));
    }
    cascade_unplaced(&mut view, node_srcs.len());
    view
}

/// Place each of the first `node_count` nodes that `view` has no position for
/// in a cascade from the camera centre. A view with no positions at all stays
/// empty.
fn cascade_unplaced(view: &mut SceneView, node_count: usize) {
    if view.layout.is_empty() {
        return;
    }
    let unplaced: Vec<NodeId> = (0..node_count)
        .map(NodeId::from_index)
        .filter(|id| !view.layout.contains_key(id))
        .collect();
    for (i, id) in unplaced.into_iter().enumerate() {
        view.layout.insert(id, cascade_pos(view.camera.center, i));
    }
}

/// Commit the current layout as a new commit on the head's existing graph
/// when node positions have changed since the head commit's frozen baseline
/// view. Advances `head` to the new commit.
///
/// The graph content is unchanged, so the new commit reuses the head's
/// [`gantz_ca::GraphAddr`]. The registry dedups the graph and never calls the
/// `graph` closure passed to [`gantz_ca::Registry::commit_graph_to_head`].
/// The VM does not need to recompile. Only the `layout` node positions are
/// compared. The `camera` is excluded, so camera pan and zoom never produce a
/// layout commit.
///
/// Returns the new commit address when a layout commit was created. Returns
/// `None` when there is no baseline view yet or no node-position change. A
/// missing baseline means the head commit's view section entry has not been
/// seeded. Seeding the new commit's view, clearing the redo stack and
/// migrating GUI state stay with the caller.
pub fn commit_layout(
    registry: &mut gantz_ca::Registry,
    timestamp: gantz_ca::Timestamp,
    head: &mut gantz_ca::Head,
    live: &SceneView,
) -> Option<gantz_ca::CommitAddr> {
    let head_commit_ca = registry.head_commit_ca(head)?;
    let baseline = crate::section::view(registry, &head_commit_ca)?;
    if baseline.layout == live.layout {
        return None;
    }
    let graph_addr = registry.commits().get(&head_commit_ca)?.graph;
    Some(registry.commit_graph_to_head(
        timestamp,
        graph_addr,
        || unreachable!("layout commit reuses an existing graph"),
        head,
    ))
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

    // carry_layout maps live positions through the navigation matching, keeps
    // the camera, and cascades unmapped new nodes from the camera centre.
    #[test]
    fn carry_layout_remaps_and_places_new_nodes() {
        let mut live = SceneView::default();
        live.camera.center = pos2(100.0, 50.0);
        live.layout.insert(NodeId::from_index(0), pos2(1.0, 1.0));
        live.layout.insert(NodeId::from_index(1), pos2(2.0, 2.0));
        live.layout.insert(NodeId::from_index(2), pos2(3.0, 3.0));

        // A swap-style remap. Live node 0 moves to 1 and live node 2 moves to
        // 0. Live node 1 was removed. The new graph has an extra node at index
        // 2 with no provenance.
        let matching: gantz_ca::Matching = [(0, 1), (2, 0)].into_iter().collect();
        let view = carry_layout(&live, &matching, 3);

        assert_eq!(view.camera, live.camera);
        assert_eq!(view.layout.len(), 3);
        assert_eq!(
            view.layout.get(&NodeId::from_index(1)).copied(),
            Some(pos2(1.0, 1.0))
        );
        assert_eq!(
            view.layout.get(&NodeId::from_index(0)).copied(),
            Some(pos2(3.0, 3.0))
        );
        // The unmapped new node cascades from the camera centre.
        assert_eq!(
            view.layout.get(&NodeId::from_index(2)).copied(),
            Some(pos2(100.0, 50.0))
        );
    }

    // When nothing carries over, the result stays empty rather than cascading
    // every node from the camera centre. The scene's one-shot auto-layout is
    // the right treatment for a genuinely layoutless graph.
    #[test]
    fn carry_layout_without_carried_positions_yields_empty() {
        let mut live = SceneView::default();
        live.layout.insert(NodeId::from_index(5), pos2(1.0, 1.0));
        let cases = [
            (
                "empty live layout",
                SceneView::default(),
                [(0, 0)].into_iter().collect(),
                4,
            ),
            (
                "matching carries no positions",
                live,
                gantz_ca::Matching::new(),
                3,
            ),
        ];
        for (case, live, matching, new_node_count) in cases {
            let view = carry_layout(&live, &matching, new_node_count);
            assert!(view.layout.is_empty(), "{case}");
        }
    }

    // merged_view sources each merged node's position from the first tip's
    // view where it survives there, falling back to the second tip's, then
    // cascading from the camera centre. The camera prefers the first side.
    #[test]
    fn merged_view_sources_ours_then_theirs_then_cascade() {
        let src = |ours: Option<usize>, theirs: Option<usize>| gantz_ca::merge::NodeSrc {
            base: None,
            ours,
            theirs,
        };
        let mut first = SceneView::default();
        first.camera.center = pos2(10.0, 10.0);
        first.layout.insert(NodeId::from_index(0), pos2(1.0, 0.0));
        let mut second = SceneView::default();
        second.layout.insert(NodeId::from_index(0), pos2(2.0, 0.0));
        second.layout.insert(NodeId::from_index(1), pos2(3.0, 0.0));

        // Merged node 0 exists on both sides, so ours wins. Node 1 is
        // theirs-only. Node 2 has no stored position anywhere.
        let srcs = [
            src(Some(0), Some(0)),
            src(None, Some(1)),
            src(Some(9), None),
        ];
        let view = merged_view(&srcs, Some(&first), Some(&second));

        assert_eq!(view.camera, first.camera);
        assert_eq!(
            view.layout.get(&NodeId::from_index(0)).copied(),
            Some(pos2(1.0, 0.0))
        );
        assert_eq!(
            view.layout.get(&NodeId::from_index(1)).copied(),
            Some(pos2(3.0, 0.0))
        );
        // The positionless node cascades from the camera centre.
        assert_eq!(
            view.layout.get(&NodeId::from_index(2)).copied(),
            Some(pos2(10.0, 10.0))
        );
    }

    // Without a stored view on either side, merged_view yields an empty
    // layout. Callers must not store empty-layout views.
    #[test]
    fn merged_view_no_views_yields_empty() {
        let srcs = [gantz_ca::merge::NodeSrc {
            base: None,
            ours: Some(0),
            theirs: None,
        }];
        let view = merged_view(&srcs, None, None);
        assert!(view.layout.is_empty());
    }
}
