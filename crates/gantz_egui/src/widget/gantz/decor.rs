//! The window's own decorations, for hosts that draw the window without the
//! system's title bar. See [`WindowStyle`].
//!
//! The window is one more surface of the app: the scene fills it, its corners
//! take the pane radius, and its controls sit with the panes. The widget only
//! draws the decorations and asks for the window actions through
//! [`WindowAction`] payloads. The host applies them to its window.

use super::chrome::{self, Icon};
use crate::{
    keybind,
    style::{self, Role},
};

/// How the window is decorated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum WindowStyle {
    /// The system's own title bar and frame.
    Native,
    /// No title bar. The window's controls sit over the scene's top-right
    /// corner, or in the header of the pane there.
    Controls,
    /// A slim header on the scene with the logo, the graph's name and the
    /// window's controls. It pairs with the default status line.
    #[default]
    Header,
}

/// The state of a window that the widget decorates. A host passes it through
/// [`super::Gantz::native_window`] when it can draw its window without the
/// system's decorations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeWindow {
    /// Whether the window is maximised or fullscreen. Its corners are then
    /// square.
    pub maximized: bool,
}

/// An action on the window, emitted as a payload for the host to apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    Minimize,
    /// Maximise the window, or restore it when maximised.
    ToggleMaximize,
    Close,
    /// Move the window with the pointer, from a press on a drag area.
    Drag,
    /// Resize the window with the pointer, from a press on an edge.
    Resize(WindowEdge),
}

/// An edge or corner of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEdge {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

/// The width of the band along the window's edges that resizes it.
const EDGE: f32 = 5.0;

/// The id under which the context records whether the widget decorates the
/// window. See [`decorates`].
const DECORATES_ID: &str = "gantz-window-decorates";

impl WindowStyle {
    /// The status line style that suits the window. The controls float like
    /// the status pill, and the header pairs with the gutter line.
    pub fn status(self) -> super::StatusStyle {
        match self {
            Self::Controls => super::StatusStyle::Pill,
            Self::Native | Self::Header => super::StatusStyle::Gutter,
        }
    }
}

impl WindowEdge {
    const ALL: [WindowEdge; 8] = [
        WindowEdge::North,
        WindowEdge::NorthEast,
        WindowEdge::East,
        WindowEdge::SouthEast,
        WindowEdge::South,
        WindowEdge::SouthWest,
        WindowEdge::West,
        WindowEdge::NorthWest,
    ];

    /// The band of `window` that resizes it from this edge.
    fn band(self, window: egui::Rect) -> egui::Rect {
        let (l, r, t, b) = (window.left(), window.right(), window.top(), window.bottom());
        let x = |lo: f32, hi: f32| egui::Rangef::new(lo, hi);
        let (xs, ys) = match self {
            WindowEdge::North => (x(l + EDGE, r - EDGE), x(t, t + EDGE)),
            WindowEdge::South => (x(l + EDGE, r - EDGE), x(b - EDGE, b)),
            WindowEdge::West => (x(l, l + EDGE), x(t + EDGE, b - EDGE)),
            WindowEdge::East => (x(r - EDGE, r), x(t + EDGE, b - EDGE)),
            WindowEdge::NorthWest => (x(l, l + EDGE), x(t, t + EDGE)),
            WindowEdge::NorthEast => (x(r - EDGE, r), x(t, t + EDGE)),
            WindowEdge::SouthWest => (x(l, l + EDGE), x(b - EDGE, b)),
            WindowEdge::SouthEast => (x(r - EDGE, r), x(b - EDGE, b)),
        };
        egui::Rect::from_x_y_ranges(xs, ys)
    }

    fn cursor(self) -> egui::CursorIcon {
        match self {
            WindowEdge::North => egui::CursorIcon::ResizeNorth,
            WindowEdge::NorthEast => egui::CursorIcon::ResizeNorthEast,
            WindowEdge::East => egui::CursorIcon::ResizeEast,
            WindowEdge::SouthEast => egui::CursorIcon::ResizeSouthEast,
            WindowEdge::South => egui::CursorIcon::ResizeSouth,
            WindowEdge::SouthWest => egui::CursorIcon::ResizeSouthWest,
            WindowEdge::West => egui::CursorIcon::ResizeWest,
            WindowEdge::NorthWest => egui::CursorIcon::ResizeNorthWest,
        }
    }
}

/// The window's minimise, maximise and close buttons, in a row. Each shows a
/// fill only while hovered. The row reads left to right in any layout.
/// Returns the action of the button clicked.
pub(super) fn controls(ui: &mut egui::Ui, window: NativeWindow) -> Option<WindowAction> {
    let mut action = None;
    let side = ui.spacing().interact_size.y;
    let gap = 2.0;
    let size = egui::vec2(3.0 * side + 2.0 * gap, side);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let row = egui::UiBuilder::new()
        .max_rect(rect)
        .layout(egui::Layout::left_to_right(egui::Align::Center));
    ui.scope_builder(row, |ui| {
        ui.spacing_mut().item_spacing.x = gap;
        if chrome::icon_button(ui, Icon::Minimize, "minimise").clicked() {
            action = Some(WindowAction::Minimize);
        }
        let (icon, hint) = match window.maximized {
            true => (Icon::Restore, "restore"),
            false => (Icon::Maximize, "maximise"),
        };
        if chrome::icon_button(ui, icon, hint).clicked() {
            action = Some(WindowAction::ToggleMaximize);
        }
        if chrome::icon_button(ui, Icon::Close, "close").clicked() {
            action = Some(WindowAction::Close);
        }
    });
    action
}

/// The window's controls over the scene, with their top-right corner at
/// `pos`. Returns the action of the button clicked.
pub(super) fn floating_controls(
    ctx: &egui::Context,
    pos: egui::Pos2,
    window: NativeWindow,
) -> Option<WindowAction> {
    egui::Area::new(egui::Id::new("gantz-window-controls"))
        .order(egui::Order::Middle)
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            // The controls centre on the tab strip's height beside them.
            let margin = 4.0;
            ui.set_min_height(crate::widget::kit::pill_height(ui) + 2.0 * margin);
            ui.horizontal_centered(|ui| {
                ui.add_space(margin);
                let action = controls(ui, window);
                ui.add_space(margin);
                action
            })
            .inner
        })
        .inner
}

/// The slim header across the top of the window, in `rect`: the logo, the
/// focused graph's name and the window's controls. A drag on its empty space
/// moves the window and a double-click maximises it. Returns the window
/// action. See [`logo_button`] for the logo's.
///
/// The header shows in an area, as the main scene's layer reaches under it.
pub(super) fn header(
    ctx: &egui::Context,
    rect: egui::Rect,
    title: &str,
    window: NativeWindow,
    logo_dot: Option<egui::Color32>,
    view: &mut super::ViewToggles,
    responses: &mut crate::response::Responses,
    ext_panes: &[crate::widget::ExtPaneEntry],
) -> Option<WindowAction> {
    egui::Area::new(egui::Id::new("gantz-window-header"))
        .order(egui::Order::Middle)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            ui.expand_to_include_rect(rect);
            header_ui(
                ui, rect, title, window, logo_dot, view, responses, ext_panes,
            )
        })
        .inner
}

fn header_ui(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    title: &str,
    window: NativeWindow,
    logo_dot: Option<egui::Color32>,
    view: &mut super::ViewToggles,
    responses: &mut crate::response::Responses,
    ext_panes: &[crate::widget::ExtPaneEntry],
) -> Option<WindowAction> {
    let drag = ui.interact(
        rect,
        egui::Id::new("gantz-window-drag"),
        egui::Sense::click_and_drag(),
    );
    let mut action = drag_action(&drag);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let weak = ui.visuals().weak_text_color();
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        title,
        font,
        weak,
    );
    let inner = rect.shrink2(egui::vec2(chrome::HEADER_INSET, 0.0));
    let row = |rect: egui::Rect| {
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center))
    };
    ui.scope_builder(row(inner), |ui| {
        logo_button(ui, logo_dot, view, responses, ext_panes);
    });
    let right = egui::UiBuilder::new()
        .max_rect(inner)
        .layout(egui::Layout::right_to_left(egui::Align::Center));
    ui.scope_builder(right, |ui| {
        if let Some(a) = controls(ui, window) {
            action = Some(a);
        }
    });
    action
}

/// The window action of a press on a drag area: a drag moves the window and
/// a double-click maximises it.
pub(super) fn drag_action(response: &egui::Response) -> Option<WindowAction> {
    if response.double_clicked() {
        Some(WindowAction::ToggleMaximize)
    } else if response.drag_started() {
        Some(WindowAction::Drag)
    } else {
        None
    }
}

/// The bands along the edges of `window` that resize it, each showing its
/// resize cursor. Returns the edge pressed this pass.
pub(super) fn resize_edges(ctx: &egui::Context, window: egui::Rect) -> Option<WindowAction> {
    let mut action = None;
    for edge in WindowEdge::ALL {
        let band = edge.band(window);
        egui::Area::new(egui::Id::new(("gantz-window-edge", edge as u8)))
            .order(egui::Order::Foreground)
            .fixed_pos(band.min)
            .show(ctx, |ui| {
                let (_, response) = ui.allocate_exact_size(band.size(), egui::Sense::drag());
                if response.drag_started() {
                    action = Some(WindowAction::Resize(edge));
                }
                response.on_hover_cursor(edge.cursor());
            });
    }
    action
}

/// The gantz mark as a button. A click shows or hides the panes and a
/// secondary click opens the app menu. A dot in `dot`'s colour shows there
/// are problems in the log.
fn logo(ui: &mut egui::Ui, dot: Option<egui::Color32>) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    // The logo moves between the header, a pane's tabs and the scene's strip
    // as the layout changes, so it takes a fixed id. One shows at a time.
    let (rect, _) = ui.allocate_exact_size(egui::vec2(height + 6.0, height), egui::Sense::hover());
    let response = ui.interact(rect, egui::Id::new("gantz-logo"), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let color = match response.hovered() {
            true => visuals.strong_text_color(),
            false => visuals.text_color(),
        };
        if response.hovered() {
            let radius = style::shape(ui.ctx()).widget_corner(height);
            ui.painter()
                .rect_filled(rect, radius, visuals.widgets.hovered.weak_bg_fill);
        }
        let mark = crate::logo::mark(height * 0.7, color);
        let size = mark.size().unwrap_or(egui::Vec2::splat(height * 0.7));
        mark.paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
        if let Some(color) = dot {
            let pos = rect.right_top() + egui::vec2(-3.0, 3.0);
            ui.painter().circle_filled(pos, 3.0, color);
            let ring = egui::Stroke::new(1.5f32, style::color(ui, Role::Scene));
            ui.painter().circle_stroke(pos, 3.5, ring);
        }
    }
    let hint = match keybind::shortcut_text(ui.ctx(), crate::Action::TogglePatchOnly) {
        Some(keys) => format!("show or hide the panes ({keys}). right-click for the menu"),
        None => "show or hide the panes. right-click for the menu".to_string(),
    };
    response.on_hover_text(hint)
}

/// The logo with its actions: a click shows or hides the panes and a
/// secondary click opens the app menu. See [`logo`].
pub(super) fn logo_button(
    ui: &mut egui::Ui,
    dot: Option<egui::Color32>,
    view: &mut super::ViewToggles,
    responses: &mut crate::response::Responses,
    ext_panes: &[crate::widget::ExtPaneEntry],
) {
    let response = logo(ui, dot);
    if response.clicked() {
        view.patch_only = !view.patch_only;
    }
    app_menu(&response, view, responses, ext_panes);
}

/// The app menu on a secondary click of `response`: a new graph, the
/// sidebar, the panes and the settings. It leads with the wordmark.
fn app_menu(
    response: &egui::Response,
    view: &mut super::ViewToggles,
    responses: &mut crate::response::Responses,
    ext_panes: &[crate::widget::ExtPaneEntry],
) {
    egui::Popup::context_menu(response).show(|ui| {
        let strong = ui.visuals().strong_text_color();
        ui.add(crate::logo::wordmark(14.0, strong));
        ui.add_space(4.0);
        let ctx = ui.ctx().clone();
        let keys = |action| keybind::shortcut_text(&ctx, action);
        let new_graph = keys(crate::Action::NewGraph);
        if crate::widget::kit::menu_item(ui, "new graph", new_graph).clicked() {
            responses.push(None, crate::NewGraph);
            ui.close();
        }
        ui.separator();
        ui.checkbox(&mut view.sidebar_open, "sidebar");
        ui.menu_button("panes", |ui| {
            crate::widget::panes_config(view, ext_panes, ui);
            ui.separator();
            if crate::widget::reset_layout_button(ui) {
                responses.push(None, crate::ResetTilesLayout);
                ui.close();
            }
        });
        if ui.button("settings").clicked() {
            view.settings = true;
            view.sidebar_open = true;
            ui.close();
        }
    });
}

/// Record whether the host lets the widget decorate its window this pass.
pub(super) fn set_decorates(ctx: &egui::Context, decorates: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(DECORATES_ID), decorates));
}

/// Whether the host lets the widget decorate its window. The window style
/// setting only shows when it does.
pub(crate) fn decorates(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp(egui::Id::new(DECORATES_ID)))
        .unwrap_or(false)
}
