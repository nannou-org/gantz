//! Egui integration for bevy_gantz.
//!
//! This crate provides:
//! - [`GantzEguiPlugin`], the Bevy plugin for the egui-based UI
//! - GUI state resources and observers
//! - The main `update` system for rendering the gantz GUI

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use bevy_gantz::Registry;
use bevy_gantz::head;
use gantz_ca as ca;
use std::collections::HashSet;
use std::ops::{Deref, DerefMut};

mod access;
pub mod base;
mod edit;
pub mod entities;
mod graphs;
mod gui;
mod history;
mod hooks;
mod io;
mod merge;
pub mod node;
#[cfg(not(target_arch = "wasm32"))]
pub mod pane_window;
pub mod reg;
mod remote;
mod response;
mod settings;
pub mod storage;
pub mod sugar;
#[cfg(test)]
mod tests;
#[cfg(not(target_arch = "wasm32"))]
mod theme;
mod view;
pub mod vm;

pub use access::{HeadAccess, OpenHeadViews};
pub use edit::{
    on_branch_node, on_copy_nodes, on_create_nested_graph, on_create_node, on_cut_nodes,
    on_drop_unknown_data, on_duplicate_nodes, on_inspect_edge, on_nest_nodes, on_paste,
    on_replace_node,
};
pub use entities::{NodeKindAppExt, NodeKinds};
use graphs::{on_new_graph, on_remove_name, on_set_demo, on_set_description};
pub(crate) use gui::handle_gantz_response;
pub use gui::{GantzInputs, update};
pub use history::{on_redo, on_undo};
use hooks::on_eval_entry_complete;
pub use hooks::{
    on_branch_created, on_branched_head_fork_nested, on_head_changed, on_head_closed,
    on_head_committed, on_head_committed_resync, on_head_opened,
};
pub use io::{
    ExportAllNamedEvent, ExportHeadEvent, ExportPaletteEvent, ExportStyleEvent, ImportFileEvent,
    ImportPaletteEvent, ImportStyleEvent, ImportTask, PaletteImportTask, ResetBaseGraphEvent,
    StyleImportTask, on_export_all_named, on_export_head, on_export_palette, on_export_style,
    on_import_file, on_import_palette, on_import_style, on_reset_base_graph,
};
use io::{
    on_import, on_reset_all_demos, on_set_base_source, poll_import_task, poll_palette_import_task,
    poll_style_import_task,
};
pub use merge::{on_merge_head, seed_view};
pub use node::{builtins, entrypoints};
pub use reg::{BuiltinNodes, GraphCache, env, lookup_node, prune_unused, refresh_cache};
pub use remote::{
    RenameHeadEvent, ResetHeadEvent, ResyncRefsEvent, SyncRemoteTip, on_rename_head, on_reset_head,
    on_resync_refs, on_sync_remote_tip,
};
pub use response::{
    DispatchFn, ForApp, ForHead, RegisterResponseExt, ResponseDispatchers, downcast_payload,
};
use response::{
    dispatch_branch_head, dispatch_close_head, dispatch_eval_entry, dispatch_export_all_named,
    dispatch_export_head, dispatch_export_palette, dispatch_export_style, dispatch_file_drop,
    dispatch_import_palette, dispatch_import_style, dispatch_open_head, dispatch_replace_head,
    dispatch_reset_base_graph, dispatch_state_written,
};
use settings::{on_compile_config, on_validate_change_tracking};
pub use sugar::BevySugar;
#[cfg(not(target_arch = "wasm32"))]
pub use theme::SystemTheme;
pub use view::{
    LayoutCommittedEvent, LayoutEdited, ViewPersistSet, persist_camera_and_seed, settle_layout,
};
pub use vm::{EntrypointFn, EntrypointFns};

/// Plugin providing egui-based UI for gantz.
///
/// This plugin:
/// - Initializes the GUI resources such as `GuiState` and `TraceCapture`
/// - Owns the typed side of the registry, [`GraphCache`] and [`BuiltinNodes`],
///   and keeps head VMs in sync with their compile inputs via [`vm::sync`]
/// - Registers the GUI state and response payload observers
/// - Runs the main GUI update system
pub struct GantzEguiPlugin {
    base_immutable: bool,
}

impl Default for GantzEguiPlugin {
    fn default() -> Self {
        Self {
            base_immutable: true,
        }
    }
}

impl GantzEguiPlugin {
    /// Whether base node graphs are immutable in the GUI.
    ///
    /// When `true`, the default, graphs for heads whose branch name appears
    /// in `BaseNames` are shown in view-only mode.
    ///
    /// Set to `false` for developer tools like `update-base` that need to
    /// edit base nodes.
    pub fn base_immutable(mut self, base_immutable: bool) -> Self {
        self.base_immutable = base_immutable;
        self
    }
}

impl Plugin for GantzEguiPlugin {
    fn build(&self, app: &mut App) {
        // Follow the system's light or dark preference.
        #[cfg(not(target_arch = "wasm32"))]
        theme::build(app);

        // Push the entrypoint providers. See `vm::EntrypointFns`.
        app.world_mut()
            .get_resource_or_init::<vm::EntrypointFns>()
            .0
            .push(Box::new(node::entrypoints));

        // The core base source. Domain plugins push their own the same way.
        app.world_mut()
            .get_resource_or_init::<base::BaseSources>()
            .0
            .push(gantz_edit::base::BaseSource {
                name: "gantz",
                bytes: gantz_base::BYTES,
                renames: gantz_base::RENAMES,
            });

        // Builtin GUI response payload dispatchers. Head-scoped payloads
        // arrive at the observers below as `ForHead<T>` events. The rest map
        // onto existing event types via custom dispatch fns. Observers that
        // edit a head's working graph commit it before returning, see
        // `head::WorkingGraph`, so no dispatch-side handling is needed.
        app.register_head_response::<gantz_egui::BranchNode>()
            .register_head_response::<gantz_egui::CopyNodes>()
            .register_head_response::<gantz_egui::CutNodes>()
            .register_head_response::<gantz_egui::DuplicateNodes>()
            .register_head_response::<gantz_egui::NestNodes>()
            .register_head_response::<gantz_egui::CreateNode>()
            .register_head_response::<gantz_egui::ReplaceNode>()
            .register_head_response::<gantz_egui::DropUnknownData>()
            .register_head_response::<gantz_egui::CreateNestedGraph>()
            .register_head_response::<gantz_egui::InspectEdge>()
            .register_head_response::<gantz_egui::MergeHead>()
            .register_head_response::<gantz_egui::Paste>()
            .register_head_response::<gantz_egui::Redo>()
            .register_head_response::<gantz_egui::Undo>()
            .register_response_with::<gantz_egui::EvalEntry>(dispatch_eval_entry)
            .register_response_with::<gantz_egui::StateWritten>(dispatch_state_written)
            .register_response_with::<gantz_egui::OpenHead>(dispatch_open_head)
            .register_response_with::<gantz_egui::ReplaceHead>(dispatch_replace_head)
            .register_response_with::<gantz_egui::CloseHead>(dispatch_close_head)
            .register_response_with::<gantz_egui::BranchHead>(dispatch_branch_head)
            .register_response_with::<gantz_egui::widget::gantz::FileDrop>(dispatch_file_drop)
            .register_response_with::<gantz_egui::ResetBaseGraph>(dispatch_reset_base_graph)
            .register_response_with::<gantz_egui::ExportHead>(dispatch_export_head)
            .register_response_with::<gantz_egui::ExportAllNamed>(dispatch_export_all_named)
            .register_response_with::<gantz_egui::ExportStyle>(dispatch_export_style)
            .register_response_with::<gantz_egui::ImportStyle>(dispatch_import_style)
            .register_response_with::<gantz_egui::ImportPalette>(dispatch_import_palette)
            .register_response_with::<gantz_egui::ExportPalette>(dispatch_export_palette)
            .register_app_response::<gantz_egui::NewGraph>()
            .register_app_response::<gantz_egui::widget::WindowAction>()
            .register_app_response::<gantz_egui::RemoveName>()
            .register_app_response::<gantz_egui::Import>()
            .register_app_response::<gantz_egui::SetDemo>()
            .register_app_response::<gantz_egui::SetDescription>()
            .register_app_response::<gantz_egui::SetBaseSource>()
            .register_app_response::<gantz_egui::ResetAllDemos>()
            .register_app_response::<gantz_core::compile::Config>()
            .register_app_response::<gantz_egui::ValidateChangeTracking>();

        app.insert_resource(BaseImmutable(self.base_immutable))
            .init_resource::<GraphCache>()
            .init_resource::<BuiltinNodes>()
            .init_resource::<BaseNames>()
            .init_resource::<NodeKinds>()
            .add_node_kind::<node::Await>()
            .add_node_kind::<node::LoadBang>()
            .add_node_kind::<node::TickBang>()
            .add_node_kind::<node::UpdateBang>()
            .init_resource::<base::BaseNameSources>()
            .init_resource::<GuiState>()
            .init_resource::<TraceCapture>()
            .init_resource::<PerfGui>()
            .init_resource::<WindowedPanesRequested>()
            .init_resource::<SettingsTabs>()
            .init_resource::<ExtPanes>()
            .init_resource::<RefExtUis>()
            .init_resource::<EdgeStyles>()
            .init_resource::<AudioHeads>()
            .init_resource::<StatusItems>()
            // GUI state observers
            .add_observer(on_head_opened)
            .add_observer(on_head_changed)
            .add_observer(on_head_closed)
            .add_observer(on_branch_created)
            .add_observer(on_head_committed)
            .add_observer(on_head_committed_resync)
            .add_observer(on_branched_head_fork_nested)
            // VM timing observer
            .add_observer(on_eval_entry_complete)
            // Marker refresh dirty flag, see `node::gui_refresh`
            .add_observer(node::gui_refresh::mark_gui_dirty_on_push)
            // GUI response payload observers
            .add_observer(on_create_node)
            .add_observer(on_replace_node)
            .add_observer(on_drop_unknown_data)
            .add_observer(on_create_nested_graph)
            .add_observer(on_branch_node)
            .add_observer(on_inspect_edge)
            .add_observer(on_copy_nodes)
            .add_observer(on_cut_nodes)
            .add_observer(on_nest_nodes)
            .add_observer(on_duplicate_nodes)
            .add_observer(on_merge_head)
            .add_observer(on_sync_remote_tip)
            .add_observer(on_resync_refs)
            .add_observer(on_reset_head)
            .add_observer(on_rename_head)
            .add_observer(on_paste)
            .add_observer(on_undo)
            .add_observer(on_redo)
            .add_observer(on_export_head)
            .add_observer(on_export_all_named)
            .add_observer(on_export_style)
            .add_observer(on_import_style)
            .add_observer(on_import_palette)
            .add_observer(on_export_palette)
            .add_observer(on_import_file)
            .add_observer(on_reset_base_graph)
            .add_observer(on_new_graph)
            .add_observer(on_remove_name)
            .add_observer(on_import)
            .add_observer(on_set_demo)
            .add_observer(on_set_description)
            .add_observer(on_set_base_source)
            .add_observer(on_reset_all_demos)
            .add_observer(on_compile_config)
            .add_observer(on_validate_change_tracking)
            // Systems. The drivers in `EntrypointSet` evaluate head VMs. The
            // set runs after `VmSet` and `NodeSet`, so the drivers never
            // observe the gap between a head pointing at a new graph and
            // `vm::sync` reinitializing its VM, or the mirror updating its
            // node entities.
            .add_systems(
                Update,
                (
                    // Recompiles whenever a head's compile inputs change.
                    vm::sync.in_set(bevy_gantz::VmSet),
                    // Mirrors the node entities of each head's graph.
                    entities::mirror.in_set(bevy_gantz::NodeSet),
                    // Runs before the other drivers so a loaded graph
                    // evaluates its `load!` nodes before its first `update!`
                    // and `tick!` passes.
                    node::load_bang::drive_load_bangs
                        .before(node::update_bang::drive_update_bangs)
                        .before(node::tick_bang::drive_tick_bangs)
                        .before(node::await_::drive_awaits)
                        .in_set(bevy_gantz::EntrypointSet),
                    node::update_bang::drive_update_bangs.in_set(bevy_gantz::EntrypointSet),
                    node::tick_bang::drive_tick_bangs.in_set(bevy_gantz::EntrypointSet),
                    node::await_::drive_awaits.in_set(bevy_gantz::EntrypointSet),
                    // Re-pulls gui markers of dirty or recompiled heads. Runs
                    // after the entrypoint drivers so their pushes refresh
                    // markers the same frame.
                    node::gui_refresh::refresh_gui_markers
                        .after(node::load_bang::drive_load_bangs)
                        .after(node::update_bang::drive_update_bangs)
                        .after(node::tick_bang::drive_tick_bangs)
                        .after(node::await_::drive_awaits)
                        .in_set(bevy_gantz::EntrypointSet),
                    persist_camera_and_seed.in_set(ViewPersistSet),
                    // On layout settle, fork a layout-only commit. Runs after
                    // the camera and seed pass so the head's baseline exists.
                    settle_layout
                        .in_set(ViewPersistSet)
                        .after(persist_camera_and_seed)
                        .run_if(on_message::<bevy_gantz::debounced_input::DebouncedInputEvent>),
                    poll_import_task,
                    poll_style_import_task,
                    poll_palette_import_task,
                ),
            )
            .add_systems(First, clear_ui_providers)
            .add_systems(EguiPrimaryContextPass, update);
    }
}

/// Empty the domain-provided GUI collections so domain providers refill them
/// with fresh snapshots each frame. See [`SettingsTabs`] for the schedule
/// contract.
fn clear_ui_providers(
    mut tabs: ResMut<SettingsTabs>,
    mut ext_panes: ResMut<ExtPanes>,
    mut ref_ext_uis: ResMut<RefExtUis>,
    mut edge_styles: ResMut<EdgeStyles>,
    mut audio_heads: ResMut<AudioHeads>,
    mut status_items: ResMut<StatusItems>,
) {
    tabs.0.clear();
    ext_panes.0.clear();
    ref_ext_uis.0.clear();
    edge_styles.0.clear();
    audio_heads.0.clear();
    status_items.0.clear();
}

/// The live view of a head's graph, its camera and node layout.
///
/// Requires the rest of the per-head GUI state so every spawn path gets all
/// of it. App-side session restore bypasses the [`on_head_opened`] observer.
/// The `OpenHeadViews` query silently skips entities missing any of them.
#[derive(Component, Default, Clone)]
#[require(HeadNodeInstances, PerfVm)]
pub struct GraphView(pub gantz_edit::SceneView);

/// Per-head cache of reified node instances for the working graph.
///
/// Reset wholesale on head navigation in `on_head_changed`. Not required for
/// correctness, since each entry's witness check self-heals, but it bounds
/// memory when the graph is replaced outright.
#[derive(Component, Default)]
pub struct HeadNodeInstances(pub gantz_egui::node::NodeInstances);

/// Marker. This open head participates in a live collaborative session, or
/// syncs with a vault.
///
/// The session layer inserts it on join and removes it on leave. The vault
/// layer keeps it on every open head that syncs with the vault. While
/// present, the undo and redo handlers mint forward revert commits, see
/// [`gantz_egui::ops::session_undo`], instead of navigating backwards. Peers
/// plan an ancestor tip as up-to-date and drop it, so plain navigation undo
/// never syncs.
#[derive(Component)]
pub struct SessionHead;

/// Captures tracing logs for the TraceView widget.
#[derive(Default, Resource)]
pub struct TraceCapture(pub gantz_egui::widget::trace_view::TraceCapture);

/// A head's performance capture for VM execution timing. The VM Perf pane
/// shows the focused head's capture.
#[derive(Component, Default)]
pub struct PerfVm(pub gantz_egui::widget::PerfCapture);

/// Performance capture for GUI frame timing.
#[derive(Default, Resource)]
pub struct PerfGui(pub gantz_egui::widget::PerfCapture);

/// The gantz GUI state, such as open head states.
#[derive(Resource, Default)]
pub struct GuiState(pub gantz_egui::widget::GantzState);

/// The application's value-level [`NodeCodec`][gantz_egui::node::NodeCodec].
/// The seam through which `.gantz` parse and export paths validate and
/// normalize stored nodes.
///
/// The application inserts this alongside [`BuiltinNodes`], typically as
/// `NodeCodecRes(node::codec())`. [`GantzEguiPlugin`]'s import, export and
/// base systems read it.
#[derive(Clone, Copy, Resource)]
pub struct NodeCodecRes(pub gantz_egui::node::NodeCodec);

/// The collaborative-session display state threaded into the Gantz widget.
///
/// Absent unless a collab layer such as `bevy_gantz_collab` inserts and fills
/// it. When present, the Graph Config pane renders its collab row. The
/// Settings > Collab subtab arrives separately, via [`SettingsTabs`].
#[derive(Resource, Default)]
pub struct CollabUi(pub gantz_egui::collab::CollabUiState);

/// Names of base nodes baked into the binary.
///
/// When present, these names are displayed with a `[base]` prefix and
/// cannot be deleted from the Graphs pane.
#[derive(Resource, Default)]
pub struct BaseNames(pub gantz_egui::reg::Names);

/// Whether base node graphs are immutable in the GUI.
///
/// Inserted by [`GantzEguiPlugin`] based on its `base_immutable` setting.
#[derive(Resource)]
pub struct BaseImmutable(pub bool);

/// The panes the widget currently has popped out into windows, mirrored from
/// [`GantzResponse::windowed_panes`][gantz_egui::widget::gantz::GantzResponse]
/// each frame by [`update`]. A native host reads this to create and destroy
/// its OS windows. Present on all targets, but only read natively.
#[derive(Resource, Default)]
pub struct WindowedPanesRequested(pub Vec<gantz_egui::widget::WindowedPane>);

/// Marker a native host inserts to signal that it owns the pop-out windows, so
/// [`update`] builds the widget in
/// [`PaneWindowMode::HostNative`][gantz_egui::widget::PaneWindowMode] and stops
/// drawing `egui::Window`s itself.
#[derive(Resource)]
pub struct HostNativePaneWindows;

/// Settings subtabs contributed by domains. See
/// [`SettingsTab`][gantz_egui::widget::SettingsTab].
///
/// The provider contract: cleared each frame in `First`. Domain provider
/// systems refill it in `PreUpdate`, so the tabs exist before both GUI
/// render paths. Consumed by [`update`] and the native pop-out window render
/// pass `pane_window::render_windowed_panes`. A provider typically pushes a
/// fresh snapshot of its domain's config and status, and applies the change
/// payloads its tab emitted on the previous frame.
#[derive(Default, Resource)]
pub struct SettingsTabs(pub Vec<Box<dyn gantz_egui::widget::SettingsTab + Send + Sync>>);

/// Top-level panes contributed by domains. See
/// [`ExtPane`][gantz_egui::widget::ExtPane].
///
/// Same contract as [`SettingsTabs`]. A provider typically pushes a fresh
/// snapshot of its domain's per-head data.
#[derive(Default, Resource)]
pub struct ExtPanes(pub Vec<Box<dyn gantz_egui::widget::ExtPane + Send + Sync>>);

/// `NamedRef` inspector extensions contributed by domains. See
/// [`RefExtUi`][gantz_egui::node::RefExtUi].
///
/// Same contract as [`SettingsTabs`]. Extensions take `&self`, so consumers
/// borrow the resource shared.
#[derive(Default, Resource)]
pub struct RefExtUis(pub Vec<Box<dyn gantz_egui::node::RefExtUi + Send + Sync>>);

/// Graph-scene edge stylers contributed by domains. See
/// [`EdgeStyle`][gantz_egui::widget::EdgeStyle].
///
/// Same contract as [`SettingsTabs`]. Stylers take `&self`, so consumers
/// borrow the resource shared. A provider typically pushes a styler holding
/// precomputed per-head classifications, since the erased GUI registry hides
/// concrete node types.
#[derive(Default, Resource)]
pub struct EdgeStyles(pub Vec<Box<dyn gantz_egui::widget::EdgeStyle + Send + Sync>>);

/// The open heads whose graph produces audio, so their tabs show the mute
/// toggle. Same contract as [`SettingsTabs`].
#[derive(Default, Resource)]
pub struct AudioHeads(pub HashSet<ca::Head>);

/// The primary window, drawn without the system's decorations when the GUI
/// asks for its own. A host that can draw its window so inserts it and keeps
/// it up to date. The GUI then decorates the window and emits
/// [`gantz_egui::widget::WindowAction`] payloads as [`ForApp`] events for the
/// host to apply.
#[derive(Default, Resource)]
pub struct DecoratedWindow(pub gantz_egui::widget::NativeWindow);

/// The app's version, such as its release and commit. The GUI shows it on
/// hovering the wordmark. See [`gantz_egui::widget::Gantz::version`].
#[derive(Resource)]
pub struct AppVersion(pub String);

/// The host's items for the status line, such as the audio device's rate
/// and load. Same contract as [`SettingsTabs`].
#[derive(Default, Resource)]
pub struct StatusItems(pub Vec<gantz_egui::widget::StatusItem>);

impl Deref for GuiState {
    type Target = gantz_egui::widget::GantzState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GuiState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for GraphView {
    type Target = gantz_edit::SceneView;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GraphView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
