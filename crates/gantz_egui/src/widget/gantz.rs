use crate::{
    Env, HeadAccess, OpenGraphPane, OpenLogs, OpenNodePalette, OpenNodeView, OpenReplacePalette,
    ResetTilesLayout,
    node::NodeCodec,
    response::Responses,
    style::{self, Role, StyleConfig},
    widget::{self, graph_scene, toast},
};
use std::collections::{HashMap, HashSet};

mod chrome;
mod corner;
mod decor;
mod graph_tabs;
mod panes;
mod state;
mod status;
mod tiles;
mod windowing;

pub(crate) use decor::decorates;
pub use decor::{NativeWindow, WindowAction, WindowEdge, WindowStyle};
use panes::{collect_gantz_file_drops, gui_role_badge, render_pane, set_gui_role};
pub use state::{
    AlignConfig, EdgeConfig, EdgeKind, FollowSelectionConfig, GantzState, GraphPlacement,
    GridConfig, LayoutConfig, OpenHeadState, OpenHeadStates, PaneWindowGeometry, SceneConfig,
    SnapConfig, SnapMode, ViewToggles, request_clear_egui_memory,
};
use state::{
    clear_egui_memory_id, default_sidebar_width, default_tray_height, load_ron, load_tree,
    store_ron, store_tree,
};
pub use status::{MeterLevel, StatusAction, StatusIcon, StatusItem, StatusStyle};
pub use tiles::{NodeViewPane, Pane};
use tiles::{
    add_graph_pane, add_node_view_pane, apply_reveal, capture_fixed_sizes, create_tree,
    ext_pane_entries, impose_fixed_sizes, impose_min_sizes, migrate_node_view_paths,
    pane_is_hideable, pane_is_poppable, pane_is_visible, pane_label, pane_min_size, pane_title,
    reveal_pane, set_pane_visible, set_tile_visibility, simplify_tree, sync_ext_panes,
    sync_singleton_pane,
};
pub use windowing::{close_windowed_pane, pane_key, redock_windowed_pane};
use windowing::{
    detach_pane, drain_pending, migrate_windowed_node_views, pending_close_id, pending_redock_id,
    redock_pane, windowed_panes_id,
};

/// A file dropped onto a gantz pane.
#[derive(Debug)]
pub struct FileDrop {
    pub bytes: Vec<u8>,
    pub target: FileDropTarget,
}

/// Which pane received the file drop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDropTarget {
    /// The Graphs pane. Merge the registry and views only.
    Graphs,
    /// The GraphScene pane. Merge, then open the root graph if unique.
    GraphScene,
}

/// The reserved node-type name that creates a new nested graph.
///
/// Selecting this entry in the node palette emits a
/// [`CreateNestedGraph`] rather than a [`CreateNode`], so a nested graph is
/// created like any other node but routed through the registry-aware op.
///
/// [`CreateNestedGraph`]: crate::CreateNestedGraph
/// [`CreateNode`]: crate::CreateNode
pub const NESTED_GRAPH_TYPE: &str = "graph";

/// The top-level gantz widget.
pub struct Gantz<'a> {
    env: &'a Env<'a>,
    /// The value-level codec through which working-graph nodes reify for
    /// their UI passes and erase back on change.
    codec: &'a NodeCodec,
    base_names: &'a crate::reg::Names,
    log_source: Option<LogSource>,
    perf_vm: Option<&'a mut widget::PerfCapture>,
    perf_gui: Option<&'a mut widget::PerfCapture>,
    base_immutable: bool,
    compile_config: Option<gantz_core::compile::Config>,
    validate_change_tracking: Option<bool>,
    settings_tabs: &'a mut [&'a mut dyn widget::SettingsTab],
    ext_panes: &'a mut [&'a mut dyn widget::ExtPane],
    ref_ext_uis: &'a [&'a dyn crate::node::RefExtUi],
    edge_styles: &'a [&'a dyn widget::EdgeStyle],
    base_sources: Option<BaseSourcesCtx<'a>>,
    pane_window_mode: PaneWindowMode,
    collab: Option<&'a crate::collab::CollabUiState>,
    /// A host-provided clipboard reader for widget paste affordances, since
    /// egui alone cannot read the clipboard. `None` hides them.
    clipboard: Option<&'a dyn Fn() -> Option<String>>,
    audio_heads: Option<&'a HashSet<gantz_ca::Head>>,
    /// The host's items for the status line.
    status_items: &'a [StatusItem],
    /// The app's version, shown on hovering the wordmark.
    version: Option<&'a str>,
    /// The window the widget decorates, when the host draws it without the
    /// system's decorations.
    native_window: Option<NativeWindow>,
}

/// Base-source authoring context for the graph config pane's "source"
/// dropdown. See [`widget::GraphConfig::base_sources`]. Supplied only by
/// base-authoring hosts like `update-base`, where the per-source write-back
/// makes an association change durable.
#[derive(Clone, Copy)]
pub struct BaseSourcesCtx<'a> {
    /// The available base source names, in load order.
    pub sources: &'a [&'a str],
    /// Each base name's owning source.
    pub name_sources: &'a HashMap<String, &'static str>,
    /// The source an unattributed name is written to. Session-created names
    /// are unattributed.
    pub default_source: &'a str,
}

/// Selects who draws the windows that popped-out panes live in.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PaneWindowMode {
    /// The widget draws each windowed pane as a floating `egui::Window`. Used on
    /// web, in the eframe demo, and as the native fallback. The default.
    #[default]
    EguiWindow,
    /// The host owns real OS windows and renders each windowed pane itself via
    /// [`Gantz::render_windowed_pane`]. The widget only reports the windowed
    /// set via [`GantzResponse::windowed_panes`].
    HostNative,
}

enum LogSource {
    Logger(widget::log_view::Logger),
    #[cfg(feature = "tracing")]
    TraceCapture(
        widget::trace_view::TraceCapture,
        tracing::level_filters::LevelFilter,
    ),
}

/// The `egui_tiles::Behavior::resize_stroke` shared by both tile trees.
///
/// An idle gap draws nothing, so the scene shows through it. A hovered or
/// dragged gap draws a thin line down its middle, to show it resizes.
fn separator_stroke(
    cfg: &StyleConfig,
    style: &egui::Style,
    resize_state: egui_tiles::ResizeState,
) -> egui::Stroke {
    let theme = style::theme_of(&style.visuals);
    match resize_state {
        egui_tiles::ResizeState::Idle => egui::Stroke::NONE,
        egui_tiles::ResizeState::Hovering | egui_tiles::ResizeState::Dragging => egui::Stroke::new(
            (cfg.shape.separator_width * 0.25).max(1.5),
            cfg.resolve(theme, Role::SeparatorHover),
        ),
    }
}

/// The data a single pane needs to render, independent of where the pane lives.
///
/// Extracted from [`TreeBehaviour`] so the same [`render_pane`] can draw a pane
/// as a tile in the tree, as a floating `egui::Window`, or into a native
/// host's own OS window.
struct PaneCtx<'a, 's, Access>
where
    Access: HeadAccess,
{
    gantz: &'a mut Gantz<'s>,
    state: &'a mut GantzState,
    access: &'a mut Access,
    focused_head: usize,
    response: &'a mut GantzResponse,
}

/// The context passed to the `egui_tiles::Tree` widget.
struct TreeBehaviour<'a, 's, Access>
where
    Access: HeadAccess,
{
    cx: PaneCtx<'a, 's, Access>,
    /// Panes detached into windows. The tab context menu pushes to this when a
    /// pane is popped out.
    windowed: &'a mut Vec<Pane>,
    /// Actions from the pane headers' buttons, applied after the tree is
    /// drawn.
    actions: Vec<chrome::PaneAction>,
    /// The focused pane, whose header shows its buttons.
    focus: Option<egui_tiles::TileId>,
    /// Each tabbed pane's header tab count, whether it leads, and its
    /// container. See [`chrome::headers`].
    headers: HashMap<egui_tiles::TileId, (usize, bool, egui_tiles::TileId)>,
    /// The pane whose header leads with the logo: the one in the tree's
    /// top-left corner, unless the window's header holds the logo.
    logo_tile: Option<egui_tiles::TileId>,
}

/// Response from the top-level gantz widget.
///
/// The plain fields report the state of this frame. Every action the
/// application must take, from tab management to a config change, arrives as
/// a dynamic payload in [`responses`][Self::responses] for the application to
/// drain and handle. A compile config change arrives as the new
/// [`gantz_core::compile::Config`].
#[derive(Debug)]
pub struct GantzResponse {
    /// The focused head index. User interaction may have changed it.
    pub focused_head: usize,
    /// Heads whose graph had a CA-affecting edit this frame from a node UI,
    /// an inspector edit or a structural scene edit. Lets the application
    /// commit and recompile only the changed heads instead of re-hashing
    /// every open graph each frame. May contain duplicates. Treat membership
    /// as a set.
    pub changed_heads: Vec<gantz_ca::Head>,
    /// Heads whose node layout the user edited this frame. See
    /// [`GraphSceneResponse::layout_edited`]. Lets the application commit a
    /// layout change only after an edit. May contain duplicates. Treat
    /// membership as a set.
    ///
    /// [`GraphSceneResponse::layout_edited`]: crate::widget::graph_scene::GraphSceneResponse::layout_edited
    pub layout_edited_heads: Vec<gantz_ca::Head>,
    /// Dynamic payloads emitted from within the widget tree, tagged with the
    /// emitting head. See [`crate::response`] for the handling contract.
    pub responses: Responses,
    /// Panes currently popped out into windows, reported every frame.
    /// Populated in both [`PaneWindowMode`]s. Under
    /// [`PaneWindowMode::HostNative`] a host diffs this to create, title and
    /// destroy its OS windows and renders each via
    /// [`Gantz::render_windowed_pane`].
    pub windowed_panes: Vec<WindowedPane>,
    /// Per-head node index remappings from this frame's deletions, collected
    /// during traversal and applied to the top-level tree's [`Pane::NodeView`]
    /// paths by `Gantz::show` after layout. Internal scratch that is drained
    /// before the response is returned, so applications can ignore it.
    pub(crate) node_view_reindexes: Vec<(gantz_ca::Head, crate::ops::Reindex)>,
}

/// A pane currently popped out into a window, reported via
/// [`GantzResponse::windowed_panes`].
#[derive(Clone, Debug)]
pub struct WindowedPane {
    /// The pane's identity and payload. Pass it back to
    /// [`Gantz::render_windowed_pane`] to draw it into the host's window.
    pub pane: Pane,
    /// The pane's display title, the same text as its tab.
    pub title: String,
}

struct NodeTyCmd<'a> {
    env: &'a Env<'a>,
    name: &'a str,
}

impl GantzResponse {
    /// An empty response for the given focused head.
    fn new(focused_head: usize) -> Self {
        GantzResponse {
            focused_head,
            changed_heads: Vec::new(),
            layout_edited_heads: Vec::new(),
            responses: Responses::default(),
            windowed_panes: Vec::new(),
            node_view_reindexes: Vec::new(),
        }
    }
}

impl<'a> Gantz<'a> {
    /// Instantiate the full top-level gantz widget.
    pub fn new(env: &'a Env<'a>, base_names: &'a crate::reg::Names) -> Self {
        Self {
            env,
            codec: env.codec,
            base_names,
            log_source: None,
            perf_vm: None,
            perf_gui: None,
            base_immutable: true,
            compile_config: None,
            validate_change_tracking: None,
            settings_tabs: &mut [],
            ext_panes: &mut [],
            ref_ext_uis: &[],
            edge_styles: &[],
            base_sources: None,
            pane_window_mode: PaneWindowMode::default(),
            collab: None,
            clipboard: None,
            audio_heads: None,
            status_items: &[],
            version: None,
            native_window: None,
        }
    }

    /// Provide the open heads whose graph produces audio. Their tabs show a
    /// speaker that toggles [`OpenHeadState::muted`].
    pub fn audio_heads(mut self, heads: &'a HashSet<gantz_ca::Head>) -> Self {
        self.audio_heads = Some(heads);
        self
    }

    /// Decorate the host's window, drawn without the system's decorations,
    /// as the [`WindowStyle`] in the view settings says. The widget emits
    /// [`WindowAction`] payloads for the host to apply.
    pub fn native_window(mut self, window: NativeWindow) -> Self {
        self.native_window = Some(window);
        self
    }

    /// Add the host's items to the status line, such as the audio device's
    /// rate and load. They sit after the focused graph's link and before the
    /// VM time.
    pub fn status_items(mut self, items: &'a [StatusItem]) -> Self {
        self.status_items = items;
        self
    }

    /// The app's version, such as its release and commit. It shows on hovering
    /// the wordmark over the scene.
    pub fn version(mut self, version: &'a str) -> Self {
        self.version = Some(version);
        self
    }

    /// Provide the collaborative-session display state so the Graph Config
    /// pane shows the collab row for shared or shareable graphs.
    pub fn collab(mut self, collab: &'a crate::collab::CollabUiState) -> Self {
        self.collab = Some(collab);
        self
    }

    /// Provide a clipboard reader for widget paste affordances, for example
    /// the join popup's right-click paste. Ctrl+V works through egui's own
    /// event path regardless.
    pub fn clipboard(mut self, clipboard: &'a dyn Fn() -> Option<String>) -> Self {
        self.clipboard = Some(clipboard);
        self
    }

    /// Choose whether the widget draws popped-out panes as `egui::Window`s,
    /// the default, or leaves them to the host as native OS windows.
    pub fn pane_window_mode(mut self, mode: PaneWindowMode) -> Self {
        self.pane_window_mode = mode;
        self
    }

    /// Provide the current compile config so the Graph Config pane shows
    /// the compile toggles. The config is global. It applies to all open
    /// heads, and a change is emitted as the new config payload.
    pub fn compile_config(mut self, config: gantz_core::compile::Config) -> Self {
        self.compile_config = Some(config);
        self
    }

    /// Provide the current change-tracking validation state so the Settings >
    /// Global pane shows its toggle. A change is emitted as a
    /// [`ValidateChangeTracking`](crate::ValidateChangeTracking) payload.
    ///
    /// When not provided, validation defaults on in debug builds and off in
    /// release. The node instance cache would otherwise mask a missed
    /// `changed` flag.
    pub fn validate_change_tracking(mut self, enabled: bool) -> Self {
        self.validate_change_tracking = Some(enabled);
        self
    }

    /// Whether the per-node change-tracking validator is on. See
    /// [`Gantz::validate_change_tracking`] for the default.
    fn validates_change_tracking(&self) -> bool {
        self.validate_change_tracking
            .unwrap_or(cfg!(debug_assertions))
    }

    /// Provide extension settings subtabs. See
    /// [`SettingsTab`][widget::SettingsTab]. One subtab appears per entry,
    /// and any payloads a tab emits are reported via
    /// [`GantzResponse::responses`].
    pub fn settings_tabs(mut self, tabs: &'a mut [&'a mut dyn widget::SettingsTab]) -> Self {
        self.settings_tabs = tabs;
        self
    }

    /// Provide extension top-level panes. See [`ExtPane`][widget::ExtPane].
    /// One tray tile appears per entry, and any payloads a pane emits are
    /// reported via [`GantzResponse::responses`] tagged with the focused head.
    pub fn ext_panes(mut self, panes: &'a mut [&'a mut dyn widget::ExtPane]) -> Self {
        self.ext_panes = panes;
        self
    }

    /// Provide domain extensions for the `NamedRef` node inspector. See
    /// [`RefExtUi`][crate::node::RefExtUi]. Each applicable extension's rows
    /// are appended after the ref's own inspector rows.
    pub fn ref_ext_uis(mut self, uis: &'a [&'a dyn crate::node::RefExtUi]) -> Self {
        self.ref_ext_uis = uis;
        self
    }

    /// Provide domain edge stylers for the graph scenes. See
    /// [`EdgeStyle`][widget::EdgeStyle]. The first styler returning `Some`
    /// styles each edge. Unclaimed edges keep the default styling.
    pub fn edge_styles(mut self, styles: &'a [&'a dyn widget::EdgeStyle]) -> Self {
        self.edge_styles = styles;
        self
    }

    /// Provide base-source authoring context so the graph config pane shows
    /// a "source" dropdown selecting which base file a graph belongs to. A
    /// change is emitted as a [`SetBaseSource`](crate::SetBaseSource) payload.
    pub fn base_sources(mut self, ctx: BaseSourcesCtx<'a>) -> Self {
        self.base_sources = Some(ctx);
        self
    }

    /// Enable the logging window with a basic env logger.
    pub fn logger(mut self, logger: widget::log_view::Logger) -> Self {
        self.log_source = Some(LogSource::Logger(logger));
        self
    }

    /// Enable the logging window for tracking tracing.
    #[cfg(feature = "tracing")]
    pub fn trace_capture(
        mut self,
        trace_capture: widget::trace_view::TraceCapture,
        level: tracing::level_filters::LevelFilter,
    ) -> Self {
        self.log_source = Some(LogSource::TraceCapture(trace_capture, level));
        self
    }

    /// Set the performance capture sources for VM and GUI timing.
    ///
    /// `perf_vm` is the focused head's VM timing, if a head is focused. The VM
    /// Perf pane is empty without it.
    pub fn perf_captures(
        mut self,
        perf_vm: Option<&'a mut widget::PerfCapture>,
        perf_gui: &'a mut widget::PerfCapture,
    ) -> Self {
        self.perf_vm = perf_vm;
        self.perf_gui = Some(perf_gui);
        self
    }

    /// Whether base node graphs should be immutable.
    ///
    /// When `true`, the default, graphs for heads whose branch name appears
    /// in `base_names` are shown in immutable mode. Navigation and selection
    /// work, but structural edits are disabled.
    ///
    /// Set to `false` for developer tools like `update-base` that need
    /// to edit base nodes.
    pub fn base_immutable(mut self, base_immutable: bool) -> Self {
        self.base_immutable = base_immutable;
        self
    }

    /// Present the gantz UI.
    ///
    /// The `access` parameter provides access to all open heads and their data.
    /// The `focused_head` is the index of the currently focused head.
    ///
    /// Returns a response containing the possibly updated focused head index.
    pub fn show<'s, Access>(
        mut self,
        state: &'s mut GantzState,
        focused_head: usize,
        access: &'s mut Access,
        ui: &'s mut egui::Ui,
    ) -> GantzResponse
    where
        's: 'a,
        Access: HeadAccess,
    {
        // Honour a pending "clear egui memory" request before loading any
        // persisted UI state this frame.
        if ui
            .ctx()
            .data(|d| d.get_temp::<bool>(clear_egui_memory_id()))
            .unwrap_or(false)
        {
            ui.ctx().memory_mut(|m| m.data.clear());
        }

        style::apply(ui.ctx(), &state.style);
        state.keymap.publish(ui.ctx());

        // Toggle patch-only. While a widget has the keyboard focus, its
        // binding (Tab by default) moves the focus on instead.
        let focused = ui.ctx().memory(|m| m.focused().is_some());
        if !focused && state.keymap.consume(ui, crate::Action::TogglePatchOnly) {
            state.view_toggles.patch_only = !state.view_toggles.patch_only;
            // egui reads Tab as a focus move before any widget runs.
            ui.ctx()
                .memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
        let patch_only = state.view_toggles.patch_only;
        let status_style = match patch_only {
            true => StatusStyle::Hidden,
            false => state.view_toggles.status,
        };
        // The window decorations the widget draws: the host's window, when the
        // host lets the widget decorate it and the system's are not asked for.
        decor::set_decorates(ui.ctx(), self.native_window.is_some());
        let window_style = state.view_toggles.window;
        let window = self
            .native_window
            .filter(|_| window_style != WindowStyle::Native);
        let header = window.is_some() && window_style == WindowStyle::Header;
        let problems = status::scan_log(ui.ctx(), self.log_source.as_ref());
        status::set_last(ui.ctx(), problems);

        // The persisted outer tree. The version suffix invalidates any tree
        // persisted before the latest default-layout change, forcing a
        // rebuild via `create_tree`.
        let tree_id = egui::Id::new("gantz-tiles-tree-storage-v5");

        let mut tree: egui_tiles::Tree<Pane> =
            load_tree(ui.ctx(), tree_id).unwrap_or_else(create_tree);

        // The set of panes popped out into windows, persisted alongside the tree.
        let mut windowed: Vec<Pane> = load_ron(ui.ctx(), windowed_panes_id()).unwrap_or_default();

        // Ensure every supplied extension pane has a tile.
        let ext_keys: Vec<&str> = self.ext_panes.iter().map(|p| p.key()).collect();
        sync_ext_panes(&mut tree, &ext_keys);

        // Ensure the GUI Preview and GUI Tree panes have tiles. Trees
        // persisted before they existed lack them.
        sync_singleton_pane(&mut tree, Pane::GuiPreview);
        sync_singleton_pane(&mut tree, Pane::GuiTree);

        // Check the `view_toggles` match the pane visibility.
        set_tile_visibility(&mut tree, &state.view_toggles);
        apply_reveal(ui.ctx(), &mut tree);

        // Simplify the tree, and ensure tabs are where they should be.
        simplify_tree(&mut tree, ui.ctx());

        // `available_rect_before_wrap` matches the root rect `Tree::ui` uses.
        let widget_area = ui.available_rect_before_wrap();
        // The panes sit inside the window margin. The status gutter takes the
        // bottom margin's place.
        let margin = state.style.shape.window_margin;
        let mut tree_area = widget_area.shrink(margin);
        // The window's header takes the top margin's place.
        let header_rect = header.then(|| {
            let rect = widget_area.with_max_y(widget_area.min.y + chrome::HEADER_HEIGHT);
            tree_area.min.y = rect.max.y;
            rect
        });
        if status_style == StatusStyle::Gutter {
            tree_area.max.y = (widget_area.max.y - status::GUTTER_HEIGHT).max(tree_area.min.y);
        }
        // Raise any pane below its minimum before the fixed sizes are imposed,
        // so a clamped minimum is not captured as a shrunken sidebar or tray.
        impose_min_sizes(&mut tree, state.style.shape.separator_width, tree_area);
        // Maintain a fixed sidebar width and tray height across window resizes
        // by imposing the stored pixel sizes on the share splits before layout.
        impose_fixed_sizes(&mut tree, state, tree_area);

        // The pane in the tree's top-left corner takes the logo. The window's
        // corner holds the wordmark and the window's controls over the tree's
        // top-right corner, and a pane there moves below it. The window's
        // header holds all three when it shows, and patch-only hides the
        // wordmark. A dragged tile may drop anywhere, so the corner keeps no
        // room of its own during a drag.
        let (logo_tile, corner_tile) = chrome::corner_grounds(&tree);
        let controls = window.filter(|_| window_style == WindowStyle::Controls);
        let wordmark = !header && !patch_only;
        let (corner_rect, corner_action) = decor::corner(
            ui.ctx(),
            tree_area.right_top(),
            controls,
            wordmark,
            self.version,
            controls.is_some(),
        );
        corner::set_rect(ui.ctx(), corner_rect.is_positive().then_some(corner_rect));
        let reserved = corner_tile
            .filter(|_| corner_rect.is_positive() && tree.dragged_id(ui.ctx()).is_none())
            .and_then(|pane| {
                let gap = state.style.shape.separator_width;
                corner::reserve(&mut tree, pane, tree_area, chrome::HEADER_HEIGHT, gap)
            });
        let mut window_actions: Vec<WindowAction> = corner_action.into_iter().collect();
        if let (Some(reserved), Some(_)) = (&reserved, controls) {
            // The row's empty space left of the corner moves the window.
            let band = reserved.row.with_max_x(corner_rect.left());
            window_actions.extend(decor::drag_band(ui.ctx(), "gantz-window-corner-band", band));
        }

        let mut response = GantzResponse::new(focused_head);

        let mut behaviour = TreeBehaviour {
            cx: PaneCtx {
                gantz: &mut self,
                state: &mut *state,
                access: &mut *access,
                focused_head,
                response: &mut response,
            },
            windowed: &mut windowed,
            actions: vec![],
            focus: chrome::update_focus(ui.ctx(), &tree, tree_area),
            headers: chrome::headers(&tree.tiles),
            logo_tile: logo_tile.filter(|_| !header),
        };
        // Reserve the bottom layer for the scene colour and the pane
        // grounds. They are painted once the tree has laid out its rects.
        chrome::set_bounds(ui.ctx(), tree_area, widget_area);
        let grounds = ui.painter().add(egui::Shape::Noop);
        let tree_ui = egui::UiBuilder::new().max_rect(tree_area);
        ui.scope_builder(tree_ui, |ui| tree.ui(&mut behaviour, ui));
        if let Some(reserved) = reserved {
            corner::release(&mut tree, reserved);
        }

        let focused_head = behaviour.cx.focused_head;
        let actions = std::mem::take(&mut behaviour.actions);
        let focus = behaviour.focus;
        response.focused_head = focused_head;
        let theme = style::theme_of(ui.visuals());
        // The window's corners take the pane radius, unless it fills the screen.
        let window_radius = match window {
            Some(w) if !w.maximized => state.style.shape.window_corner(),
            _ => 0.0,
        };
        let shapes = chrome::ground_shapes(
            ui.ctx(),
            &tree,
            &state.style,
            theme,
            widget_area,
            window_radius,
            focus,
        );
        ui.painter().set(grounds, egui::Shape::Vec(shapes));

        // The status line, and the toasts over the scene.
        let scene_rect = tree
            .tiles
            .find_pane(&Pane::GraphScene)
            .and_then(|id| tree.tiles.rect(id))
            .unwrap_or(tree_area);
        let head = access.heads().get(focused_head).cloned();
        let session = match (self.collab, &head) {
            (Some(collab), Some(gantz_ca::Head::Branch(name))) => collab.sessions.get(name),
            _ => None,
        };
        let vault = self.collab.and_then(|c| c.vault.as_ref());
        let link = match (session, vault) {
            (Some(session), _) => Some(status::Link::Session(session)),
            (None, Some(vault)) => Some(status::Link::Vault(vault)),
            (None, None) => None,
        };
        let saved = head
            .as_ref()
            .and_then(|h| self.env.registry.head_commit(h))
            .and_then(|commit| status::since(commit.timestamp));
        let muted = head
            .as_ref()
            .and_then(|h| state.open_heads.get(h))
            .is_some_and(|s| s.muted);
        let status = status::Status {
            graph: head.as_ref().map(ToString::to_string),
            saved,
            link,
            items: self.status_items,
            muted,
            vm: self
                .perf_vm
                .as_deref()
                .map(status::vm_recent)
                .unwrap_or_default(),
            problems,
        };
        if saved.is_some() {
            // "saved 2m ago" counts up by itself.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(30));
        }
        let inset = chrome::HEADER_INSET;
        let status_res = match status_style {
            StatusStyle::Gutter => {
                let rect = egui::Rect::from_min_max(
                    egui::pos2(tree_area.min.x + inset, tree_area.max.y),
                    egui::pos2(tree_area.max.x - inset, widget_area.max.y),
                );
                status::gutter(ui.ctx(), rect, &status)
            }
            StatusStyle::Pill => {
                // The scene tile already keeps the window margin, so the pill sits
                // in its corner, in line with the panes' own edges.
                let anchor = scene_rect.right_bottom();
                status::pill(ui.ctx(), anchor, &status)
            }
            StatusStyle::Hidden => status::StatusResponse::default(),
        };
        match status_res.open {
            Some(status::Open::Pane(pane)) => reveal_pane(ui.ctx(), &mut state.view_toggles, pane),
            Some(status::Open::Settings(title)) => {
                widget::settings::open_section(ui.ctx(), &title);
                reveal_pane(ui.ctx(), &mut state.view_toggles, Pane::Settings);
            }
            None => (),
        }
        if let (true, Some(head)) = (status_res.toggle_mute, &head) {
            let head_state = state.open_heads.entry(head.clone()).or_default();
            head_state.muted = !head_state.muted;
        }
        if let Some(rect) = header_rect {
            let title = head.as_ref().map(ToString::to_string).unwrap_or_default();
            let dot = patch_only
                .then(|| status::color(problems, ui.visuals()))
                .flatten();
            let ext_panes = ext_pane_entries(&self);
            let view = &mut state.view_toggles;
            let responses = &mut response.responses;
            let w = window.unwrap_or_default();
            let action = decor::header(ui.ctx(), rect, &title, w, dot, view, responses, &ext_panes);
            window_actions.extend(action);
        }
        if window.is_some_and(|w| !w.maximized) {
            window_actions.extend(decor::resize_edges(ui.ctx(), widget_area, window_radius));
            let color = style::color(ui, Role::WindowOutline);
            let stroke = egui::Stroke::new(state.style.shape.window_outline, color);
            decor::outline(ui.ctx(), widget_area, window_radius, stroke);
        }
        for action in window_actions {
            response.responses.push(None, action);
        }
        let toast_margin = match status_style {
            StatusStyle::Pill => inset * 2.0 + ui.spacing().interact_size.y,
            StatusStyle::Gutter | StatusStyle::Hidden => inset * 2.0,
        };
        toast::show(
            ui.ctx(),
            scene_rect.center_bottom() - egui::vec2(0.0, toast_margin),
        );
        for action in actions {
            match action {
                chrome::PaneAction::Hide(pane) => {
                    set_pane_visible(&mut state.view_toggles, &pane, false);
                }
                chrome::PaneAction::PopOut(tile_id, pane) => {
                    let view = &mut state.view_toggles;
                    detach_pane(view, &mut windowed, &mut tree.tiles, tile_id, &pane);
                }
                chrome::PaneAction::ShowOnScene(tile_id, id) => {
                    // The pane's graph moves to the scene. The scene's graph
                    // takes its place in the pane, or the pane closes.
                    let Some(head) = state.graphs.panes.remove(&id) else {
                        continue;
                    };
                    match state.graphs.main.replace(head.clone()) {
                        Some(old) => {
                            state.graphs.panes.insert(id, old);
                        }
                        None => {
                            tree.tiles.remove(tile_id);
                        }
                    }
                    if let Some(ix) = access.heads().iter().position(|h| *h == head) {
                        response.focused_head = ix;
                    }
                }
            }
        }

        // Capture the sidebar width and tray height from the laid-out tree,
        // which reflects any manual divider drag, to re-impose next frame.
        capture_fixed_sizes(&tree, state, tree_area);

        // Detect .gantz file drops globally rather than per pane, since the
        // pointer position may be unavailable during OS file drags on some
        // platforms.
        for drop in collect_gantz_file_drops(ui.ctx()) {
            response.responses.push(None, drop);
        }

        // Apply payloads that only affect the widget's own state, so
        // applications never see them. These are node palette toggling,
        // opening it to replace a node, and resetting the tile layout to its
        // default arrangement.
        for _ in response.responses.take::<OpenNodePalette>() {
            state.palette_replace = None;
            state.node_palette.toggle();
        }
        for (head, OpenReplacePalette(node)) in response.responses.take() {
            if let Some(head) = head {
                state.palette_replace = Some((head, node));
                state.node_palette.open();
            }
        }
        for _ in response.responses.take::<ResetTilesLayout>() {
            tree = create_tree();
            // Restore the default sidebar width and tray height too.
            state.sidebar_width = default_sidebar_width();
            state.tray_height = default_tray_height();
            // Restore default pane visibility, but keep the sidebar's open
            // state since the user just acted from within it.
            let sidebar_open = state.view_toggles.sidebar_open;
            state.view_toggles = ViewToggles::default();
            state.view_toggles.sidebar_open = sidebar_open;
        }
        for _ in response.responses.take::<OpenLogs>() {
            state.view_toggles.logs = true;
        }
        for (_, OpenGraphPane(head)) in response.responses.take() {
            if access.heads().contains(&head) && !state.graphs.in_pane(&head) {
                let id = state.graphs.open_pane(head);
                add_graph_pane(&mut tree, id);
            }
        }
        // Migrate node-view tiles past this frame's node deletions. They were
        // collected during traversal, since the live top-level tree was not
        // reachable then.
        for (head, reindex) in &response.node_view_reindexes {
            migrate_node_view_paths(&mut tree, head, reindex);
            migrate_windowed_node_views(&mut windowed, head, reindex);
        }
        // Add or focus a node-view tile in the top-level tree. The node's head
        // is the payload's head tag.
        for (head, view) in response.responses.take::<OpenNodeView>() {
            let Some(head) = head else { continue };
            // Skip if this view is already popped out into a window.
            let windowed_already = windowed.iter().any(
                |p| matches!(p, Pane::NodeView(np) if np.head == head && np.path == view.path),
            );
            if windowed_already {
                continue;
            }
            add_node_view_pane(&mut tree, head, view.path, view.ty_name);
        }

        // Reconcile the windowed set. A singleton whose toggle was turned back
        // on re-docks, so drop it here. Node views stay windowed until their
        // window is closed.
        windowed.retain(|p| {
            matches!(p, Pane::Graph(_) | Pane::NodeView(_))
                || !pane_is_visible(&state.view_toggles, p)
        });

        // Redock and close intents queued by a native host between frames.
        // Its OS-window buttons call `redock_windowed_pane` and
        // `close_windowed_pane`.
        let mut redock: Vec<Pane> = drain_pending(ui.ctx(), pending_redock_id());
        let close: Vec<Pane> = drain_pending(ui.ctx(), pending_close_id());

        // In the egui-window backend, draw each windowed pane as a floating
        // `egui::Window`. Its title-bar close returns the pane to the tile
        // tree. Under `HostNative` the host draws them as OS windows instead.
        if self.pane_window_mode == PaneWindowMode::EguiWindow {
            for pane in &mut windowed {
                let title = pane_title(&self, &*access, &state.graphs, focused_head, pane);
                let mut open = true;
                let min = pane_min_size(pane);
                egui::Window::new(title)
                    .id(egui::Id::new(("gantz-windowed-pane", pane_key(pane))))
                    .min_size(min)
                    .open(&mut open)
                    .show(ui.ctx(), |ui| {
                        // Namespace child ids so a windowed pane never collides
                        // with its briefly still-visible docked instance.
                        ui.push_id(("gantz-windowed", pane_key(pane)), |ui| {
                            let mut cx = PaneCtx {
                                gantz: &mut self,
                                state: &mut *state,
                                access: &mut *access,
                                focused_head,
                                response: &mut response,
                            };
                            render_pane(&mut cx, ui, pane);
                        });
                    });
                if !open {
                    redock.push(pane.clone());
                }
            }
        }

        // Apply redocks, which return to the tree, and closes, which destroy
        // node views. Singletons have no destroy, so re-dock them.
        for pane in redock {
            let key = pane_key(&pane);
            windowed.retain(|p| pane_key(p) != key);
            redock_pane(&mut state.view_toggles, &mut tree, pane);
        }
        for pane in close {
            let key = pane_key(&pane);
            windowed.retain(|p| pane_key(p) != key);
            if !matches!(pane, Pane::Graph(_) | Pane::NodeView(_)) {
                redock_pane(&mut state.view_toggles, &mut tree, pane);
            }
        }

        // Report the final windowed set so a native host can match its OS windows.
        response.windowed_panes = windowed
            .iter()
            .map(|pane| WindowedPane {
                pane: pane.clone(),
                title: pane_title(&self, &*access, &state.graphs, focused_head, pane),
            })
            .collect();

        // Keep the graph panes and their placement in step. A pane closes with
        // its graph, and a placement goes with its pane.
        let docked: HashSet<u64> = tree
            .tiles
            .tiles()
            .filter_map(|tile| match tile {
                egui_tiles::Tile::Pane(Pane::Graph(id)) => Some(*id),
                _ => None,
            })
            .collect();
        let shown =
            |id: u64| docked.contains(&id) || windowed.iter().any(|p| *p == Pane::Graph(id));
        state.graphs.retain(access.heads(), shown);
        let orphans: Vec<egui_tiles::TileId> = tree
            .tiles
            .iter()
            .filter_map(|(tile_id, tile)| match tile {
                egui_tiles::Tile::Pane(Pane::Graph(id)) if !state.graphs.panes.contains_key(id) => {
                    Some(*tile_id)
                }
                _ => None,
            })
            .collect();
        for tile_id in orphans {
            tree.tiles.remove(tile_id);
        }
        windowed.retain(|p| match p {
            Pane::Graph(id) => state.graphs.panes.contains_key(id),
            _ => true,
        });

        store_tree(ui.ctx(), tree_id, &tree);
        store_ron(ui.ctx(), windowed_panes_id(), &windowed);

        response
    }

    /// Render a single popped-out pane into `ui`, typically a native host's
    /// OS-window egui context under [`PaneWindowMode::HostNative`].
    ///
    /// Mirrors [`Self::show`]'s per-pane rendering. The returned
    /// [`GantzResponse`] carries this pane's `changed_heads` and payloads.
    /// Apply it exactly like `show`'s response.
    pub fn render_windowed_pane<'s, Access>(
        mut self,
        state: &'s mut GantzState,
        focused_head: usize,
        access: &'s mut Access,
        pane: &mut Pane,
        ui: &mut egui::Ui,
    ) -> GantzResponse
    where
        's: 'a,
        Access: HeadAccess,
    {
        style::apply(ui.ctx(), &state.style);
        state.keymap.publish(ui.ctx());
        let mut response = GantzResponse::new(focused_head);
        let mut cx = PaneCtx {
            gantz: &mut self,
            state,
            access,
            focused_head,
            response: &mut response,
        };
        render_pane(&mut cx, ui, pane);
        response
    }
}

impl<'a, 's, Access> egui_tiles::Behavior<Pane> for TreeBehaviour<'a, 's, Access>
where
    Access: HeadAccess,
{
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        let cx = &self.cx;
        pane_title(cx.gantz, cx.access, &cx.state.graphs, cx.focused_head, pane).into()
    }

    fn on_tab_button(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
        button_response: egui::Response,
    ) -> egui::Response {
        // Right-click a tab for pane actions. Hideable panes offer hide. Any
        // pane but the graph scene offers pop out into a window.
        let pane = match tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(pane))
                if pane_is_hideable(pane) || pane_is_poppable(pane) =>
            {
                pane.clone()
            }
            _ => return button_response,
        };
        button_response.context_menu(|ui| {
            if pane_is_hideable(&pane) && ui.button("hide").clicked() {
                set_pane_visible(&mut self.cx.state.view_toggles, &pane, false);
                ui.close();
            }
            if pane_is_poppable(&pane) && ui.button("pop out to window").clicked() {
                detach_pane(
                    &mut self.cx.state.view_toggles,
                    self.windowed,
                    tiles,
                    tile_id,
                    &pane,
                );
                ui.close();
            }
        });
        button_response
    }

    fn is_tab_closable(
        &self,
        tiles: &egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        // The tray panes and detached node views get a close button. The rest
        // are toggled via the Panes settings or the tab right-click menu.
        matches!(
            tiles.get_pane(&tile_id),
            Some(
                Pane::Logs
                    | Pane::Steel
                    | Pane::GuiPreview
                    | Pane::GuiTree
                    | Pane::Graph(_)
                    | Pane::NodeView(_)
            )
        )
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        match tiles.get_pane(&tile_id) {
            // Node views are user-created tiles. Closing removes them entirely.
            Some(Pane::Graph(_) | Pane::NodeView(_)) => true,
            // Hide other panes via their toggle so they can be reopened, rather
            // than letting egui_tiles remove the tile from the tree.
            Some(pane) => {
                let pane = pane.clone();
                set_pane_visible(&mut self.cx.state.view_toggles, &pane, false);
                false
            }
            None => false,
        }
    }

    fn tab_ui(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        ui: &mut egui::Ui,
        id: egui::Id,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        // A pane alone in its header shows its title. Panes that share a
        // header show pills, the active one filled.
        let (count, first, parent) = self
            .headers
            .get(&tile_id)
            .map_or((1, true, None), |&(count, first, parent)| {
                (count, first, Some(parent))
            });
        if first {
            ui.add_space(chrome::HEADER_INSET);
            // The pane in the tree's top-left corner leads with the logo.
            if parent.is_some() && parent == self.logo_tile {
                let ext_panes = ext_pane_entries(self.cx.gantz);
                let cx = &mut self.cx;
                let view = &mut cx.state.view_toggles;
                decor::logo_button(ui, None, view, &mut cx.response.responses, &ext_panes);
                ui.add_space(4.0);
            }
        }
        let (title, sub) = match tiles.get_pane(&tile_id) {
            Some(pane) => {
                let cx = &self.cx;
                pane_label(cx.gantz, cx.access, &cx.state.graphs, cx.focused_head, pane)
            }
            None => (
                self.tab_title_for_tile(tiles, tile_id).text().to_string(),
                None,
            ),
        };
        let (look, sub) = match count > 1 {
            true => (widget::tab::TabLook::Pill, None),
            false => (widget::tab::TabLook::Title, sub),
        };
        let mut tab = widget::Tab::new(title, id)
            .look(look)
            .sub(sub)
            .active(state.active)
            .closable(state.closable);
        // The GUI panes pick the `gui` role they show from a badge on their
        // tab, so their bodies stay free of controls.
        let badge = match tiles.get_pane(&tile_id) {
            Some(Pane::GuiPreview | Pane::GuiTree) => {
                gui_role_badge(self.cx.access, self.cx.focused_head, ui.ctx())
            }
            _ => None,
        };
        if let Some((_, _, role)) = &badge {
            tab = tab.badge(format!("[{}]", role.as_str()));
        }
        let res = tab.show(ui);
        if let (Some((head, roles, role)), Some(badge_res)) = (badge, &res.badge) {
            egui::Popup::menu(badge_res)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    for r in roles {
                        if ui.selectable_label(r == role, r.as_str()).clicked() {
                            set_gui_role(ui.ctx(), &head, r);
                            ui.close();
                        }
                    }
                });
        }
        if res.close.is_some_and(|r| r.clicked()) && self.on_tab_close(tiles, tile_id) {
            tiles.remove(tile_id);
        }
        // Preserve the right-click-to-hide menu.
        self.on_tab_button(tiles, tile_id, res.tab)
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> egui::Color32 {
        // The header sits on the pane's rounded ground. See `chrome`.
        egui::Color32::TRANSPARENT
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        chrome::HEADER_HEIGHT
    }

    fn tab_bar_hline_stroke(&self, _visuals: &egui::Visuals) -> egui::Stroke {
        egui::Stroke::NONE
    }

    fn top_bar_right_ui(
        &mut self,
        tiles: &egui_tiles::Tiles<Pane>,
        ui: &mut egui::Ui,
        tile_id: egui_tiles::TileId,
        tabs: &egui_tiles::Tabs,
        _scroll_offset: &mut f32,
    ) {
        let Some(active) = tabs.active else { return };
        let Some(pane) = tiles.get_pane(&active) else {
            return;
        };
        // The active pane's actions show while its pane is hovered or
        // focused, so a quiet header stays quiet. They keep their space while
        // hidden, so the tabs beside them stay still.
        let hovered = tiles
            .rect(tile_id)
            .zip(ui.ctx().pointer_hover_pos())
            .is_some_and(|(rect, pos)| rect.contains(pos));
        let shown = hovered || self.focus == Some(tile_id);
        ui.add_space(chrome::HEADER_INSET - 2.0);
        let actions = match shown {
            true => egui::UiBuilder::new(),
            false => egui::UiBuilder::new().invisible(),
        };
        ui.scope_builder(actions, |ui| {
            if pane_is_hideable(pane)
                && chrome::icon_button(ui, chrome::Icon::Hide, "hide").clicked()
            {
                self.actions.push(chrome::PaneAction::Hide(pane.clone()));
            }
            if pane_is_poppable(pane)
                && chrome::icon_button(ui, chrome::Icon::PopOut, "pop out to a window").clicked()
            {
                self.actions
                    .push(chrome::PaneAction::PopOut(active, pane.clone()));
            }
            if let Pane::Graph(id) = pane {
                let hint = "show on the scene, in place of the graph there";
                if chrome::icon_button(ui, chrome::Icon::Scene, hint).clicked() {
                    self.actions
                        .push(chrome::PaneAction::ShowOnScene(active, *id));
                }
            }
        });
    }

    fn resize_stroke(
        &self,
        style: &egui::Style,
        resize_state: egui_tiles::ResizeState,
    ) -> egui::Stroke {
        separator_stroke(&self.cx.state.style, style, resize_state)
    }

    fn is_container_resizable(
        &self,
        _tiles: &egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        // The window corner's row keeps its height. See `corner::reserve`.
        tile_id != corner::column_id()
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        self.cx.state.style.shape.separator_width
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<Pane>,
        _tile_id: egui_tiles::TileId,
        _state: &egui_tiles::TabState,
    ) -> egui::Stroke {
        egui::Stroke::NONE
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        // The tree is simplified manually before `tree.ui`. See `simplify_tree`.
        egui_tiles::SimplificationOptions::OFF
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut Pane,
    ) -> egui_tiles::UiResponse {
        // A graph-tab click while rendering changes `cx.focused_head`.
        render_pane(&mut self.cx, ui, pane);
        egui_tiles::UiResponse::None
    }
}

impl widget::node_palette::Command for NodeTyCmd<'_> {
    fn text(&self) -> &str {
        self.name
    }

    fn description(&self) -> Option<std::borrow::Cow<'static, str>> {
        self.env.node_description(self.name)
    }

    fn info_ui(&self, ui: &mut egui::Ui) {
        crate::node_info_ui(&self.env.command_info(self.name), ui);
    }

    fn group(&self) -> Option<std::borrow::Cow<'static, str>> {
        let name = self.name;
        let group = if name == NESTED_GRAPH_TYPE {
            "new graph"
        } else if name.starts_with('~') {
            "signal"
        } else if name.starts_with("pat.") {
            "pattern"
        } else if name.starts_with("ui.") {
            "gui"
        } else if name.starts_with("demo.") {
            "demos"
        } else {
            "nodes"
        };
        Some(group.into())
    }
}

impl Clone for NodeTyCmd<'_> {
    fn clone(&self) -> Self {
        Self {
            env: self.env,
            name: self.name,
        }
    }
}

impl Copy for NodeTyCmd<'_> {}
