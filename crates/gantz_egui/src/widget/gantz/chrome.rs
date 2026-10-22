//! The chrome of the pane tree: rounded pane grounds over the scene, the
//! focused pane's outline, and the buttons in a pane's header.
//!
//! The scene is the ground of the whole window. Each pane is a rounded tile
//! in the pane colour on top of it, so the gaps between panes are the scene
//! showing through. Dragging a gap resizes the panes on either side, and
//! dragging a header moves its pane.

use super::Pane;
use crate::style::{self, GraphPaneStyle, Role, StyleConfig};

/// The radius of a ground dot, in points.
const DOT_RADIUS: f32 = 1.0;

/// The least gap between ground dots, in points. A zoomed-out grid doubles
/// its step until its dots are at least this far apart.
const MIN_DOT_GAP: f32 = 10.0;

/// The height of a pane's header, where its tabs or its title sit.
pub(super) const HEADER_HEIGHT: f32 = 34.0;

/// The space before the first tab or title of a pane header.
pub(super) const HEADER_INSET: f32 = 8.0;

/// The margin between a graph pane's edge and the scene inset in it.
pub(super) const GRAPH_INSET: f32 = 6.0;

/// The id under which the focused pane's tile is kept.
const FOCUS_ID: &str = "gantz-pane-focus";

/// The id under which the pane tree's bounds are kept. See [`set_bounds`].
const BOUNDS_ID: &str = "gantz-pane-bounds";

/// An action from a pane header's buttons, applied once the tree is drawn.
pub(super) enum PaneAction {
    Hide(Pane),
    PopOut(egui_tiles::TileId, Pane),
    /// Show a graph pane's graph on the scene. See [`Icon::Scene`].
    ShowOnScene(egui_tiles::TileId, u64),
}

/// Where a graph scene sat this pass and how its camera mapped graph space
/// onto it, so that its grid can be dotted across the ground. See
/// [`record_ground`].
#[derive(Clone, Copy)]
pub(super) struct Ground {
    /// The pass in which the scene was drawn.
    pass: u64,
    /// The scene's rect on the screen.
    rect: egui::Rect,
    camera: gantz_edit::Camera,
    grid: super::GridConfig,
}

/// A painted icon for a header button.
#[derive(Clone, Copy)]
pub(super) enum Icon {
    /// Pop the pane out into its own window.
    PopOut,
    /// Hide the pane.
    Hide,
    /// Show a graph pane's graph on the scene, in place of the graph there.
    Scene,
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

/// The shapes under the pane tree. The scene colour fills `area`, with the
/// main scene's grid dotted across it, so the scene seems to run on under
/// the panes, the gaps and the window margin. Each pane in [`grounds`] is a
/// rounded tile in the pane colour, and a graph pane's inset carries its own
/// scene's grid. The focused pane takes a quiet outline.
pub(super) fn ground_shapes(
    ctx: &egui::Context,
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
    let dot = style::over(cfg.resolve(theme, Role::Dot), scene);
    let radius = cfg.shape.window_corner();
    let mut shapes = vec![egui::Shape::rect_filled(area, 0.0, scene)];
    if let Some(ground) = ground_of(ctx, main_scene_id()) {
        dots(&mut shapes, area, &ground, dot, |_| true);
    }
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
        if let Some((inset, pane_id)) = graph_inset(tree, id, rect, cfg.shape.graph_panes) {
            let inset_radius = (radius - GRAPH_INSET).max(0.0);
            shapes.push(egui::Shape::rect_filled(inset, inset_radius, scene));
            if let Some(ground) = ground_of(ctx, graph_pane_scene_id(pane_id)) {
                let within = |p| in_rounded_rect(inset, inset_radius, p);
                dots(&mut shapes, inset, &ground, dot, within);
            }
        }
    }
    shapes
}

/// The scene inset of a ground whose active pane is a graph pane, and the
/// graph pane's id. A framed pane insets the scene below its header. An open
/// pane runs the scene up under its header.
fn graph_inset(
    tree: &egui_tiles::Tree<Pane>,
    id: egui_tiles::TileId,
    rect: egui::Rect,
    style: GraphPaneStyle,
) -> Option<(egui::Rect, u64)> {
    let tile_id = match tree.tiles.get(id)? {
        egui_tiles::Tile::Pane(Pane::Graph(_)) => id,
        egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs)) => tabs.active?,
        _ => return None,
    };
    let Some(Pane::Graph(pane_id)) = tree.tiles.get_pane(&tile_id) else {
        return None;
    };
    let body = tree.tiles.rect(tile_id)?;
    let inset = match style {
        GraphPaneStyle::Framed => body,
        GraphPaneStyle::Open => body.with_min_y(rect.top()),
    };
    Some((inset.shrink(GRAPH_INSET), *pane_id))
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
        Icon::Scene => {
            // A frame around a filled core: the graph fills the scene.
            let radius = rect.width() * 0.25;
            painter.rect_stroke(rect, radius, stroke, egui::StrokeKind::Middle);
            painter.rect_filled(rect.shrink(rect.width() * 0.3), radius * 0.4, color);
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

/// The mark that stands in for the hidden panes while patch-only is on, with
/// its top-left corner at `pos`. A dot in `problem`'s colour shows there are
/// problems in the log. Returns whether the mark was clicked.
pub(super) fn patch_mark(
    ctx: &egui::Context,
    pos: egui::Pos2,
    problem: Option<egui::Color32>,
) -> bool {
    egui::Area::new(egui::Id::new("gantz-patch-mark"))
        .order(egui::Order::Middle)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            let side = ui.spacing().interact_size.y;
            let (rect, resp) =
                ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::click());
            let visuals = ui.visuals();
            let color = match resp.hovered() {
                true => visuals.text_color(),
                false => visuals.weak_text_color().gamma_multiply(0.6),
            };
            let mark = crate::logo::mark(side * 0.7, color);
            let size = mark.size().unwrap_or(egui::Vec2::splat(side * 0.7));
            mark.paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
            if let Some(color) = problem {
                let dot = rect.right_top() + egui::vec2(-2.0, 3.0);
                ui.painter().circle_filled(dot, 3.0, color);
            }
            let hint = match crate::keybind::shortcut_text(ctx, crate::Action::TogglePatchOnly) {
                Some(keys) => format!("show the panes ({keys})"),
                None => "show the panes".to_string(),
            };
            resp.on_hover_text(hint).clicked()
        })
        .inner
}

/// The id of the main graph's scene.
pub(super) fn main_scene_id() -> egui::Id {
    egui::Id::new("gantz-main-scene")
}

/// The id of the scene in the graph pane `id`.
pub(super) fn graph_pane_scene_id(id: u64) -> egui::Id {
    egui::Id::new(("gantz-graph-pane", id))
}

/// Record that the scene `id` sat in `rect` this pass, seen through `camera`
/// with `grid`. [`ground_shapes`] dots its grid across the ground.
pub(super) fn record_ground(
    ctx: &egui::Context,
    id: egui::Id,
    rect: egui::Rect,
    camera: gantz_edit::Camera,
    grid: super::GridConfig,
) {
    let pass = ctx.cumulative_pass_nr();
    let ground = Ground {
        pass,
        rect,
        camera,
        grid,
    };
    ctx.data_mut(|d| d.insert_temp(id.with("ground"), ground));
}

/// The ground of the scene `id`, if the scene was drawn this pass.
fn ground_of(ctx: &egui::Context, id: egui::Id) -> Option<Ground> {
    let ground: Ground = ctx.data(|d| d.get_temp(id.with("ground")))?;
    (ground.pass == ctx.cumulative_pass_nr()).then_some(ground)
}

/// Push a dot in `color` for each point of the grid of `ground` that falls in
/// `area` and that `within` keeps.
fn dots(
    shapes: &mut Vec<egui::Shape>,
    area: egui::Rect,
    ground: &Ground,
    color: egui::Color32,
    within: impl Fn(egui::Pos2) -> bool,
) {
    let (camera, center) = (ground.camera, ground.rect.center());
    let valid = |x: f32| x.is_finite() && x > 0.0;
    if !ground.grid.show || !valid(ground.grid.step) || !valid(camera.zoom) {
        return;
    }
    let mut step = ground.grid.step;
    while step * camera.zoom < MIN_DOT_GAP {
        step *= 2.0;
    }
    let to_graph = |p: egui::Pos2| camera.center + (p - center) / camera.zoom;
    let (lo, hi) = (to_graph(area.min), to_graph(area.max));
    let cells = |lo: f32, hi: f32| (lo / step).ceil() as i64..=(hi / step).floor() as i64;
    for i in cells(lo.x, hi.x) {
        for j in cells(lo.y, hi.y) {
            let point = egui::pos2(i as f32 * step, j as f32 * step);
            let p = center + (point - camera.center) * camera.zoom;
            if within(p) {
                shapes.push(egui::Shape::circle_filled(p, DOT_RADIUS, color));
            }
        }
    }
}

/// Whether `p` lies in `rect` with its corners rounded by `radius`.
fn in_rounded_rect(rect: egui::Rect, radius: f32, p: egui::Pos2) -> bool {
    let inner = rect.shrink(radius);
    rect.contains(p) && (inner.clamp(p) - p).length() <= radius
}

/// Record that the pane tree fills `tree` this pass, inside the window's
/// `window` area. See [`reach_window`].
pub(super) fn set_bounds(ctx: &egui::Context, tree: egui::Rect, window: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(BOUNDS_ID), (tree, window)));
}

/// `tile` with each edge that lies on the pane tree's edge moved out to the
/// window's edge, so that the main scene runs on under the window margin and
/// the status gutter. See [`set_bounds`].
pub(super) fn reach_window(ctx: &egui::Context, tile: egui::Rect) -> egui::Rect {
    let Some((tree, window)) =
        ctx.data(|d| d.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new(BOUNDS_ID)))
    else {
        return tile;
    };
    let reach = |edge: f32, tree: f32, window: f32| match (edge - tree).abs() < 0.5 {
        true => window,
        false => edge,
    };
    egui::Rect::from_min_max(
        egui::pos2(
            reach(tile.min.x, tree.min.x, window.min.x),
            reach(tile.min.y, tree.min.y, window.min.y),
        ),
        egui::pos2(
            reach(tile.max.x, tree.max.x, window.max.x),
            reach(tile.max.y, tree.max.y, window.max.y),
        ),
    )
}
