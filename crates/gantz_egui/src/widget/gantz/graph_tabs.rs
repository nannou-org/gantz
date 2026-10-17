//! The graph scene's tabs, one per open head.

use super::panes::{GRAPH_TAB_BAR_HEIGHT, graph_scene, head_immutable, name_breadcrumb};
use super::{PaneCtx, separator_stroke};
use crate::{
    HeadAccess,
    widget::{self},
};

/// A pane within the inner graph tree. Contains the head that this pane
/// displays.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub(super) struct GraphPane(pub(super) gantz_ca::Head);

/// Point the graph pane of `old` at `new`, so the tab keeps its place.
pub(super) fn migrate_graph_pane(
    tree: &mut egui_tiles::Tree<GraphPane>,
    old: &gantz_ca::Head,
    new: &gantz_ca::Head,
) {
    let pane = tree.tiles.iter_mut().find_map(|(_, tile)| match tile {
        egui_tiles::Tile::Pane(GraphPane(head)) if head == old => Some(head),
        _ => None,
    });
    if let Some(head) = pane {
        *head = new.clone();
    }
}

/// State for editing a tab name via double-click.
#[derive(Clone, Default)]
struct TabEditState {
    /// The tile currently being edited, if any.
    editing_tile_id: Option<egui_tiles::TileId>,
    /// The text being edited.
    edit_text: String,
    /// Whether to request focus on the next frame.
    request_focus: bool,
}

/// The context passed to the inner graph `egui_tiles::Tree` widget. It renders
/// each graph tab through the shared pane context.
pub(super) struct GraphTreeBehaviour<'a, 'c, 's, Access>
where
    Access: HeadAccess,
{
    pub(super) cx: &'a mut PaneCtx<'c, 's, Access>,
    /// Extension-pane toggle entries for the scene's "Panes" context submenu.
    /// See [`ext_pane_entries`].
    pub(super) ext_panes: &'a [widget::ExtPaneEntry],
}

impl<Access> egui_tiles::Behavior<GraphPane> for GraphTreeBehaviour<'_, '_, '_, Access>
where
    Access: HeadAccess,
{
    fn tab_title_for_pane(&mut self, pane: &GraphPane) -> egui::WidgetText {
        let GraphPane(head) = pane;
        head.to_string().into()
    }

    fn tab_bar_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        visuals.panel_fill
    }

    fn resize_stroke(
        &self,
        style: &egui::Style,
        resize_state: egui_tiles::ResizeState,
    ) -> egui::Stroke {
        separator_stroke(self.cx.state.style.separator, style, resize_state)
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        self.cx.state.style.separator.width
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        GRAPH_TAB_BAR_HEIGHT
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<GraphPane>,
        _tile_id: egui_tiles::TileId,
        _state: &egui_tiles::TabState,
    ) -> egui::Stroke {
        egui::Stroke::NONE
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }

    fn is_tab_closable(
        &self,
        _tiles: &egui_tiles::Tiles<GraphPane>,
        _tile_id: egui_tiles::TileId,
    ) -> bool {
        // Allow closing tabs if there is more than one head open.
        self.cx.access.heads().len() > 1
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<GraphPane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        if let Some(GraphPane(head)) = tiles.get_pane(&tile_id).cloned() {
            self.cx
                .response
                .responses
                .push(None, crate::CloseHead(head));
        }
        // Return true to allow egui_tiles to remove the tile.
        true
    }

    fn tab_ui(
        &mut self,
        tiles: &mut egui_tiles::Tiles<GraphPane>,
        ui: &mut egui::Ui,
        id: egui::Id,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        let edit_state_id = egui::Id::new("tab_edit_state");
        let mut edit_state: TabEditState = ui
            .memory_mut(|m| m.data.get_temp(edit_state_id))
            .unwrap_or_default();

        let is_editing = edit_state.editing_tile_id == Some(tile_id);

        let response = if is_editing {
            let head = tiles.get_pane(&tile_id).map(|GraphPane(h)| h.clone());
            let names = crate::reg::names(self.cx.gantz.env.registry);

            let name_res = head.as_ref().map(|h| {
                ui.scope(|ui| {
                    ui.set_max_width(ui.available_width().min(150.0));
                    widget::head_name_edit(h, &mut edit_state.edit_text, &names, ui)
                })
                .inner
            });

            let Some(name_res) = name_res else {
                edit_state.editing_tile_id = None;
                edit_state.edit_text.clear();
                ui.memory_mut(|m| m.data.insert_temp(edit_state_id, edit_state));
                return ui.label("");
            };

            // Request focus on the first frame after entering edit mode.
            if edit_state.request_focus {
                name_res.response.request_focus();
                edit_state.request_focus = false;
            }

            // head_name_edit resets the text on commit or cancel, so detect
            // focus loss or escape to clear the tab editing state.
            let editing_ended =
                name_res.response.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            if editing_ended {
                if let Some((original, new_name)) = name_res.new_branch {
                    let branch = crate::BranchHead { original, new_name };
                    self.cx.response.responses.push(None, branch);
                }
                edit_state.editing_tile_id = None;
                edit_state.edit_text.clear();
            }

            name_res.response
        } else {
            // Append a filled circle if this head is focused.
            let mut title = self.tab_title_for_tile(tiles, tile_id).text().to_string();
            let mut session = None;
            let mut audio = None;
            if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
                let heads = self.cx.access.heads();
                if crate::head_is_focused(heads, self.cx.focused_head, head) {
                    title.push_str(" ⚫");
                }
                // The head's collab session, when shared.
                if let (Some(collab), gantz_ca::Head::Branch(name)) = (self.cx.gantz.collab, head) {
                    session = collab.sessions.get(name);
                }
                if self
                    .cx
                    .gantz
                    .audio_heads
                    .is_some_and(|heads| heads.contains(head))
                {
                    audio = Some(self.cx.state.open_heads.get(head).is_some_and(|s| s.muted));
                }
            }
            let mut tab = widget::Tab::new(title, id)
                .active(state.active)
                .closable(state.closable)
                .hint("double-click to rename");
            if let Some(display) = session {
                tab = tab.status_dot(display.conn.color(), display.hover_text());
            }
            if let Some(muted) = audio {
                tab = tab.audio(muted);
            }
            let res = tab.show(ui);

            if res.audio.as_ref().is_some_and(|r| r.clicked()) {
                if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
                    let head_state = self.cx.state.open_heads.entry(head.clone()).or_default();
                    head_state.muted = !head_state.muted;
                }
            }

            // Handle double-click to enter edit mode.
            if res.tab.double_clicked() {
                if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
                    let initial_text = match head {
                        gantz_ca::Head::Branch(name) => name.to_string(),
                        gantz_ca::Head::Commit(_) => String::new(),
                    };
                    edit_state.editing_tile_id = Some(tile_id);
                    edit_state.edit_text = initial_text;
                    edit_state.request_focus = true;
                }
            }

            // Update focused_head when this tab is clicked.
            if res.tab.clicked() {
                if let Some(GraphPane(head)) = tiles.get_pane(&tile_id) {
                    if let Some(ix) = self.cx.access.heads().iter().position(|h| h == head) {
                        self.cx.focused_head = ix;
                    }
                }
            }

            // Handle close button click directly, like egui_tiles default does.
            if res.close.is_some_and(|r| r.clicked()) {
                if self.on_tab_close(tiles, tile_id) {
                    tiles.remove(tile_id);
                }
            }

            res.tab
        };

        ui.memory_mut(|m| m.data.insert_temp(edit_state_id, edit_state));

        response
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut GraphPane,
    ) -> egui_tiles::UiResponse {
        let GraphPane(pane_head) = pane;

        // Find the index of this head, for updating `focused_head`.
        let ix = self
            .cx
            .access
            .heads()
            .iter()
            .position(|h| h == pane_head)
            .expect("pane head not found in heads");

        let immutable = head_immutable(
            pane_head,
            self.cx.gantz.base_immutable,
            self.cx.gantz.base_names,
        );
        let diagnostics = self.cx.access.diagnostics(pane_head).to_vec();

        // Global layout params combined with this head's flow.
        let layout_config = self.cx.state.layout_config;
        // Global grid, snap and align options, applied to every head.
        let scene_config = self.cx.state.scene_config;
        let head_state = self
            .cx
            .state
            .open_heads
            .entry(pane_head.clone())
            .or_default();
        let layout_params = layout_config.to_params(head_state.layout_flow);
        // Disjoint borrow of a sibling field of `open_heads` for the graph
        // scene's "Panes" context submenu.
        let view_toggles = &mut self.cx.state.view_toggles;
        // Disjoint borrow for the scene-level Select-all shortcut.
        let keymap = &self.cx.state.keymap;

        // The rect positions the floating breadcrumb window.
        let rect = ui.available_rect_before_wrap();

        // Get mutable access to this head's data and render the graph scene.
        // The camera rides out for overlays that map graph-space positions to
        // the pane, such as peer pointers.
        let (graph_response, camera) = match self.cx.access.with_head_mut(pane_head, |data| {
            let camera = data.view.camera;
            let res = graph_scene(
                self.cx.gantz.env,
                self.cx.gantz.codec,
                data.graph,
                data.instances,
                pane_head,
                head_state,
                view_toggles,
                self.ext_panes,
                self.cx.gantz.edge_styles,
                data.view,
                layout_params,
                scene_config,
                immutable,
                self.cx.gantz.validates_change_tracking(),
                keymap,
                &diagnostics,
                data.vm,
                ui,
            );
            (res, camera)
        }) {
            Some((res, camera)) => (res, Some(camera)),
            None => (None, None),
        };

        if let Some(response) = graph_response {
            // Focus this head when clicking on the graph or any of its nodes.
            if response.scene.clicked() || response.any_node_interacted() {
                self.cx.focused_head = ix;
            }
            // Record a CA-affecting edit so the app can commit just this head.
            if response.changed {
                self.cx.response.changed_heads.push(pane_head.clone());
            }
            // Collect this frame's deletions. `Gantz::show` migrates the
            // top-level tree's node-view paths past them after layout, since
            // the live top-level tree is not reachable here mid-traversal.
            if !response.reindex.is_empty() {
                self.cx
                    .response
                    .node_view_reindexes
                    .push((pane_head.clone(), response.reindex));
            }
            // Tag the scene's emissions with this head.
            self.cx
                .response
                .responses
                .extend(Some(&*pane_head), response.responses);
        }

        // Floating name breadcrumb for nested graphs.
        let crumbs = name_breadcrumb(rect, pane_head, ui);
        self.cx.response.responses.extend(Some(&*pane_head), crumbs);

        // A collab-session overlay. While a join is still connecting or has
        // failed, the pane's graph is only a placeholder, so dim the scene and
        // say what is happening. Peers' live pointers paint beneath it.
        if let (Some(collab), gantz_ca::Head::Branch(name)) = (self.cx.gantz.collab, &*pane_head) {
            if let Some(display) = collab.sessions.get(name) {
                if self.cx.state.collab.show_pointers && !display.pointers.is_empty() {
                    if let Some(camera) = camera {
                        paint_peer_pointers(rect, camera, &display.pointers, ui);
                        // Cursors move between this peer's frames.
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(100));
                    }
                }
                paint_session_overlay(rect, display, ui);
            }
        }

        egui_tiles::UiResponse::None
    }
}

/// Paint session peers' live pointers over the pane.
///
/// Positions arrive in graph-space coordinates. The head's camera maps them
/// to screen space, so cursors land on the right nodes regardless of either
/// peer's viewport. Painted on a foreground layer for the same reason as
/// [`paint_session_overlay`]. The scene's sublayer background would hide a
/// plain `ui.painter()` overlay.
fn paint_peer_pointers(
    rect: egui::Rect,
    camera: gantz_edit::Camera,
    pointers: &[crate::collab::PointerDisplay],
    ui: &egui::Ui,
) {
    let layer = egui::LayerId::new(egui::Order::Foreground, ui.id().with("peer_pointers"));
    let mut painter = ui.ctx().layer_painter(layer);
    painter.set_clip_rect(rect);
    for pointer in pointers {
        let screen = rect.center() + (pointer.pos - camera.center) * camera.zoom;
        // Skip cursors far outside the viewport. The clip rect would hide
        // them anyway, and this skips the label layout too.
        if !rect.expand(24.0).contains(screen) {
            continue;
        }
        painter.circle(
            screen,
            4.0,
            pointer.color,
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
        );
        painter.text(
            screen + egui::vec2(8.0, 6.0),
            egui::Align2::LEFT_TOP,
            &pointer.label,
            egui::FontId::proportional(11.0),
            pointer.color,
        );
    }
}

/// Dim a joining session's still-empty scene with its sync progress, or the
/// error when the join failed. Painted only while the join placeholder is
/// shown, that is while `awaiting_snapshot`. Once the snapshot arrives the
/// graph renders unobscured. A host never shows a placeholder.
fn paint_session_overlay(rect: egui::Rect, display: &crate::collab::SessionDisplay, ui: &egui::Ui) {
    use crate::collab::SessionConn;
    // Only ever cover the empty placeholder scene. Once the graph has loaded a
    // mid-session error surfaces through the tab's status dot, not a full-scene
    // overlay that would obscure a usable graph.
    if !display.awaiting_snapshot {
        return;
    }
    let (heading, detail, color) = if let Some(error) = &display.error {
        (
            "Failed to connect".to_string(),
            Some(error.clone()),
            SessionConn::Degraded.color(),
        )
    } else {
        // Animated ellipsis while waiting.
        let dots = 1 + (ui.input(|i| i.time) * 2.0) as usize % 3;
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
        let heading = format!("Connecting{}", ".".repeat(dots));
        (
            heading,
            Some(display.sync_status()),
            ui.visuals().strong_text_color(),
        )
    };
    // egui_graph draws the scene in a sublayer with an opaque background,
    // composited directly above this pane's own layer, so a `ui.painter()`
    // overlay would be hidden beneath it. Paint on a foreground layer instead.
    let layer = egui::LayerId::new(egui::Order::Foreground, ui.id().with("session_overlay"));
    let mut painter = ui.ctx().layer_painter(layer);
    painter.set_clip_rect(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(120));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        heading,
        egui::FontId::proportional(20.0),
        color,
    );
    if let Some(detail) = detail {
        painter.text(
            rect.center() + egui::vec2(0.0, 28.0),
            egui::Align2::CENTER_CENTER,
            detail,
            egui::FontId::proportional(14.0),
            ui.visuals().weak_text_color(),
        );
    }
}

/// Create an empty graph tree. Panes will be added by `sync_graph_panes`.
pub(super) fn create_empty_graph_tree() -> egui_tiles::Tree<GraphPane> {
    egui_tiles::Tree::empty("graph-tiles")
}

/// Sync the graph tree panes with the current heads.
///
/// Adds missing panes for new heads and removes panes for heads that no longer exist.
pub(super) fn sync_graph_panes(tree: &mut egui_tiles::Tree<GraphPane>, heads: &[gantz_ca::Head]) {
    use std::collections::HashSet;

    let existing: HashSet<gantz_ca::Head> = tree
        .tiles
        .iter()
        .filter_map(|(_, tile)| match tile {
            egui_tiles::Tile::Pane(GraphPane(head)) => Some(head.clone()),
            _ => None,
        })
        .collect();

    let current: HashSet<gantz_ca::Head> = heads.iter().cloned().collect();

    for head in heads {
        if !existing.contains(head) {
            let pane_id = tree.tiles.insert_pane(GraphPane(head.clone()));
            if let Some(root_id) = tree.root() {
                tree.move_tile_to_container(pane_id, root_id, usize::MAX, true);
            } else {
                let root = tree.tiles.insert_tab_tile(vec![pane_id]);
                tree.root = Some(root);
            }
        }
    }

    let orphaned: Vec<egui_tiles::TileId> = tree
        .tiles
        .iter()
        .filter_map(|(id, tile)| match tile {
            egui_tiles::Tile::Pane(GraphPane(head)) if !current.contains(head) => Some(*id),
            _ => None,
        })
        .collect();
    for id in orphaned {
        tree.tiles.remove(id);
    }
}
