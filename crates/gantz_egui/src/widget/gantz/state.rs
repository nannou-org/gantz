//! The widget's persistent state and configuration, and its egui memory persistence.

use super::status::StatusStyle;
use crate::{
    Keymap,
    widget::{self, GraphSceneState, graph_scene},
};
use std::collections::{BTreeMap, HashMap};

/// All state for the widget.
#[derive(serde::Deserialize, serde::Serialize)]
pub struct GantzState {
    /// State for each open head.
    #[serde(serialize_with = "gantz_ca::serde_sorted::serialize_map")]
    pub open_heads: OpenHeadStates,
    pub view_toggles: ViewToggles,
    #[serde(default, alias = "command_palette")]
    pub node_palette: widget::NodePalette,
    /// The node that the open node palette replaces, with its head. `None`
    /// while the palette creates nodes. Not persisted, since a node index
    /// can go stale.
    #[serde(skip)]
    pub palette_replace: Option<(gantz_ca::Head, graph_scene::NodeIndex)>,
    /// Global auto-layout parameters. These are the non-flow `egui_graph`
    /// layout params. Flow stays per-head in [`OpenHeadState::layout_flow`].
    #[serde(default)]
    pub layout_config: LayoutConfig,
    /// Global interactive-scene parameters for the dot grid, drag snapping
    /// and snap-align. They mirror the per-frame `egui_graph::Graph` builder
    /// options and apply to every open head.
    #[serde(default)]
    pub scene_config: SceneConfig,
    /// Which panes scroll to follow the node selection. Edited in the
    /// Settings Global subtab.
    #[serde(default)]
    pub follow_selection: FollowSelectionConfig,
    /// The theme preference, palettes, roles and shape, applied to
    /// every gantz context. See [`crate::style`]. Edited in the Settings
    /// Style subtab.
    #[serde(default)]
    pub style: crate::StyleConfig,
    /// The command keyboard shortcuts. The single source of truth for editor
    /// command bindings. See [`crate::keybind`]. Edited in the Settings
    /// Keybinds subtab.
    #[serde(default)]
    pub keymap: Keymap,
    /// User-editable collaboration configuration, edited in the Settings
    /// Collab subtab.
    #[serde(default)]
    pub collab: crate::collab::CollabConfig,
    /// How graph merges resolve conflicts. Edited via the merge row's "⛭"
    /// menu in the Graph Config pane.
    #[serde(default)]
    pub merge_resolutions: gantz_ca::merge::Resolutions,
    /// Per-head redo stacks for undo and redo.
    #[serde(default, serialize_with = "gantz_ca::serde_sorted::serialize_map")]
    pub redo_stacks: HashMap<gantz_ca::Head, Vec<gantz_ca::CommitAddr>>,
    /// Per-head stepping state for session undo and redo by revert commit.
    /// See [`crate::ops::session_undo`]. Lives and migrates beside
    /// [`Self::redo_stacks`].
    #[serde(default, serialize_with = "gantz_ca::serde_sorted::serialize_map")]
    pub undo_cursors: HashMap<gantz_ca::Head, crate::ops::RevertCursor>,
    /// The sidebar width in pixels. It is fixed, not proportional, so it
    /// survives window resizes. Dragging the divider updates it.
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    /// The bottom tray's pixel height, maintained across window resizes.
    #[serde(default = "default_tray_height")]
    pub tray_height: f32,
    /// Last-seen size of each pane popped out into its own OS window, keyed by
    /// [`pane_key`], so a native host can restore it across sessions. Only the
    /// native window backend populates this. Web `egui::Window`s persist
    /// their own geometry in egui memory.
    ///
    /// [`pane_key`]: super::pane_key
    #[serde(default)]
    pub windowed_geometry: HashMap<String, PaneWindowGeometry>,
    /// The main graph on the scene and the graphs open in graph panes.
    #[serde(default)]
    pub graphs: GraphPlacement,
}

/// Where the open graphs show. The main graph fills the scene, and any graph
/// may also open in a graph pane. A graph in a pane is never the main graph.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct GraphPlacement {
    /// The graph on the scene. See [`Self::resolve_main`].
    pub main: Option<gantz_ca::Head>,
    /// The graph of each graph pane, keyed by the id its
    /// [`Pane::Graph`][super::Pane::Graph] carries.
    pub panes: BTreeMap<u64, gantz_ca::Head>,
    /// The id the next graph pane takes.
    next_pane: u64,
}

/// The persisted geometry of a pane's pop-out window, in logical pixels.
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
pub struct PaneWindowGeometry {
    pub width: f32,
    pub height: f32,
}

pub(super) fn default_sidebar_width() -> f32 {
    300.0
}

pub(super) fn default_tray_height() -> f32 {
    300.0
}

pub type OpenHeadStates = HashMap<gantz_ca::Head, OpenHeadState>;

/// State associated with a single open graph.
#[derive(serde::Deserialize, serde::Serialize)]
pub struct OpenHeadState {
    /// State associated with the `GraphScene` widget.
    pub scene: GraphSceneState,
    /// The per-head flow direction used when auto-layout is invoked.
    #[serde(default = "default_layout_flow")]
    pub layout_flow: egui::Direction,
    /// Whether the head's audio output is muted.
    #[serde(default)]
    pub muted: bool,
}

fn default_layout_flow() -> egui::Direction {
    GantzState::DEFAULT_DIRECTION
}

impl Default for OpenHeadState {
    fn default() -> Self {
        Self {
            scene: GraphSceneState::default(),
            layout_flow: GantzState::DEFAULT_DIRECTION,
            muted: false,
        }
    }
}

/// Global auto-layout parameters, mirroring the non-flow fields of
/// [`egui_graph::LayoutParams`]. Flow stays per-head. See
/// [`OpenHeadState::layout_flow`]. These apply to every head's auto-layout.
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct LayoutConfig {
    /// The gap between adjacent layers along the flow direction.
    #[serde(default = "default_layer_gap")]
    pub layer_gap: f32,
    /// The gap between adjacent nodes within a layer.
    #[serde(default = "default_node_gap")]
    pub node_gap: f32,
    /// The gap between disconnected components of the graph.
    #[serde(default = "default_component_gap")]
    pub component_gap: f32,
    /// Whether the layout accounts for the socket each edge connects to.
    #[serde(default = "default_socket_aware")]
    pub socket_aware: bool,
}

fn default_layer_gap() -> f32 {
    egui_graph::LayoutParams::DEFAULT_LAYER_GAP
}

fn default_node_gap() -> f32 {
    egui_graph::LayoutParams::DEFAULT_NODE_GAP
}

fn default_component_gap() -> f32 {
    egui_graph::LayoutParams::DEFAULT_COMPONENT_GAP
}

fn default_socket_aware() -> bool {
    true
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            layer_gap: default_layer_gap(),
            node_gap: default_node_gap(),
            component_gap: default_component_gap(),
            socket_aware: default_socket_aware(),
        }
    }
}

impl LayoutConfig {
    /// Build [`egui_graph::LayoutParams`] from these globals plus a per-head
    /// `flow` direction.
    pub fn to_params(&self, flow: egui::Direction) -> egui_graph::LayoutParams {
        egui_graph::LayoutParams::new(flow)
            .layer_gap(self.layer_gap)
            .node_gap(self.node_gap)
            .component_gap(self.component_gap)
            .socket_aware(self.socket_aware)
    }
}

/// Global interactive-scene configuration for the dot grid, drag snapping,
/// snap-align and edge shape. It mirrors the per-frame options on
/// [`egui_graph::Graph`] and its edges and applies to every open head, like
/// [`LayoutConfig`].
#[derive(Clone, Copy, Default, serde::Deserialize, serde::Serialize)]
pub struct SceneConfig {
    #[serde(default)]
    pub grid: GridConfig,
    #[serde(default)]
    pub snap: SnapConfig,
    #[serde(default)]
    pub align: AlignConfig,
    #[serde(default)]
    pub edge: EdgeConfig,
}

/// The shape of the edges drawn between sockets.
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct EdgeConfig {
    /// How far each edge curves from a straight line, from `0.0` (straight)
    /// to `1.0`. See [`egui_graph::edge::Edge::curvature_factor`].
    #[serde(default = "default_edge_curvature")]
    pub curvature: f32,
}

fn default_edge_curvature() -> f32 {
    egui_graph::bezier::Cubic::DEFAULT_CURVATURE
}

impl Default for EdgeConfig {
    fn default() -> Self {
        Self {
            curvature: default_edge_curvature(),
        }
    }
}

/// The dot grid drawn behind the graph. See [`egui_graph::Graph::dot_grid`].
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct GridConfig {
    /// Whether the dot grid is drawn.
    #[serde(default = "default_grid_show")]
    pub show: bool,
    /// The base spacing of the dot grid, in graph-space units.
    #[serde(default = "default_grid_step")]
    pub step: f32,
}

fn default_grid_show() -> bool {
    true
}

fn default_grid_step() -> f32 {
    20.0
}

impl Default for GridConfig {
    fn default() -> Self {
        Self {
            show: default_grid_show(),
            step: default_grid_step(),
        }
    }
}

/// How a dragged node's position is snapped.
#[derive(Clone, Copy, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum SnapMode {
    /// Snap to the nearest unit point, which is effectively free.
    #[default]
    Point,
    /// Snap to a relative fraction of the dot grid. See
    /// [`SnapConfig::grid_ratio`].
    Grid,
}

/// Drag snapping configuration. See [`egui_graph::Graph::snap`].
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct SnapConfig {
    #[serde(default)]
    pub mode: SnapMode,
    /// In [`SnapMode::Grid`], the snap step relative to the grid step, so
    /// `snap_step = grid.step * grid_ratio`. `1.0` snaps to the full grid,
    /// `0.5` to half and `0.25` to quarter.
    #[serde(default = "default_grid_ratio")]
    pub grid_ratio: f32,
}

fn default_grid_ratio() -> f32 {
    1.0
}

impl Default for SnapConfig {
    fn default() -> Self {
        Self {
            mode: SnapMode::default(),
            grid_ratio: default_grid_ratio(),
        }
    }
}

/// Drag-time snap-align configuration. See [`egui_graph::Graph::align`].
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct AlignConfig {
    /// Whether a dragged node snap-aligns to its neighbours.
    #[serde(default = "default_align_enabled")]
    pub enabled: bool,
    /// Align to neighbours' left/right/top/bottom edges.
    #[serde(default = "default_align_edges")]
    pub edges: bool,
    /// Align to neighbours' horizontal/vertical centres.
    #[serde(default = "default_align_centers")]
    pub centers: bool,
}

fn default_align_enabled() -> bool {
    true
}

fn default_align_edges() -> bool {
    true
}

fn default_align_centers() -> bool {
    false
}

impl Default for AlignConfig {
    fn default() -> Self {
        Self {
            enabled: default_align_enabled(),
            edges: default_align_edges(),
            centers: default_align_centers(),
        }
    }
}

impl SceneConfig {
    /// Apply the grid, snap and align options onto an [`egui_graph::Graph`]
    /// builder. The snap step derives from the mode. [`SnapMode::Point`]
    /// snaps to unit points and [`SnapMode::Grid`] to a fraction of the grid.
    pub fn apply(self, graph: egui_graph::Graph) -> egui_graph::Graph {
        let snap_step = match self.snap.mode {
            SnapMode::Point => 1.0,
            SnapMode::Grid => self.grid.step * self.snap.grid_ratio,
        };
        graph
            // gantz owns zoom persistence. The camera stores centre and zoom,
            // and the scene rect is rebuilt from it each frame against the
            // live viewport. Use `MaintainView` so `egui_graph` does not
            // rescale the rect on resize as well and double-adjust the zoom.
            .resize_behavior(egui_graph::ResizeBehavior::MaintainView)
            .dot_grid(self.grid.show)
            .dot_grid_step(self.grid.step)
            .snap(Some(egui_graph::Snap::Round))
            .snap_step(snap_step)
            .align(self.align.enabled)
            .align_targets(egui_graph::AlignTargets {
                edges: self.align.edges,
                centers: self.align.centers,
            })
    }
}

/// Which panes scroll to keep the selected node in view when the selection
/// changes.
#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct FollowSelectionConfig {
    /// The Node Inspector pane scrolls to the selected node's entry.
    #[serde(default = "default_follow_selection")]
    pub inspector: bool,
    /// The Steel pane scrolls to the selected node's code.
    #[serde(default = "default_follow_selection")]
    pub steel: bool,
}

fn default_follow_selection() -> bool {
    true
}

impl Default for FollowSelectionConfig {
    fn default() -> Self {
        Self {
            inspector: default_follow_selection(),
            steel: default_follow_selection(),
        }
    }
}

/// Load a value persisted in egui memory as a RON `String`.
///
/// Values are stored as a RON `String` rather than typed. A `String`'s
/// `TypeId` is stable across recompiles, whereas the `TypeId` of a type like
/// `Tree<Pane>` changes whenever this crate is rebuilt. Storing the typed
/// value would leave a stale copy behind in egui's per-type persisted map on
/// every dev build. egui never evicts entries of a type the running build no
/// longer uses. The persisted memory would bloat without bound.
pub(super) fn load_ron<T: serde::de::DeserializeOwned>(
    ctx: &egui::Context,
    id: egui::Id,
) -> Option<T> {
    let ron = ctx.memory_mut(|m| m.data.get_persisted::<String>(id))?;
    ron::from_str(&ron).ok()
}

/// Persist a value to egui memory as a RON `String`. See [`load_ron`].
pub(super) fn store_ron<T: serde::Serialize>(ctx: &egui::Context, id: egui::Id, value: &T) {
    if let Ok(ron) = ron::to_string(value) {
        ctx.memory_mut(|m| m.data.insert_persisted(id, ron));
    }
}

/// Load a tile tree from egui's persisted memory. See [`load_ron`].
pub(super) fn load_tree<P: serde::de::DeserializeOwned>(
    ctx: &egui::Context,
    id: egui::Id,
) -> Option<egui_tiles::Tree<P>> {
    load_ron(ctx, id)
}

/// Persist a tile tree to egui memory as a RON `String`. See [`load_ron`].
pub(super) fn store_tree<P: serde::Serialize>(
    ctx: &egui::Context,
    id: egui::Id,
    tree: &egui_tiles::Tree<P>,
) {
    store_ron(ctx, id, tree)
}

/// egui temp-memory flag id for a pending "clear egui memory" request.
pub(super) fn clear_egui_memory_id() -> egui::Id {
    egui::Id::new("gantz-clear-egui-memory-request")
}

/// Request that egui's persisted memory be cleared at the start of the next
/// `Gantz::show`.
///
/// A recovery tool for when egui's persisted memory accumulates stale state.
/// It discards the UI memory but leaves the graph registry and other app
/// storage untouched. Deferring to the next frame keeps the clear
/// deterministic. It runs before any persisted UI state is loaded, rather
/// than racing this frame's widgets.
pub fn request_clear_egui_memory(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(clear_egui_memory_id(), true));
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ViewToggles {
    /// Whether the sidebar is open. The hamburger toggles it.
    pub sidebar_open: bool,
    pub graphs: bool,
    pub history: bool,
    pub settings: bool,
    pub logs: bool,
    pub node_inspector: bool,
    pub perf_gui: bool,
    pub perf_vm: bool,
    pub steel: bool,
    pub gui_preview: bool,
    pub gui_tree: bool,
    pub graph_config: bool,
    /// Where the status line shows.
    pub status: StatusStyle,
    /// Whether only the graphs show. Every pane with a toggle hides, along
    /// with the tab strip and the status line, until patch-only ends.
    pub patch_only: bool,
    /// Per-[`Pane::Ext`] visibility, keyed by the provider key. A missing
    /// entry means hidden, matching the tray panes' default. Keys of
    /// long-gone providers linger harmlessly.
    ///
    /// [`Pane::Ext`]: super::Pane::Ext
    pub ext: BTreeMap<String, bool>,
}

impl Default for ViewToggles {
    fn default() -> Self {
        // The sidebar starts closed so a fresh launch shows only the graph
        // scene, but its content panes default to visible so that opening the
        // sidebar reveals the full arrangement. The Logs/Steel tray stays
        // hidden until toggled.
        Self {
            sidebar_open: false,
            graphs: true,
            history: true,
            settings: true,
            logs: false,
            node_inspector: true,
            perf_gui: false,
            perf_vm: false,
            steel: false,
            gui_preview: false,
            gui_tree: false,
            graph_config: true,
            status: StatusStyle::default(),
            patch_only: false,
            ext: BTreeMap::new(),
        }
    }
}

impl GantzState {
    pub const DEFAULT_DIRECTION: egui::Direction = egui::Direction::TopDown;

    /// Shorthand for initialising graph state with no initial layout, so the
    /// first pass determines the layout automatically.
    pub fn new() -> Self {
        Self::from_open_heads(Default::default())
    }

    pub fn from_open_heads(open_heads: OpenHeadStates) -> Self {
        Self {
            open_heads,
            node_palette: widget::NodePalette::default(),
            palette_replace: None,
            view_toggles: ViewToggles::default(),
            layout_config: LayoutConfig::default(),
            scene_config: SceneConfig::default(),
            follow_selection: FollowSelectionConfig::default(),
            style: Default::default(),
            keymap: Keymap::default(),
            collab: Default::default(),
            merge_resolutions: Default::default(),
            redo_stacks: HashMap::new(),
            undo_cursors: HashMap::new(),
            sidebar_width: default_sidebar_width(),
            tray_height: default_tray_height(),
            windowed_geometry: HashMap::new(),
            graphs: GraphPlacement::default(),
        }
    }

    /// Migrate GUI state when a head's identity changes.
    ///
    /// Moves the `open_heads` entry and the graph's placement from the old to the new
    /// head. When `clear_redo` is true, removes redo stacks for both keys,
    /// since a new edit commit invalidates them. Otherwise migrates the redo
    /// stack to the new key. Hosts call this for every change of a head's
    /// identity, and nothing else needs to migrate.
    pub fn migrate_head(&mut self, old: &gantz_ca::Head, new: &gantz_ca::Head, clear_redo: bool) {
        if let Some(state) = self.open_heads.remove(old) {
            self.open_heads.insert(new.clone(), state);
        }
        migrate_or_clear(&mut self.redo_stacks, old, new, clear_redo);
        migrate_or_clear(&mut self.undo_cursors, old, new, clear_redo);
        self.graphs.migrate(old, new);
    }
}

impl GraphPlacement {
    /// Open `head` in a new graph pane. Returns the pane's id.
    pub fn open_pane(&mut self, head: gantz_ca::Head) -> u64 {
        let id = self.next_pane;
        self.next_pane += 1;
        if self.main.as_ref() == Some(&head) {
            self.main = None;
        }
        self.panes.insert(id, head);
        id
    }

    /// Whether `head` shows in a graph pane.
    pub fn in_pane(&self, head: &gantz_ca::Head) -> bool {
        self.panes.values().any(|h| h == head)
    }

    /// The main graph: the focused head unless it is in a pane, otherwise the
    /// kept main graph while it is open, otherwise the first open head not in
    /// a pane. So focusing a graph pane leaves the scene as it is. `None`
    /// while every open head is in a pane.
    pub fn resolve_main(
        &mut self,
        heads: &[gantz_ca::Head],
        focused: usize,
    ) -> Option<gantz_ca::Head> {
        let main = {
            let free = |h: &&gantz_ca::Head| !self.in_pane(h);
            heads
                .get(focused)
                .filter(free)
                .or_else(|| {
                    self.main
                        .as_ref()
                        .filter(|h| heads.contains(h))
                        .filter(free)
                })
                .or_else(|| heads.iter().find(free))
                .cloned()
        };
        self.main = main.clone();
        main
    }

    /// Drop the panes whose graph is no longer open, and those that `keep`
    /// rejects by id.
    pub fn retain(&mut self, heads: &[gantz_ca::Head], mut keep: impl FnMut(u64) -> bool) {
        self.panes
            .retain(|&id, head| heads.contains(head) && keep(id));
    }

    /// Follow a graph to its new identity. See [`GantzState::migrate_head`].
    fn migrate(&mut self, old: &gantz_ca::Head, new: &gantz_ca::Head) {
        if self.main.as_ref() == Some(old) {
            self.main = Some(new.clone());
        }
        for head in self.panes.values_mut() {
            if head == old {
                *head = new.clone();
            }
        }
    }
}

/// Migrate one per-head map entry across a head-identity change. When
/// `clear`, a new edit invalidates it, so it is cleared for both keys.
/// Otherwise it moves to the new key.
fn migrate_or_clear<V>(
    map: &mut HashMap<gantz_ca::Head, V>,
    old: &gantz_ca::Head,
    new: &gantz_ca::Head,
    clear: bool,
) {
    if clear {
        map.remove(old);
        map.remove(new);
    } else if let Some(v) = map.remove(old) {
        map.insert(new.clone(), v);
    }
}

impl Default for GantzState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(name: &str) -> gantz_ca::Head {
        gantz_ca::Head::Branch(name.parse().unwrap())
    }

    // The main graph and a graph pane follow their heads to new identities,
    // and persist with the state.
    #[test]
    fn migrate_head_keeps_the_graph_placement() {
        let (a, b, c, d) = (head("a"), head("b"), head("c"), head("d"));
        let mut state = GantzState::new();
        state.graphs.main = Some(a.clone());
        let pane = state.graphs.open_pane(b.clone());
        state.migrate_head(&a, &c, false);
        state.migrate_head(&b, &d, false);
        assert_eq!(state.graphs.main, Some(c.clone()));
        assert_eq!(state.graphs.panes.get(&pane), Some(&d));
        let state: GantzState = ron::from_str(&ron::to_string(&state).unwrap()).unwrap();
        assert_eq!(state.graphs.main, Some(c));
        assert_eq!(state.graphs.panes.get(&pane), Some(&d));
    }

    // The main graph follows the focused head, but is never one shown in a
    // pane. Then it keeps the last main graph, or any open head not in a pane.
    #[test]
    fn main_graph_avoids_panes() {
        let (a, b, c) = (head("a"), head("b"), head("c"));
        let heads = [a.clone(), b.clone(), c.clone()];
        let mut graphs = GraphPlacement::default();
        assert_eq!(graphs.resolve_main(&heads, 1), Some(b.clone()));
        graphs.open_pane(b.clone());
        assert_eq!(graphs.resolve_main(&heads, 1), Some(a.clone()));
        // Focusing the pane's graph keeps the main graph.
        assert_eq!(graphs.resolve_main(&heads, 1), Some(a.clone()));
        graphs.open_pane(a.clone());
        assert_eq!(graphs.resolve_main(&heads, 0), Some(c.clone()));
        graphs.retain(&heads[1..], |_| true);
        assert!(!graphs.in_pane(&a));
        assert!(graphs.in_pane(&b));
    }

    // State persisted before the graph placement joined it still loads.
    #[test]
    fn state_without_graph_placement_loads() {
        let ron = ron::to_string(&GantzState::new()).unwrap();
        let start = ron.rfind(",graphs:").expect("the field");
        let ron = format!("{})", &ron[..start]);
        let state: GantzState = ron::from_str(&ron).unwrap();
        assert_eq!(state.graphs, GraphPlacement::default());
    }
}
