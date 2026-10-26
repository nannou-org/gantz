//! The window's corner: a row at the pane tree's top-right that holds the
//! wordmark and the window's controls. A pane in the tree's top-right corner
//! sits below the row, so that the controls never read as the pane's own.
//!
//! `egui_tiles` has no way to inset one tile. So [`reserve`] moves the pane
//! into a column under an empty tile of the row's height for the pass that
//! draws the tree, and [`release`] restores the tree before it is stored.

use super::Pane;
use egui_tiles::{Container, Linear, LinearDir, Tile, TileId, Tiles, Tree};

/// A pane moved under the corner's row by [`reserve`].
pub(super) struct Reserved {
    pane: TileId,
    /// The rect of the empty row above the pane.
    pub(super) row: egui::Rect,
}

/// The least height left to a pane below the corner's row.
const MIN_PANE_HEIGHT: f32 = 80.0;

/// The id under which the corner's rect is kept. See [`set_rect`].
const RECT_ID: &str = "gantz-window-corner";

/// The column that holds the corner's row above the pane. Its id lies far
/// above any id `egui_tiles` allocates, and stays the same from pass to pass.
pub(super) fn column_id() -> TileId {
    TileId::from_u64(u64::MAX - 1)
}

/// The empty tile that takes the corner's row.
fn row_id() -> TileId {
    TileId::from_u64(u64::MAX - 2)
}

/// Record the corner's rect this pass, or that it holds nothing.
pub(super) fn set_rect(ctx: &egui::Context, rect: Option<egui::Rect>) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(RECT_ID), rect));
}

/// The corner's rect this pass. See [`set_rect`].
pub(super) fn rect(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<Option<egui::Rect>>(egui::Id::new(RECT_ID)))
        .flatten()
}

/// Move the tile `pane`, at the top-right corner of a tree that fills `area`,
/// into a column below an empty row `height` tall. `gap` is the gap between
/// tiles.
///
/// Returns `None` and leaves the tree as it is when `pane` does not lie in
/// rows and columns from the root, as in a grid, or when too little of it
/// would remain below the row.
pub(super) fn reserve(
    tree: &mut Tree<Pane>,
    pane: TileId,
    area: egui::Rect,
    height: f32,
    gap: f32,
) -> Option<Reserved> {
    let rect = rect_of(&tree.tiles, tree.root()?, area, pane, gap)?;
    let below = rect.height() - gap - height;
    if below < MIN_PANE_HEIGHT {
        return None;
    }
    let (column, row) = (column_id(), row_id());
    let mut linear = Linear::new(LinearDir::Vertical, vec![row, pane]);
    linear.shares.set_share(row, height);
    linear.shares.set_share(pane, below);
    // `rect_of` found the pane through rows and columns, so its parent is one.
    match tree.tiles.parent_of(pane) {
        Some(parent) => replace_child(&mut tree.tiles, parent, pane, column),
        None => tree.root = Some(column),
    }
    let empty = Linear::new(LinearDir::Vertical, vec![]);
    tree.tiles
        .insert(row, Tile::Container(Container::Linear(empty)));
    tree.tiles
        .insert(column, Tile::Container(Container::Linear(linear)));
    let row_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), height));
    Some(Reserved {
        pane,
        row: row_rect,
    })
}

/// Undo [`reserve`]. The pane takes back its place, with any share the column
/// gained from a resize of its parent.
pub(super) fn release(tree: &mut Tree<Pane>, reserved: Reserved) {
    let column = column_id();
    match tree.tiles.parent_of(column) {
        Some(parent) => replace_child(&mut tree.tiles, parent, column, reserved.pane),
        None => {
            if tree.root() == Some(column) {
                tree.root = Some(reserved.pane);
            }
        }
    }
    tree.tiles.remove(column);
    tree.tiles.remove(row_id());
}

/// Put `new` in the place of the child `old` of the container `parent`.
fn replace_child(tiles: &mut Tiles<Pane>, parent: TileId, old: TileId, new: TileId) {
    if let Some(Tile::Container(container)) = tiles.get_mut(parent) {
        super::tiles::replace_child(container, old, new);
    }
}

/// The rect that `egui_tiles` lays the tile `target` out in, when the tile
/// `id` fills `rect`. It follows only rows and columns, as `egui_tiles` lays
/// them out. `None` when `target` lies elsewhere.
fn rect_of(
    tiles: &Tiles<Pane>,
    id: TileId,
    rect: egui::Rect,
    target: TileId,
    gap: f32,
) -> Option<egui::Rect> {
    if id == target {
        return Some(rect);
    }
    let Some(Tile::Container(Container::Linear(linear))) = tiles.get(id) else {
        return None;
    };
    let visible: Vec<TileId> = linear
        .children
        .iter()
        .copied()
        .filter(|&c| tiles.is_visible(c))
        .collect();
    let gaps = gap * visible.len().saturating_sub(1) as f32;
    let (length, mut at) = match linear.dir {
        LinearDir::Horizontal => (rect.width(), rect.min.x),
        LinearDir::Vertical => (rect.height(), rect.min.y),
    };
    let sizes = linear.shares.split(&visible, (length - gaps).max(0.0));
    visible.iter().zip(sizes).find_map(|(&child, size)| {
        let child_rect = match linear.dir {
            LinearDir::Horizontal => egui::Rect::from_min_size(
                egui::pos2(at, rect.min.y),
                egui::vec2(size, rect.height()),
            ),
            LinearDir::Vertical => egui::Rect::from_min_size(
                egui::pos2(rect.min.x, at),
                egui::vec2(rect.width(), size),
            ),
        };
        at += size + gap;
        rect_of(tiles, child, child_rect, target, gap)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sidebar, the scene, and a right column of two panes.
    fn tree() -> (Tree<Pane>, TileId, TileId) {
        let mut tiles = Tiles::default();
        let graphs = tiles.insert_pane(Pane::Graphs);
        let left = tiles.insert_tab_tile(vec![graphs]);
        let scene = tiles.insert_pane(Pane::GraphScene);
        let steel = tiles.insert_pane(Pane::Steel);
        let top = tiles.insert_tab_tile(vec![steel]);
        let logs = tiles.insert_pane(Pane::Logs);
        let bottom = tiles.insert_tab_tile(vec![logs]);
        let right = tiles.insert_vertical_tile(vec![top, bottom]);
        let root = tiles.insert_horizontal_tile(vec![left, scene, right]);
        (Tree::new("corner", root, tiles), top, right)
    }

    fn area() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))
    }

    #[test]
    fn rects_follow_the_shares_and_gaps() {
        let (tree, top, _) = tree();
        let root = tree.root().expect("root");
        let rect = rect_of(&tree.tiles, root, area(), top, 1.0).expect("laid out");
        // Three even columns across 1198 px, then two even rows of 799 px.
        let column = (1200.0 - 2.0) / 3.0;
        assert!((rect.min.x - (2.0 * column + 2.0)).abs() < 1e-3);
        assert!((rect.width() - column).abs() < 1e-3);
        assert_eq!(rect.min.y, 0.0);
        assert!((rect.height() - 399.5).abs() < 1e-3);
    }

    #[test]
    fn reserve_puts_a_row_of_the_height_above_the_pane() {
        let (mut tree, top, right) = tree();
        let reserved = reserve(&mut tree, top, area(), 34.0, 1.0).expect("reserved");
        let root = tree.root().expect("root");
        let row = rect_of(&tree.tiles, root, area(), row_id(), 1.0).expect("row");
        let pane = rect_of(&tree.tiles, root, area(), top, 1.0).expect("pane");
        assert!((row.height() - 34.0).abs() < 1e-3);
        assert!((pane.min.y - 35.0).abs() < 1e-3);
        assert!((pane.max.y - 399.5).abs() < 1e-3);
        release(&mut tree, reserved);
        assert_eq!(tree.tiles.parent_of(top), Some(right));
        assert!(tree.tiles.get(column_id()).is_none());
        assert!(tree.tiles.get(row_id()).is_none());
        let pane = rect_of(&tree.tiles, root, area(), top, 1.0).expect("pane");
        assert!((pane.height() - 399.5).abs() < 1e-3);
    }

    #[test]
    fn reserve_skips_a_pane_too_short_for_the_row() {
        let (mut tree, top, _) = tree();
        let short = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 200.0));
        assert!(reserve(&mut tree, top, short, 34.0, 1.0).is_none());
        assert!(tree.tiles.get(column_id()).is_none());
    }
}
