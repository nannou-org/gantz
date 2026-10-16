use crate::{
    Env, HeadAccess, OpenLogs, OpenNodePalette, OpenNodeView, OpenReplacePalette, ResetTilesLayout,
    node::NodeCodec,
    response::Responses,
    style::SeparatorConfig,
    widget::{self, graph_scene},
};
use std::collections::{HashMap, HashSet};

mod graph_tabs;
mod panes;
mod state;
mod tiles;
mod windowing;

use panes::{collect_gantz_file_drops, gui_role_badge, render_pane, set_gui_role};
pub use state::{
    AlignConfig, EdgeConfig, FollowSelectionConfig, GantzState, GridConfig, LayoutConfig,
    OpenHeadState, OpenHeadStates, PaneWindowGeometry, SceneConfig, SnapConfig, SnapMode,
    ViewToggles, request_clear_egui_memory,
};
use state::{
    clear_egui_memory_id, default_sidebar_width, default_tray_height, load_ron, load_tree,
    store_ron, store_tree,
};
pub use tiles::{NodeViewPane, Pane};
use tiles::{
    add_node_view_pane, capture_fixed_sizes, create_tree, impose_fixed_sizes, impose_min_sizes,
    migrate_node_view_paths, pane_is_hideable, pane_is_poppable, pane_is_visible, pane_min_size,
    pane_title, set_pane_visible, set_tile_visibility, simplify_tree, sync_ext_panes,
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
fn separator_stroke(
    cfg: SeparatorConfig,
    style: &egui::Style,
    resize_state: egui_tiles::ResizeState,
) -> egui::Stroke {
    let color = match resize_state {
        egui_tiles::ResizeState::Idle => cfg.color,
        egui_tiles::ResizeState::Hovering | egui_tiles::ResizeState::Dragging => {
            cfg.hover.unwrap_or(cfg.color)
        }
    };
    egui::Stroke::new(cfg.width, color.resolve(&style.visuals))
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
        }
    }

    /// Provide the open heads whose graph produces audio. Their tabs show a
    /// speaker that toggles [`OpenHeadState::muted`].
    pub fn audio_heads(mut self, heads: &'a HashSet<gantz_ca::Head>) -> Self {
        self.audio_heads = Some(heads);
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

        crate::style::apply(ui.ctx(), &state.style);

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

        // Simplify the tree, and ensure tabs are where they should be.
        simplify_tree(&mut tree, ui.ctx());

        // `available_rect_before_wrap` matches the root rect `Tree::ui` uses.
        let widget_area = ui.available_rect_before_wrap();
        // Raise any pane below its minimum before the fixed sizes are imposed,
        // so a clamped minimum is not captured as a shrunken sidebar or tray.
        impose_min_sizes(&mut tree, state.style.separator.width, widget_area);
        // Maintain a fixed sidebar width and tray height across window resizes
        // by imposing the stored pixel sizes on the share splits before layout.
        impose_fixed_sizes(&mut tree, state, widget_area);

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
        };
        tree.ui(&mut behaviour, ui);

        response.focused_head = behaviour.cx.focused_head;

        // Capture the sidebar width and tray height from the laid-out tree,
        // which reflects any manual divider drag, to re-impose next frame.
        capture_fixed_sizes(&tree, state, widget_area);

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
        windowed
            .retain(|p| matches!(p, Pane::NodeView(_)) || !pane_is_visible(&state.view_toggles, p));

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
                let title = pane_title(&self, &*access, focused_head, pane);
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
            if !matches!(pane, Pane::NodeView(_)) {
                redock_pane(&mut state.view_toggles, &mut tree, pane);
            }
        }

        // Report the final windowed set so a native host can match its OS windows.
        response.windowed_panes = windowed
            .iter()
            .map(|pane| WindowedPane {
                pane: pane.clone(),
                title: pane_title(&self, &*access, focused_head, pane),
            })
            .collect();

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
        crate::style::apply(ui.ctx(), &state.style);
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
        pane_title(self.cx.gantz, self.cx.access, self.cx.focused_head, pane).into()
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
            Some(Pane::Logs | Pane::Steel | Pane::GuiPreview | Pane::GuiTree | Pane::NodeView(_))
        )
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        match tiles.get_pane(&tile_id) {
            // Node views are user-created tiles. Closing removes them entirely.
            Some(Pane::NodeView(_)) => true,
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
        // Render with the shared `Tab` widget so the sidebar and tray tabs and
        // their small close button match the graph tabs.
        let title = self.tab_title_for_tile(tiles, tile_id);
        let mut tab = widget::Tab::new(title, id)
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

    fn tab_bar_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        // This matches the `CentralPanel` fill so that the color looks
        // continuous.
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

    fn formatted_kb_shortcut(&self, ctx: &egui::Context) -> Option<String> {
        self.env.command_formatted_kb_shortcut(ctx, self.name)
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
