//! The chrome of the pane tree: rounded pane grounds over the scene, the
//! focused pane's outline, and the buttons in a pane's header.
//!
//! The scene is the ground of the whole window. Each pane is a rounded tile
//! in the pane colour on top of it, so the gaps between panes are the scene
//! showing through. Dragging a gap resizes the panes on either side, and
//! dragging a header moves its pane.

use super::Pane;
use crate::style::{self, Role, StyleConfig};

/// The height of a pane's header, where its tabs or its title sit.
pub(super) const HEADER_HEIGHT: f32 = 34.0;

/// The space before the first tab or title of a pane header.
pub(super) const HEADER_INSET: f32 = 8.0;

/// The id under which the focused pane's tile is kept.
const FOCUS_ID: &str = "gantz-pane-focus";

/// An action from a pane header's buttons, applied once the tree is drawn.
pub(super) enum PaneAction {
    Hide(Pane),
    PopOut(egui_tiles::TileId, Pane),
}

/// A painted icon for a header button.
#[derive(Clone, Copy)]
pub(super) enum Icon {
    /// Pop the pane out into its own window.
    PopOut,
    /// Hide the pane.
    Hide,
}

/// The tiles drawn as panes: each visible tabs container, and each pane that
/// is not in one. The scene is the ground, so it and the container holding it
/// are left out.
pub(super) fn grounds(tree: &egui_tiles::Tree<Pane>) -> Vec<egui_tiles::TileId> {
    let mut out = vec![];
    if let Some(root) = tree.root() {
        collect_grounds(&tree.tiles, root, &mut out);
    }
    out
}

/// The shapes under the pane tree. The scene colour fills `area`, and each
/// pane in [`grounds`] is a rounded tile in the pane colour. The focused pane
/// takes a quiet outline.
pub(super) fn ground_shapes(
    tree: &egui_tiles::Tree<Pane>,
    cfg: &StyleConfig,
    theme: egui::Theme,
    area: egui::Rect,
    focus: Option<egui_tiles::TileId>,
) -> Vec<egui::Shape> {
    let scene = cfg.resolve(theme, Role::Scene);
    let pane = cfg.resolve(theme, Role::Panel);
    let border = cfg.resolve(theme, Role::Border);
    let weak = cfg.resolve(theme, Role::WeakText);
    let radius = cfg.shape.window_corner();
    let mut shapes = vec![egui::Shape::rect_filled(area, 0.0, scene)];
    for id in grounds(tree) {
        let Some(rect) = tree.tiles.rect(id) else {
            continue;
        };
        let stroke = match (focus == Some(id), cfg.shape.pane_outline > 0.0) {
            (true, _) => egui::Stroke::new(cfg.shape.pane_outline.max(1.5), weak),
            (false, true) => egui::Stroke::new(cfg.shape.pane_outline, border),
            (false, false) => egui::Stroke::NONE,
        };
        shapes.push(egui::Shape::Rect(egui::epaint::RectShape::new(
            rect,
            radius,
            pane,
            stroke,
            egui::StrokeKind::Inside,
        )));
    }
    shapes
}

/// The focused pane's tile. A press inside a pane focuses it, and a press on
/// the scene clears the focus.
pub(super) fn update_focus(
    ctx: &egui::Context,
    tree: &egui_tiles::Tree<Pane>,
    area: egui::Rect,
) -> Option<egui_tiles::TileId> {
    let id = egui::Id::new(FOCUS_ID);
    let pressed = ctx.input(|i| {
        i.pointer
            .primary_pressed()
            .then(|| i.pointer.interact_pos())
            .flatten()
    });
    if let Some(pos) = pressed.filter(|pos| area.contains(*pos)) {
        let hit = grounds(tree)
            .into_iter()
            .find(|&t| tree.tiles.rect(t).is_some_and(|r| r.contains(pos)));
        ctx.data_mut(|d| {
            if let Some(t) = hit {
                d.insert_temp(id, t);
            } else {
                d.remove::<egui_tiles::TileId>(id);
            }
        });
    }
    ctx.data(|d| d.get_temp(id))
}

/// A small icon button for a pane header.
pub(super) fn icon_button(ui: &mut egui::Ui, icon: Icon, hint: &str) -> egui::Response {
    let side = ui.spacing().interact_size.y;
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let color = match resp.hovered() {
            true => visuals.strong_text_color(),
            false => visuals.weak_text_color(),
        };
        if resp.hovered() {
            let radius = style::shape(ui.ctx()).widget_corner(side);
            ui.painter()
                .rect_filled(rect, radius, visuals.widgets.hovered.weak_bg_fill);
        }
        paint_icon(ui.painter(), rect.shrink(side * 0.3), icon, color);
    }
    resp.on_hover_text(hint)
}

/// Paint `icon` within `rect`.
pub(super) fn paint_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: Icon,
    color: egui::Color32,
) {
    let stroke = egui::Stroke::new(1.5f32, color);
    match icon {
        Icon::PopOut => {
            // A box with an arrow leaving its top-right corner.
            let r = rect;
            let mid = r.center();
            let corner = egui::pos2(r.right(), r.top());
            let box_pts = vec![
                egui::pos2(mid.x, r.top()),
                egui::pos2(r.left(), r.top()),
                egui::pos2(r.left(), r.bottom()),
                egui::pos2(r.right(), r.bottom()),
                egui::pos2(r.right(), mid.y),
            ];
            painter.add(egui::Shape::line(box_pts, stroke));
            painter.line_segment([mid, corner], stroke);
            let head = r.width() * 0.4;
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(corner.x - head, corner.y),
                    corner,
                    egui::pos2(corner.x, corner.y + head),
                ],
                stroke,
            ));
        }
        Icon::Hide => {
            let y = rect.center().y;
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                stroke,
            );
        }
    }
}

fn collect_grounds(
    tiles: &egui_tiles::Tiles<Pane>,
    id: egui_tiles::TileId,
    out: &mut Vec<egui_tiles::TileId>,
) {
    if !tiles.is_visible(id) {
        return;
    }
    match tiles.get(id) {
        Some(egui_tiles::Tile::Pane(Pane::GraphScene)) | None => {}
        Some(egui_tiles::Tile::Pane(_)) => out.push(id),
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) => {
            let scene = tabs.children.iter().any(|c| {
                matches!(
                    tiles.get(*c),
                    Some(egui_tiles::Tile::Pane(Pane::GraphScene))
                )
            });
            if !scene {
                out.push(id);
            }
        }
        Some(egui_tiles::Tile::Container(container)) => {
            for &child in container.children() {
                collect_grounds(tiles, child, out);
            }
        }
    }
}

/// For each pane in a tabs container, the number of visible tabs in its
/// header and whether it is the first of them.
///
/// Taken before the tree is drawn, since `egui_tiles` lifts a container out
/// of the tree while drawing it.
pub(super) fn headers(
    tiles: &egui_tiles::Tiles<Pane>,
) -> std::collections::HashMap<egui_tiles::TileId, (usize, bool)> {
    let mut out = std::collections::HashMap::new();
    for (_, tile) in tiles.iter() {
        let egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs)) = tile else {
            continue;
        };
        let visible: Vec<_> = tabs
            .children
            .iter()
            .copied()
            .filter(|&c| tiles.is_visible(c))
            .collect();
        for (i, &child) in visible.iter().enumerate() {
            out.insert(child, (visible.len(), i == 0));
        }
    }
    out
}
