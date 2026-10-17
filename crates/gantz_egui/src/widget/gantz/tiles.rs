//! The panes and the tile tree that lays them out, including its sizing.

use super::state::{GantzState, ViewToggles};
use super::{Gantz, LogSource};
use crate::{
    HeadAccess,
    widget::{self},
};
use gantz_core::node;

/// A pane within the outer tree.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Pane {
    /// An application-supplied pane, identified by its provider's stable key.
    /// See [`ExtPane::key`][widget::ExtPane::key]. Renders a placeholder
    /// while no provider supplies the key.
    Ext(String),
    GraphConfig,
    /// Contains the inner graph tree with all open graph tabs.
    GraphScene,
    Graphs,
    GuiPerf,
    /// The focused head's GUI, rendered live through the
    /// [`ui_tree`][crate::ui_tree] interpreter from its `gui` marker tree.
    GuiPreview,
    /// The focused head's stored `gui` marker tree as scheme text, with its
    /// decode warnings.
    GuiTree,
    History,
    Logs,
    NodeInspector,
    /// A node detached from a graph via the "open view" action, rendered via
    /// [`NodeUi::view_ui`] for monitoring. Unlike the singleton variants it
    /// carries data, so any number can be opened and freely placed anywhere
    /// in the top-level tree.
    ///
    /// [`NodeUi::view_ui`]: crate::NodeUi::view_ui
    NodeView(NodeViewPane),
    /// Globally relevant configuration grouped into subtabs.
    Settings,
    Steel,
    VmPerf,
}

/// The payload of a [`Pane::NodeView`]. One detached node view, identified by
/// its `head` and `path` within that head's graph. `ty_name` is the node's
/// type name from [`NodeUi::name`], cached at open time so the tab title is
/// stable while the head is closed. The node type never changes. Only its
/// index does, and `migrate_node_view_paths` migrates that.
///
/// [`NodeUi::name`]: crate::NodeUi::name
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct NodeViewPane {
    pub(super) head: gantz_ca::Head,
    pub(super) path: Vec<node::Id>,
    pub(super) ty_name: String,
}

/// A node view pane over the graph `head`, for tests.
#[cfg(test)]
pub(super) fn node_view(head: &str, path: &[node::Id]) -> Pane {
    Pane::NodeView(NodeViewPane {
        head: gantz_ca::Head::Branch(head.parse().unwrap()),
        path: path.to_vec(),
        ty_name: "plot".to_string(),
    })
}

/// Migrate the [`Pane::NodeView`] panes for `head` after a node removal
/// reindexed its graph. This mirrors the state, layout and selection
/// migration in [`crate::ops::remove_nodes`]. A view whose node was removed
/// is dropped. A view whose node was swapped to a new index has its path
/// rewritten. This keeps a detached view pointing at the same node across
/// deletions, with no staleness guard. Operates on the top-level `tree`,
/// where node views live as tiles.
pub(super) fn migrate_node_view_paths(
    tree: &mut egui_tiles::Tree<Pane>,
    head: &gantz_ca::Head,
    reindex: &crate::ops::Reindex,
) {
    if reindex.is_empty() {
        return;
    }
    let mut to_remove = Vec::new();
    for (id, tile) in tree.tiles.iter_mut() {
        let egui_tiles::Tile::Pane(Pane::NodeView(pane)) = tile else {
            continue;
        };
        if pane.head != *head {
            continue;
        }
        // Root-level node views have a single-element path.
        let [ix] = pane.path[..] else { continue };
        match reindex.apply_to_index(ix) {
            Some(new_ix) => pane.path = vec![new_ix],
            None => to_remove.push(*id),
        }
    }
    for id in to_remove {
        tree.tiles.remove(id);
    }
}

/// The tab title for a node-view pane. `<head>:<path>` with the final path
/// segment rendered as `<index>-<ty_name>`, for example `main:3-plot`.
/// Intermediate segments are shown as raw indices.
fn node_view_title(pane: &NodeViewPane) -> String {
    use std::fmt::Write;
    let mut s = format!("{}", pane.head);
    let last = pane.path.len().saturating_sub(1);
    for (i, seg) in pane.path.iter().enumerate() {
        if i == last {
            let _ = write!(s, ":{seg}-{}", pane.ty_name);
        } else {
            let _ = write!(s, ":{seg}");
        }
    }
    s
}

/// The display title for a pane, shared by the tab bar, floating windows and
/// the `windowed_panes` report. Panes tied to the focused head suffix it,
/// for example `Steel - main`.
pub(super) fn pane_title<Access>(
    gantz: &Gantz<'_>,
    access: &Access,
    focused_head: usize,
    pane: &Pane,
) -> String
where
    Access: HeadAccess,
{
    let with_head = |label: &str| match access.heads().get(focused_head) {
        Some(head) => format!("{label} - {head}"),
        None => label.to_string(),
    };
    match pane {
        Pane::Ext(key) => match gantz.ext_panes.iter().find(|p| p.key() == *key) {
            Some(p) => with_head(p.title()),
            None => key.clone(),
        },
        Pane::GraphConfig => with_head("Graph"),
        Pane::GraphScene => "Graphs".to_string(),
        Pane::Graphs => "Graphs".to_string(),
        Pane::GuiPerf => "GUI Perf".to_string(),
        Pane::History => "History".to_string(),
        Pane::Settings => "Settings".to_string(),
        Pane::Logs => match gantz.log_source {
            None => "Logs (No Source)".to_string(),
            Some(LogSource::Logger(_)) => "Logs".to_string(),
            #[cfg(feature = "tracing")]
            Some(LogSource::TraceCapture(..)) => "Tracing".to_string(),
        },
        Pane::NodeInspector => with_head("Node Inspector"),
        Pane::NodeView(p) => node_view_title(p),
        Pane::Steel => with_head("Steel"),
        Pane::GuiPreview => with_head("GUI Preview"),
        Pane::GuiTree => with_head("GUI Tree"),
        Pane::VmPerf => with_head("VM Perf"),
    }
}

/// Create the initial layout of the tree of tiles.
///
/// Roughly something like this:
///
/// ```text
/// -----------------------------------------
/// |grs/hist/settings |scene               |
/// |------------------|                     |
/// |vm/gui            |                     |
/// |------------------|---------------------|
/// |conf              |logs      |steel     |
/// |------------------|          |          |
/// |insp              |          |          |
/// -----------------------------------------
/// ```
///
/// The active tab of each tab container defaults to its first child. See
/// `egui_tiles::Tabs::new`. Child ordering picks the default tabs.
pub(super) fn create_tree() -> egui_tiles::Tree<Pane> {
    let mut tiles = egui_tiles::Tiles::default();

    // The leaf panes. The GUI Preview and GUI Tree panes are not created
    // here. They join the tray via `sync_singleton_pane`, the same path that
    // serves persisted trees predating them.
    let graph_config = tiles.insert_pane(Pane::GraphConfig);
    let graph_scene = tiles.insert_pane(Pane::GraphScene);
    let graphs = tiles.insert_pane(Pane::Graphs);
    let gui_perf = tiles.insert_pane(Pane::GuiPerf);
    let history = tiles.insert_pane(Pane::History);
    let logs = tiles.insert_pane(Pane::Logs);
    let node_inspector = tiles.insert_pane(Pane::NodeInspector);
    let settings = tiles.insert_pane(Pane::Settings);
    let steel = tiles.insert_pane(Pane::Steel);
    let vm_perf = tiles.insert_pane(Pane::VmPerf);

    // Sidebar tab containers. The first child is the default-active tab.
    let graphs_history_settings = tiles.insert_tab_tile(vec![graphs, history, settings]);
    // VM Perf and GUI Perf sit side by side rather than as tabs, so both plots
    // are visible at once.
    let perf = tiles.insert_horizontal_tile(vec![vm_perf, gui_perf]);

    // The sidebar column.
    let mut shares = egui_tiles::Shares::default();
    shares.set_share(graphs_history_settings, 0.30);
    shares.set_share(perf, 0.05);
    shares.set_share(graph_config, 0.13);
    shares.set_share(node_inspector, 0.25);
    let left_column = tiles.insert_container(egui_tiles::Linear {
        children: vec![graphs_history_settings, perf, graph_config, node_inspector],
        dir: egui_tiles::LinearDir::Vertical,
        shares,
    });

    // Logs and steel code in bottom "tray".
    let tray = tiles.insert_horizontal_tile(vec![logs, steel]);

    // The right column with the graph scene above the tray.
    let right_column = tiles.insert_container(egui_tiles::Linear::new_binary(
        egui_tiles::LinearDir::Vertical,
        [graph_scene, tray],
        0.7,
    ));

    // The root with both columns. The split here is only a fallback. The
    // sidebar normally has a fixed pixel width maintained across window
    // resizes. See `impose_fixed_sizes` and `default_sidebar_width`.
    let root = tiles.insert_container(egui_tiles::Linear::new_binary(
        egui_tiles::LinearDir::Horizontal,
        [left_column, right_column],
        0.18,
    ));

    egui_tiles::Tree::new("gantz-tiles-tree", root, tiles)
}

/// Insert a [`Pane::NodeView`] for `(head, path)` into the top-level `tree`, or
/// activate the existing one. Views are deduped by head and path. `ty_name`
/// is the node's type name used for the tab title. New views land in the
/// tray, a layout-safe default that is opaque to the fixed-size anchors. The
/// user can then drag them anywhere in the tree.
pub(super) fn add_node_view_pane(
    tree: &mut egui_tiles::Tree<Pane>,
    head: gantz_ca::Head,
    path: Vec<node::Id>,
    ty_name: String,
) {
    // If a view for this head and path already exists, focus it.
    let existing = tree.tiles.iter().find_map(|(id, tile)| match tile {
        egui_tiles::Tile::Pane(Pane::NodeView(p)) if p.head == head && p.path == path => Some(*id),
        _ => None,
    });
    if let Some(id) = existing {
        tree.make_active(|tile_id, _| tile_id == id);
        return;
    }
    let pane = Pane::NodeView(NodeViewPane {
        head,
        path,
        ty_name,
    });
    insert_tray_pane(tree, pane);
}

/// Insert a new tile for `pane`, defaulting to the tray. If the layout is no
/// longer canonical, for example the tray was moved into a column beside the
/// graph scene, dock beside the graph scene so the pane joins the main area.
/// Never fall back to the root, since its split holds the fixed sidebar.
fn insert_tray_pane(tree: &mut egui_tiles::Tree<Pane>, pane: Pane) {
    let pane_id = tree.tiles.insert_pane(pane);
    let container = layout_anchors(tree)
        .map(|a| a.tray)
        .or_else(|| {
            tree.tiles
                .find_pane(&Pane::GraphScene)
                .and_then(|scene| tree.tiles.parent_of(scene))
        })
        .or_else(|| tree.root());
    match container {
        Some(c) => {
            // Match the siblings' share units. A container can hold
            // pixel-sized shares, so the default `1.0` share would leave the
            // new pane a sliver next to them.
            let share = sibling_share(tree, c);
            tree.move_tile_to_container(pane_id, c, usize::MAX, true);
            if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(l))) =
                tree.tiles.get_mut(c)
            {
                l.shares.set_share(pane_id, share);
            }
        }
        None => tree.root = Some(pane_id),
    }
}

/// The average share of a container's current children, or `1.0` when it has
/// none. Used to give an inserted pane a starting size in the same units as
/// its siblings.
fn sibling_share(tree: &egui_tiles::Tree<Pane>, container: egui_tiles::TileId) -> f32 {
    let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(l))) =
        tree.tiles.get(container)
    else {
        return 1.0;
    };
    let children: Vec<_> = l
        .children
        .iter()
        .copied()
        .filter(|&c| tree.is_visible(c))
        .collect();
    if children.is_empty() {
        return 1.0;
    }
    children.iter().map(|&c| l.shares[c]).sum::<f32>() / children.len() as f32
}

/// Ensure a [`Pane::Ext`] tile exists for each supplied provider key, adding
/// missing ones to the tray. They stay hidden until toggled, like Logs and
/// Steel. Tiles whose provider is absent are left in place. They render a
/// placeholder and keep their spot in the layout for when the provider
/// returns.
pub(super) fn sync_ext_panes(tree: &mut egui_tiles::Tree<Pane>, keys: &[&str]) {
    for &key in keys {
        let exists = tree
            .tiles
            .iter()
            .any(|(_, tile)| matches!(tile, egui_tiles::Tile::Pane(Pane::Ext(k)) if k == key));
        if !exists {
            insert_tray_pane(tree, Pane::Ext(key.to_string()));
        }
    }
}

/// Ensure a tile exists for the given singleton pane, adding it to the tray
/// when missing. A tree persisted before the pane existed lacks it.
pub(super) fn sync_singleton_pane(tree: &mut egui_tiles::Tree<Pane>, pane: Pane) {
    let exists = tree
        .tiles
        .iter()
        .any(|(_, tile)| matches!(tile, egui_tiles::Tile::Pane(p) if *p == pane));
    if !exists {
        insert_tray_pane(tree, pane);
    }
}

/// All panes should have tab bars besides the main graph scene.
///
/// In the case that a tile is being dragged, even the graph scene should show a
/// tab bar in case the user wants to add a tab there.
pub(super) fn simplify_tree(tree: &mut egui_tiles::Tree<Pane>, ctx: &egui::Context) {
    // Default options, but ensure panes have tabs.
    tree.simplify(&egui_tiles::SimplificationOptions {
        all_panes_must_have_tabs: true,
        ..Default::default()
    });
    // If a tile is being dragged, show all tab bars.
    if tree.dragged_id(ctx).is_some() {
        return;
    }
    let Some(graph_scene_id) = tree.tiles.find_pane(&Pane::GraphScene) else {
        return;
    };
    // Find its parent. This must be `Tabs` after the `simplify` pass above.
    let Some(parent_id) = tree.tiles.parent_of(graph_scene_id) else {
        return;
    };
    // If the parent has one child, replace it with the graph scene.
    let Some(parent) = tree.tiles.get_container(parent_id) else {
        return;
    };
    if parent.num_children() == 1 {
        tree.tiles.remove(graph_scene_id);
        tree.tiles
            .insert(parent_id, egui_tiles::Tile::Pane(Pane::GraphScene));
    }
}

/// The gap between sibling tiles, in points. It must match the default
/// `egui_tiles::Behavior::gap_width`, which is not overridden, so imposed
/// pixel sizes are exact and do not drift when re-imposed each frame.
const TILE_GAP: f32 = 1.0;

/// The minimum sidebar width and tray height, in points.
const MIN_PANE_SIZE: f32 = 80.0;

/// The default minimum visible size of a pane, in points. Small enough not to
/// disturb the default layout, large enough that a pane never collapses to a
/// sliver.
const DEFAULT_PANE_MIN_SIZE: f32 = 32.0;

/// The minimum width of the log pane, in points. The log table needs room for
/// its time, level and message columns.
const LOG_MIN_WIDTH: f32 = 200.0;

/// The minimum visible size of `pane`, in points.
pub(super) fn pane_min_size(pane: &Pane) -> egui::Vec2 {
    match pane {
        Pane::Logs => egui::vec2(LOG_MIN_WIDTH, DEFAULT_PANE_MIN_SIZE),
        Pane::GraphScene => egui::vec2(200.0, 150.0),
        _ => egui::Vec2::splat(DEFAULT_PANE_MIN_SIZE),
    }
}

/// The tiles whose Linear share splits hold the sidebar width and tray height,
/// when the tree has its default top-level shape.
struct LayoutAnchors {
    /// The root horizontal Linear, `[left_column | right_column]`.
    pub(super) root: egui_tiles::TileId,
    pub(super) left_column: egui_tiles::TileId,
    /// The right column vertical Linear, `[graph_scene / tray]`.
    pub(super) right_column: egui_tiles::TileId,
    pub(super) graph_scene: egui_tiles::TileId,
    pub(super) tray: egui_tiles::TileId,
}

/// Identify the layout anchors, or `None` if the tree is not in its default
/// top-level shape, for example mid-drag or after the user rearranged panes.
/// In that case the proportional layout is left untouched.
fn layout_anchors(tree: &egui_tiles::Tree<Pane>) -> Option<LayoutAnchors> {
    let graph_scene = tree.tiles.find_pane(&Pane::GraphScene)?;
    let right_column = tree.tiles.parent_of(graph_scene)?;
    let root = tree.root()?;
    let &[a, b] = linear_children(tree, root, egui_tiles::LinearDir::Horizontal)?.as_slice() else {
        return None;
    };
    let left_column = match (a == right_column, b == right_column) {
        (false, true) => a,
        (true, false) => b,
        _ => return None,
    };
    let &[c, d] = linear_children(tree, right_column, egui_tiles::LinearDir::Vertical)?.as_slice()
    else {
        return None;
    };
    let tray = match (c == graph_scene, d == graph_scene) {
        (true, false) => d,
        (false, true) => c,
        _ => return None,
    };
    Some(LayoutAnchors {
        root,
        left_column,
        right_column,
        graph_scene,
        tray,
    })
}

/// The children of `id` if it is a Linear container with direction `dir`.
fn linear_children(
    tree: &egui_tiles::Tree<Pane>,
    id: egui_tiles::TileId,
    dir: egui_tiles::LinearDir,
) -> Option<Vec<egui_tiles::TileId>> {
    match tree.tiles.get_container(id)? {
        egui_tiles::Container::Linear(l) if l.dir == dir => Some(l.children.clone()),
        _ => None,
    }
}

/// Set the two shares of a binary Linear container.
fn set_linear_shares(
    tree: &mut egui_tiles::Tree<Pane>,
    container: egui_tiles::TileId,
    a: egui_tiles::TileId,
    a_share: f32,
    b: egui_tiles::TileId,
    b_share: f32,
) {
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(l))) =
        tree.tiles.get_mut(container)
    {
        l.shares.set_share(a, a_share);
        l.shares.set_share(b, b_share);
    }
}

/// Impose the stored sidebar width and tray height in points on the tree's
/// share splits so they stay fixed as the window resizes. Call after
/// `simplify_tree` and before `tree.ui`.
pub(super) fn impose_fixed_sizes(
    tree: &mut egui_tiles::Tree<Pane>,
    state: &GantzState,
    area: egui::Rect,
) {
    let Some(anchors) = layout_anchors(tree) else {
        return;
    };
    // Both columns span the full height, so the tray's available height is the
    // area height less the gap. The sidebar's available width likewise.
    if state.view_toggles.sidebar_open {
        let avail = area.width() - TILE_GAP;
        let width = state
            .sidebar_width
            .clamp(MIN_PANE_SIZE, (avail - MIN_PANE_SIZE).max(MIN_PANE_SIZE));
        set_linear_shares(
            tree,
            anchors.root,
            anchors.left_column,
            width,
            anchors.right_column,
            (avail - width).max(1.0),
        );
    }
    if state.view_toggles.logs
        || state.view_toggles.steel
        || state.view_toggles.gui_preview
        || state.view_toggles.gui_tree
    {
        let avail = area.height() - TILE_GAP;
        let height = state
            .tray_height
            .clamp(MIN_PANE_SIZE, (avail - MIN_PANE_SIZE).max(MIN_PANE_SIZE));
        set_linear_shares(
            tree,
            anchors.right_column,
            anchors.graph_scene,
            (avail - height).max(1.0),
            anchors.tray,
            height,
        );
    }
}

/// Capture the sidebar width and tray height in points from the laid-out
/// tree, so they can be re-imposed next frame, including after manual divider
/// drags. Call after `tree.ui`.
///
/// This reads the post-layout shares rather than the cached rects. A resize
/// drag updates the shares during `tree.ui`, but the rects it computes
/// reflect the pre-drag split, so reading rects would never see the drag.
pub(super) fn capture_fixed_sizes(
    tree: &egui_tiles::Tree<Pane>,
    state: &mut GantzState,
    area: egui::Rect,
) {
    let Some(anchors) = layout_anchors(tree) else {
        return;
    };
    // Gate on the laid-out visibility, not `sidebar_open`. The hamburger can
    // flip `sidebar_open` mid-frame, but `set_tile_visibility` only runs at the
    // frame start, so the layout and the captured share reflect the
    // visibility from frame start. Capturing against a stale layout would
    // compute the column's size against the wrong set of visible siblings.
    if tree.is_visible(anchors.left_column) {
        if let Some(width) = linear_child_points(
            tree,
            anchors.root,
            anchors.left_column,
            area.width() - TILE_GAP,
        ) {
            if width > 1.0 {
                state.sidebar_width = width;
            }
        }
    }
    if tree.is_visible(anchors.tray) {
        if let Some(height) = linear_child_points(
            tree,
            anchors.right_column,
            anchors.tray,
            area.height() - TILE_GAP,
        ) {
            if height > 1.0 {
                state.tray_height = height;
            }
        }
    }
}

/// The points a Linear child currently occupies, derived from its share of the
/// visible children, mirroring `egui_tiles::Shares::split`.
fn linear_child_points(
    tree: &egui_tiles::Tree<Pane>,
    container: egui_tiles::TileId,
    child: egui_tiles::TileId,
    available: f32,
) -> Option<f32> {
    let egui_tiles::Container::Linear(l) = tree.tiles.get_container(container)? else {
        return None;
    };
    let total: f32 = l
        .children
        .iter()
        .filter(|&&c| tree.is_visible(c))
        .map(|&c| l.shares[c])
        .sum();
    (total > 0.0).then(|| available * l.shares[child] / total)
}

/// The minimum extent of `id` along `dir`, in points.
///
/// A linear container sums its children along `dir` and takes the largest
/// across it, gaps included. Tabs report the active child, since only that
/// child is laid out. Grids are not created by gantz, so they report no
/// minimum.
fn min_extent(
    tree: &egui_tiles::Tree<Pane>,
    id: egui_tiles::TileId,
    dir: egui_tiles::LinearDir,
    gap: f32,
) -> f32 {
    let Some(tile) = tree.tiles.get(id) else {
        return 0.0;
    };
    match tile {
        egui_tiles::Tile::Pane(pane) => {
            let min = pane_min_size(pane);
            match dir {
                egui_tiles::LinearDir::Horizontal => min.x,
                egui_tiles::LinearDir::Vertical => min.y,
            }
        }
        egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs)) => tabs
            .next_active(&tree.tiles)
            .map(|child| min_extent(tree, child, dir, gap))
            .unwrap_or(0.0),
        egui_tiles::Tile::Container(egui_tiles::Container::Linear(l)) => {
            let children: Vec<_> = l
                .children
                .iter()
                .copied()
                .filter(|&c| tree.is_visible(c))
                .collect();
            if children.is_empty() {
                return 0.0;
            }
            let extents: Vec<f32> = children
                .iter()
                .map(|&c| min_extent(tree, c, dir, gap))
                .collect();
            if l.dir == dir {
                extents.iter().sum::<f32>() + gap * (children.len() as f32 - 1.0)
            } else {
                extents.into_iter().fold(0.0, f32::max)
            }
        }
        egui_tiles::Tile::Container(egui_tiles::Container::Grid(_)) => 0.0,
    }
}

/// Ensure every visible pane keeps at least its minimum size.
///
/// `egui_tiles` only enforces a minimum while a divider is dragged. A pane can
/// still be laid out with a tiny share, for example a pane appended to a
/// container whose other children hold pixel-sized shares. This pass walks the
/// tree, recomputes each container's inner rect from the current shares and
/// raises any child below its minimum by taking the difference from siblings
/// that have spare. Call before `tree.ui` and `impose_fixed_sizes`.
///
/// A container whose children already meet their minimums is left untouched,
/// so proportional containers keep scaling with the window. A container too
/// small to fit every minimum splits proportionally to the minimums, so the
/// result still sums to the available extent.
pub(super) fn impose_min_sizes(tree: &mut egui_tiles::Tree<Pane>, gap: f32, area: egui::Rect) {
    if let Some(root) = tree.root() {
        impose_min_sizes_tile(tree, gap, root, area);
    }
}

fn impose_min_sizes_tile(
    tree: &mut egui_tiles::Tree<Pane>,
    gap: f32,
    id: egui_tiles::TileId,
    rect: egui::Rect,
) {
    enum Next {
        None,
        Linear,
        Tabs(egui_tiles::TileId),
    }
    let next = match tree.tiles.get(id) {
        Some(egui_tiles::Tile::Pane(_)) | None => Next::None,
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) => {
            match tabs.next_active(&tree.tiles) {
                Some(child) => Next::Tabs(child),
                None => Next::None,
            }
        }
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(_))) => Next::Linear,
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Grid(_))) => Next::None,
    };
    match next {
        Next::None => {}
        Next::Linear => impose_linear_min(tree, gap, id, rect),
        Next::Tabs(child) => impose_min_sizes_tile(tree, gap, child, rect),
    }
}

fn impose_linear_min(
    tree: &mut egui_tiles::Tree<Pane>,
    gap: f32,
    id: egui_tiles::TileId,
    rect: egui::Rect,
) {
    // Read the direction, visible children and their points from the current
    // shares. The borrow ends before the tree is mutated.
    let (dir, children, points, available) = match tree.tiles.get(id) {
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(l))) => {
            let children: Vec<_> = l
                .children
                .iter()
                .copied()
                .filter(|&c| tree.is_visible(c))
                .collect();
            let axis_extent = match l.dir {
                egui_tiles::LinearDir::Horizontal => rect.width(),
                egui_tiles::LinearDir::Vertical => rect.height(),
            };
            let available = (axis_extent - gap * (children.len() as f32 - 1.0)).max(0.0);
            let points = l.shares.split(&children, available);
            (l.dir, children, points, available)
        }
        _ => return,
    };
    if children.is_empty() {
        return;
    }

    let mins: Vec<f32> = children
        .iter()
        .map(|&c| min_extent(tree, c, dir, gap))
        .collect();

    let min_sum: f32 = mins.iter().sum();
    let deficit: f32 = points
        .iter()
        .zip(&mins)
        .map(|(p, m)| (m - p).max(0.0))
        .sum();
    let undersized = min_sum > available || deficit > f32::EPSILON;

    let new_points: Vec<f32> = if min_sum > available {
        let denom = if min_sum > 0.0 { min_sum } else { 1.0 };
        mins.iter().map(|&m| available * m / denom).collect()
    } else {
        let slack: f32 = points
            .iter()
            .zip(&mins)
            .map(|(p, m)| (p - m).max(0.0))
            .sum();
        let take = if slack > 0.0 { deficit / slack } else { 0.0 };
        points
            .iter()
            .zip(&mins)
            .map(|(p, m)| if p < m { *m } else { *p - (*p - *m) * take })
            .collect()
    };

    if undersized {
        if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(l))) =
            tree.tiles.get_mut(id)
        {
            for (&child, &share) in children.iter().zip(&new_points) {
                l.shares.set_share(child, share);
            }
        }
    }

    // Recurse with the laid-out rects.
    let mut cursor = match dir {
        egui_tiles::LinearDir::Horizontal => rect.min.x,
        egui_tiles::LinearDir::Vertical => rect.min.y,
    };
    for (&child, &extent) in children.iter().zip(&new_points) {
        let child_rect = match dir {
            egui_tiles::LinearDir::Horizontal => egui::Rect::from_min_size(
                egui::pos2(cursor, rect.min.y),
                egui::vec2(extent, rect.height()),
            ),
            egui_tiles::LinearDir::Vertical => egui::Rect::from_min_size(
                egui::pos2(rect.min.x, cursor),
                egui::vec2(rect.width(), extent),
            ),
        };
        impose_min_sizes_tile(tree, gap, child, child_rect);
        cursor += extent + gap;
    }
}

/// The supplied extension panes' checkbox entries. Both pane-toggle UIs
/// render from this single source, so a new pane cannot appear in one and
/// not the other.
pub(super) fn ext_pane_entries(gantz: &Gantz) -> Vec<widget::ExtPaneEntry> {
    gantz
        .ext_panes
        .iter()
        .map(|p| widget::ExtPaneEntry {
            key: p.key().to_string(),
            title: p.title().to_string(),
            description: p.description().to_string(),
        })
        .collect()
}

/// Whether a tab's pane can be hidden via its right-click menu. The main graph
/// scene is not hideable. Node views are closed and removed, not hidden.
pub(super) fn pane_is_hideable(pane: &Pane) -> bool {
    !matches!(pane, Pane::GraphScene | Pane::NodeView(_))
}

/// Set a pane's visibility toggle. No-op for panes without one.
pub(super) fn set_pane_visible(view: &mut ViewToggles, pane: &Pane, visible: bool) {
    match pane {
        Pane::Ext(key) => {
            view.ext.insert(key.clone(), visible);
        }
        Pane::Graphs => view.graphs = visible,
        Pane::History => view.history = visible,
        Pane::Settings => view.settings = visible,
        Pane::GraphConfig => view.graph_config = visible,
        Pane::NodeInspector => view.node_inspector = visible,
        Pane::VmPerf => view.perf_vm = visible,
        Pane::GuiPerf => view.perf_gui = visible,
        Pane::Logs => view.logs = visible,
        Pane::Steel => view.steel = visible,
        Pane::GuiPreview => view.gui_preview = visible,
        Pane::GuiTree => view.gui_tree = visible,
        // No visibility toggle. The scene is always visible and node views
        // are closable.
        Pane::GraphScene | Pane::NodeView(_) => {}
    }
}

/// Whether a pane's visibility toggle is currently on. Panes without a toggle
/// are always considered visible.
pub(super) fn pane_is_visible(view: &ViewToggles, pane: &Pane) -> bool {
    match pane {
        Pane::Ext(key) => view.ext.get(key).copied().unwrap_or(false),
        Pane::Graphs => view.graphs,
        Pane::History => view.history,
        Pane::Settings => view.settings,
        Pane::GraphConfig => view.graph_config,
        Pane::NodeInspector => view.node_inspector,
        Pane::VmPerf => view.perf_vm,
        Pane::GuiPerf => view.perf_gui,
        Pane::Logs => view.logs,
        Pane::Steel => view.steel,
        Pane::GuiPreview => view.gui_preview,
        Pane::GuiTree => view.gui_tree,
        Pane::GraphScene | Pane::NodeView(_) => true,
    }
}

/// Whether a pane may be popped out into a window. Every pane but the graph
/// scene qualifies, since it hosts the inner graph tile tree.
pub(super) fn pane_is_poppable(pane: &Pane) -> bool {
    !matches!(pane, Pane::GraphScene)
}

/// Ensure the view toggles match the pane visibility.
pub(super) fn set_tile_visibility(tree: &mut egui_tiles::Tree<Pane>, view: &ViewToggles) {
    let ids: Vec<_> = tree.tiles.tile_ids().collect();
    let open = view.sidebar_open;
    // Sidebar panes are gated by both the sidebar being open and their
    // individual toggle. The tray panes are independent of the sidebar.
    for &id in &ids {
        if let Some(pane) = tree.tiles.get_pane(&id) {
            match pane {
                Pane::GraphScene => (),
                Pane::Ext(key) => tree.set_visible(id, view.ext.get(key).copied().unwrap_or(false)),
                Pane::Settings => tree.set_visible(id, open && view.settings),
                Pane::GraphConfig => tree.set_visible(id, open && view.graph_config),
                Pane::Graphs => tree.set_visible(id, open && view.graphs),
                Pane::GuiPerf => tree.set_visible(id, open && view.perf_gui),
                Pane::History => tree.set_visible(id, open && view.history),
                Pane::NodeInspector => tree.set_visible(id, open && view.node_inspector),
                Pane::VmPerf => tree.set_visible(id, open && view.perf_vm),
                Pane::Logs => tree.set_visible(id, view.logs),
                Pane::Steel => tree.set_visible(id, view.steel),
                Pane::GuiPreview => tree.set_visible(id, view.gui_preview),
                Pane::GuiTree => tree.set_visible(id, view.gui_tree),
                // Always visible. A node view is removed by closing, not hiding.
                Pane::NodeView(_) => tree.set_visible(id, true),
            }
        }
    }
    // A container is visible when any child is.
    for &id in &ids {
        if let Some(container) = tree.tiles.get_container(id) {
            let has_visible_child = container.children().any(|&id| tree.is_visible(id));
            tree.set_visible(id, has_visible_child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixed sidebar width must be imposed on the sidebar, not the main
    /// area. The anchors must identify the sidebar as the column that does
    /// not contain the graph scene.
    #[test]
    fn impose_sets_sidebar_width_on_left_column() {
        let mut tree = create_tree();
        let anchors = layout_anchors(&tree).expect("default tree has layout anchors");

        let graph_scene = tree.tiles.find_pane(&Pane::GraphScene).unwrap();
        assert_ne!(anchors.left_column, anchors.right_column);
        assert_eq!(
            tree.tiles.parent_of(graph_scene),
            Some(anchors.right_column)
        );

        let mut state = GantzState::new();
        state.view_toggles.sidebar_open = true;
        state.sidebar_width = 240.0;
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        impose_fixed_sizes(&mut tree, &state, area);

        let Some(egui_tiles::Container::Linear(root)) = tree.tiles.get_container(anchors.root)
        else {
            panic!("root is not a linear container");
        };
        let avail = 1000.0 - TILE_GAP;
        // The sidebar gets the fixed width. The main area gets the remainder.
        assert!((root.shares[anchors.left_column] - 240.0).abs() < 0.01);
        assert!((root.shares[anchors.right_column] - (avail - 240.0)).abs() < 0.01);
    }

    /// `capture_fixed_sizes` must recover the same width `impose_fixed_sizes`
    /// set, so a sidebar that is not dragged does not drift frame to frame.
    #[test]
    fn capture_round_trips_imposed_sidebar_width() {
        let mut tree = create_tree();
        let mut state = GantzState::new();
        state.view_toggles.sidebar_open = true;
        state.sidebar_width = 240.0;
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        impose_fixed_sizes(&mut tree, &state, area);
        capture_fixed_sizes(&tree, &mut state, area);
        assert!((state.sidebar_width - 240.0).abs() < 0.01);
    }

    /// Reopening the sidebar must not inflate its width. On the open-transition
    /// frame `sidebar_open` is already true but the layout still has the left
    /// column hidden. Capturing then would size it against the wrong siblings.
    #[test]
    fn capture_skips_while_sidebar_laid_out_hidden() {
        let mut tree = create_tree();
        let anchors = layout_anchors(&tree).unwrap();
        // The layout has the sidebar hidden, as at frame start.
        tree.set_visible(anchors.left_column, false);
        let mut state = GantzState::new();
        // `sidebar_open` was just toggled on mid-frame.
        state.view_toggles.sidebar_open = true;
        state.sidebar_width = 240.0;
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        capture_fixed_sizes(&tree, &mut state, area);
        assert!((state.sidebar_width - 240.0).abs() < 0.01);
    }

    /// A pane left at the default share next to siblings with pixel-sized
    /// shares used to lay out as a sliver. The minimum pass must raise it.
    #[test]
    fn impose_min_sizes_raises_tiny_pane() {
        let mut tiles = egui_tiles::Tiles::default();
        let left = tiles.insert_pane(Pane::Graphs);
        let right = tiles.insert_pane(Pane::GraphScene);
        // The default share of a newly inserted pane is 1.0.
        let target = tiles.insert_pane(node_view("main", &[1]));
        let mut shares = egui_tiles::Shares::default();
        shares.set_share(left, 270.0);
        shares.set_share(right, 730.0);
        let root = tiles.insert_container(egui_tiles::Linear {
            children: vec![left, right, target],
            dir: egui_tiles::LinearDir::Horizontal,
            shares,
        });
        let mut tree = egui_tiles::Tree::new("test", root, tiles);
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        impose_min_sizes(&mut tree, TILE_GAP, area);

        let Some(egui_tiles::Container::Linear(l)) = tree.tiles.get_container(root) else {
            panic!("root is not a linear container");
        };
        assert!(
            l.shares[target] >= DEFAULT_PANE_MIN_SIZE - 0.01,
            "target share {}",
            l.shares[target]
        );
    }

    /// The log pane has a larger minimum width than the default, so its table
    /// stays readable when the tray is narrow.
    #[test]
    fn impose_min_sizes_keeps_log_width() {
        let mut tree = create_tree();
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 800.0));
        impose_min_sizes(&mut tree, TILE_GAP, area);

        let logs = tree.tiles.find_pane(&Pane::Logs).unwrap();
        let parent = tree.tiles.parent_of(logs).unwrap();
        let Some(egui_tiles::Container::Linear(l)) = tree.tiles.get_container(parent) else {
            panic!("the tray is not a linear container");
        };
        assert!(
            l.shares[logs] >= LOG_MIN_WIDTH - 0.01,
            "log share {}",
            l.shares[logs]
        );
    }

    /// When every pane already meets its minimum, the pass must leave the
    /// shares untouched so proportional containers keep scaling.
    #[test]
    fn impose_min_sizes_leaves_proportional_layout() {
        let mut tree = create_tree();
        let anchors = layout_anchors(&tree).unwrap();
        let Some(egui_tiles::Container::Linear(before)) =
            tree.tiles.get_container(anchors.left_column)
        else {
            panic!("the left column is not a linear container");
        };
        let before = before.shares.clone();

        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        impose_min_sizes(&mut tree, TILE_GAP, area);

        let Some(egui_tiles::Container::Linear(after)) =
            tree.tiles.get_container(anchors.left_column)
        else {
            panic!("the left column is not a linear container");
        };
        assert_eq!(before, after.shares);
    }

    /// Breaking the canonical layout used to make a new pane fall back to the
    /// root and land as a sliver. It must dock beside the graph scene instead.
    #[test]
    fn insert_tray_pane_avoids_root_when_anchors_break() {
        let mut tree = create_tree();
        let anchors = layout_anchors(&tree).unwrap();
        // Simulate moving the sole visible tray pane to a column beside the
        // graph scene. The tray loses its canonical two-child shape.
        let steel = tree.tiles.find_pane(&Pane::Steel).unwrap();
        tree.set_visible(steel, false);
        let logs = tree.tiles.find_pane(&Pane::Logs).unwrap();
        tree.move_tile_to_container(logs, anchors.right_column, 0, true);
        assert!(layout_anchors(&tree).is_none());

        let pane = node_view("main", &[1]);
        insert_tray_pane(&mut tree, pane.clone());

        let id = tree
            .tiles
            .iter()
            .find_map(|(id, tile)| match tile {
                egui_tiles::Tile::Pane(p) if *p == pane => Some(*id),
                _ => None,
            })
            .expect("new pane is in the tree");
        assert_eq!(tree.tiles.parent_of(id), Some(anchors.right_column));
        assert_ne!(tree.tiles.parent_of(id), tree.root());
    }
}
