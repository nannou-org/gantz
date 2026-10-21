//! The graphs on the scene and in graph panes, and the tab strip that floats
//! over the scene.
//!
//! The main graph fills the scene. Any other open graph may show in a graph
//! pane beside it. The tab strip lists every open graph. A click shows a
//! graph on the scene, or focuses its pane.

use super::PaneCtx;
use super::panes::{graph_scene, head_immutable};
use crate::{
    HeadAccess, OpenGraphPane, ReplaceHead,
    response::DynResponse,
    style::{self, Role},
    widget,
};

/// The id under which the tab strip keeps a rename in progress.
const RENAME_ID: &str = "gantz-tab-rename";

/// A rename of a graph from its tab.
#[derive(Clone, Default)]
struct Rename {
    /// The graph being renamed, if any.
    head: Option<gantz_ca::Head>,
    /// The text being edited.
    text: String,
    /// Whether to request focus on the next frame.
    request_focus: bool,
}

/// Draw `head`'s graph scene into `ui`. Interacting with the scene focuses
/// the head. `id` keeps the scene's state apart from other scenes.
pub(super) fn head_scene<Access>(
    cx: &mut PaneCtx<'_, '_, Access>,
    ui: &mut egui::Ui,
    head: &gantz_ca::Head,
    id: egui::Id,
    ext_panes: &[widget::ExtPaneEntry],
) where
    Access: HeadAccess,
{
    let Some(ix) = cx.access.heads().iter().position(|h| h == head) else {
        return;
    };
    let immutable = head_immutable(head, cx.gantz.base_immutable, cx.gantz.base_names);
    let diagnostics = cx.access.diagnostics(head).to_vec();

    // Global layout params combined with this head's flow.
    let layout_config = cx.state.layout_config;
    // Global grid, snap and align options, applied to every head.
    let scene_config = cx.state.scene_config;
    let head_state = cx.state.open_heads.entry(head.clone()).or_default();
    let layout_params = layout_config.to_params(head_state.layout_flow);
    // Disjoint borrows of sibling fields for the scene's context menu and
    // its select-all shortcut.
    let view_toggles = &mut cx.state.view_toggles;
    let keymap = &cx.state.keymap;

    // The rect positions the overlays.
    let rect = ui.available_rect_before_wrap();

    // The camera rides out for overlays that map graph-space positions to
    // the scene, such as peer pointers.
    let (graph_response, camera) = match cx.access.with_head_mut(head, |data| {
        let camera = data.view.camera;
        let res = graph_scene(
            id,
            cx.gantz.env,
            cx.gantz.codec,
            data.graph,
            data.instances,
            head,
            head_state,
            view_toggles,
            ext_panes,
            cx.gantz.edge_styles,
            data.view,
            layout_params,
            scene_config,
            immutable,
            cx.gantz.validates_change_tracking(),
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
        if response.scene.clicked() || response.any_node_interacted() {
            cx.focused_head = ix;
        }
        // Record a CA-affecting edit so the app can commit just this head.
        if response.changed {
            cx.response.changed_heads.push(head.clone());
        }
        if response.layout_edited {
            cx.response.layout_edited_heads.push(head.clone());
        }
        // `Gantz::show` migrates the node-view paths past this frame's
        // deletions after layout, since the live tree is not reachable here.
        if !response.reindex.is_empty() {
            cx.response
                .node_view_reindexes
                .push((head.clone(), response.reindex));
        }
        cx.response.responses.extend(Some(head), response.responses);
    }

    // While a join is still connecting or has failed, the graph is only a
    // placeholder, so dim the scene and say what is happening. Peers' live
    // pointers paint beneath it.
    if let (Some(collab), gantz_ca::Head::Branch(name)) = (cx.gantz.collab, head) {
        if let Some(display) = collab.sessions.get(name) {
            if cx.state.collab.show_pointers && !display.pointers.is_empty() {
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
}

/// The tab strip: a pill floating over the top-left of the scene, with a tab
/// per open graph and a button for a new one. The main graph's tab is filled.
/// A nested main graph shows its ancestors before its tab, each a step up.
///
/// Returns the height of the strip, so other overlays can clear it.
pub(super) fn tab_strip<Access>(
    cx: &mut PaneCtx<'_, '_, Access>,
    ui: &egui::Ui,
    anchor: egui::Pos2,
    main: Option<&gantz_ca::Head>,
) -> f32
where
    Access: HeadAccess,
{
    let rename_id = egui::Id::new(RENAME_ID);
    let mut rename: Rename = ui.data(|d| d.get_temp(rename_id)).unwrap_or_default();
    let heads: Vec<gantz_ca::Head> = cx.access.heads().to_vec();
    let shape = style::shape(ui.ctx());
    let frame = egui::Frame::new()
        .fill(style::color(ui, Role::Panel))
        .corner_radius(shape.window_corner())
        .inner_margin(egui::Margin::same(4));
    let area = egui::Area::new(egui::Id::new("gantz-tab-strip"))
        .order(egui::Order::Middle)
        .fixed_pos(anchor)
        .show(ui.ctx(), |ui| {
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if let Some(main) = main {
                        let crumbs = breadcrumb(main, ui);
                        cx.response.responses.extend(Some(main), crumbs);
                    }
                    let closable = heads.len() > 1;
                    for (ix, head) in heads.iter().enumerate() {
                        if rename.head.as_ref() == Some(head) {
                            rename_tab(cx, ui, head, &mut rename);
                        } else {
                            graph_tab(cx, ui, head, ix, main, closable, &mut rename);
                        }
                    }
                    let new = widget::kit::pill(ui, false, "+").on_hover_text("new graph");
                    if new.clicked() {
                        cx.response.responses.push(None, crate::NewGraph);
                    }
                });
            });
        });
    ui.data_mut(|d| d.insert_temp(rename_id, rename));
    area.response.rect.height()
}

/// One graph's tab. A click shows the graph on the scene, or focuses its
/// pane. A double-click renames it. A right-click offers the rest.
fn graph_tab<Access>(
    cx: &mut PaneCtx<'_, '_, Access>,
    ui: &mut egui::Ui,
    head: &gantz_ca::Head,
    ix: usize,
    main: Option<&gantz_ca::Head>,
    closable: bool,
    rename: &mut Rename,
) where
    Access: HeadAccess,
{
    let id = ui.id().with(("tab", ix));
    let in_pane = cx.state.graphs.in_pane(head);
    let title = match (main, head) {
        // A nested main graph's ancestors lead the strip, so its tab names
        // only its own level.
        (Some(m), gantz_ca::Head::Branch(name)) if m == head && name.is_nested() => name
            .segments()
            .last()
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| head.to_string()),
        _ => head.to_string(),
    };
    let mut tab = widget::Tab::new(title, id)
        .look(widget::tab::TabLook::Pill)
        .active(main == Some(head))
        .closable(closable)
        .hint("double-click to rename. right-click for more");
    if let (Some(collab), gantz_ca::Head::Branch(name)) = (cx.gantz.collab, head) {
        if let Some(display) = collab.sessions.get(name) {
            let color = style::color(ui, display.conn.role());
            tab = tab.status_dot(color, display.hover_text());
        }
    }
    if cx.gantz.audio_heads.is_some_and(|a| a.contains(head)) {
        let muted = cx.state.open_heads.get(head).is_some_and(|s| s.muted);
        tab = tab.audio(muted);
    }
    let res = tab.show(ui);
    if res.audio.as_ref().is_some_and(|r| r.clicked()) {
        let head_state = cx.state.open_heads.entry(head.clone()).or_default();
        head_state.muted = !head_state.muted;
    }
    if res.close.as_ref().is_some_and(|r| r.clicked()) {
        cx.response
            .responses
            .push(None, crate::CloseHead(head.clone()));
    }
    if res.tab.clicked() {
        cx.focused_head = ix;
        if !in_pane {
            cx.state.graphs.main = Some(head.clone());
        }
    }
    if res.tab.double_clicked() {
        start_rename(head, rename);
    }
    res.tab.context_menu(|ui| {
        if in_pane {
            if ui.button("show on the scene").clicked() {
                cx.state.graphs.panes.retain(|_, h| h != head);
                cx.state.graphs.main = Some(head.clone());
                cx.focused_head = ix;
                ui.close();
            }
        } else if ui.button("open in a pane").clicked() {
            cx.response
                .responses
                .push(None, OpenGraphPane(head.clone()));
            ui.close();
        }
        if ui.button("rename").clicked() {
            start_rename(head, rename);
            ui.close();
        }
        if closable && ui.button("close").clicked() {
            cx.response
                .responses
                .push(None, crate::CloseHead(head.clone()));
            ui.close();
        }
    });
}

/// A tab while its graph is renamed: a name field that commits on Enter or
/// focus loss and cancels on Escape.
fn rename_tab<Access>(
    cx: &mut PaneCtx<'_, '_, Access>,
    ui: &mut egui::Ui,
    head: &gantz_ca::Head,
    rename: &mut Rename,
) where
    Access: HeadAccess,
{
    let names = crate::reg::names(cx.gantz.env.registry);
    let res = ui
        .scope(|ui| {
            ui.set_max_width(160.0);
            widget::head_name_edit(head, &mut rename.text, &names, ui)
        })
        .inner;
    if rename.request_focus {
        res.response.request_focus();
        rename.request_focus = false;
    }
    let ended = res.response.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Escape));
    if ended {
        if let Some((original, new_name)) = res.new_branch {
            let branch = crate::BranchHead { original, new_name };
            cx.response.responses.push(None, branch);
        }
        *rename = Rename::default();
    }
}

fn start_rename(head: &gantz_ca::Head, rename: &mut Rename) {
    rename.head = Some(head.clone());
    rename.text = match head {
        gantz_ca::Head::Branch(name) => name.to_string(),
        gantz_ca::Head::Commit(_) => String::new(),
    };
    rename.request_focus = true;
}

/// The ancestors of a nested graph, each a weak label that steps up to it,
/// with a separator after each. A root graph has none.
fn breadcrumb(head: &gantz_ca::Head, ui: &mut egui::Ui) -> Vec<DynResponse> {
    let mut responses = vec![];
    let gantz_ca::Head::Branch(name) = head else {
        return responses;
    };
    if !name.is_nested() {
        return responses;
    }
    let segs: Vec<&str> = name.segments().iter().map(|s| s.as_str()).collect();
    let sep = gantz_ca::name::SEP.to_string();
    for i in 0..segs.len() - 1 {
        let prefix = segs[..=i].join(&sep);
        let resp =
            widget::kit::pill(ui, false, segs[i]).on_hover_text(format!("go up to {prefix}"));
        if resp.clicked() {
            let parent = gantz_ca::Head::Branch(prefix.parse().expect("a valid prefix"));
            responses.push(DynResponse::new(ReplaceHead(parent)));
        }
        ui.weak("/");
    }
    responses
}

/// Paint session peers' live pointers over the scene.
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
            egui::Stroke::new(1.0f32, style::color(ui, Role::Scrim)),
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
            style::color(ui, SessionConn::Degraded.role()),
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
    painter.rect_filled(rect, 0.0, style::color(ui, Role::Scrim));
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
