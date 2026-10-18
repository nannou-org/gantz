//! Import and export of graphs and the style, and resets of base graphs.

use crate::{BaseNames, ForApp, GraphCache, GuiState, NodeCodecRes, base, refresh_cache};
use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_gantz::head;
use bevy_log as log;
use gantz_ca as ca;

/// In-flight import file dialog task.
#[derive(Resource)]
pub struct ImportTask(bevy_tasks::Task<Option<Vec<u8>>>);

/// In-flight style import file dialog task.
#[derive(Resource)]
pub struct StyleImportTask(bevy_tasks::Task<Option<Vec<u8>>>);

/// Event emitted when the user requests exporting a head.
#[derive(Event)]
pub struct ExportHeadEvent {
    /// The head entity to export.
    pub head: Entity,
}

/// Event emitted when the user requests exporting all named graphs.
#[derive(Event)]
pub struct ExportAllNamedEvent;

/// Event emitted when the user requests exporting the GUI style.
#[derive(Event)]
pub struct ExportStyleEvent;

/// Event emitted when the user requests importing a GUI style.
#[derive(Event)]
pub struct ImportStyleEvent;

/// Event emitted when a `.gantz` file is dropped onto a pane.
#[derive(Event)]
pub struct ImportFileEvent {
    /// The raw bytes of the dropped file.
    pub bytes: Vec<u8>,
    /// Whether to open the root head after merging. Set for a GraphScene drop.
    pub open_head: bool,
}

/// Event emitted when a base graph should be reset to its original state.
#[derive(Event)]
pub struct ResetBaseGraphEvent(pub ca::Name);

/// Handle export head events.
///
/// Exports the head's graph with transitive dependencies and views to a
/// `.gantz` file chosen via an `rfd` file dialog. The export is serialized as
/// `.gantz` text by [`gantz_egui::export`].
pub fn on_export_head(
    trigger: On<ExportHeadEvent>,
    registry: Res<Registry>,
    codec: Res<NodeCodecRes>,
    heads: Query<&head::HeadRef, With<head::OpenHead>>,
) {
    let event = trigger.event();
    let Ok(head_ref) = heads.get(event.head) else {
        log::error!("ExportHead: head not found for entity {:?}", event.head);
        return;
    };
    let head: &ca::Head = &**head_ref;

    let text = match gantz_egui::export::export_heads_sexpr(&registry, [head], &codec.0) {
        Ok(s) => s,
        Err(e) => {
            log::error!("ExportHead: failed to serialize: {e}");
            return;
        }
    };

    let default_name = gantz_egui::export::default_filename(&head);

    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Export Graph")
        .set_file_name(&default_name)
        .add_filter("Gantz Export", &[gantz_egui::export::FILE_EXTENSION]);
    spawn_save(dialog, text, "graph");
}

/// Handle export-all-named events.
///
/// Exports every named graph with transitive dependencies and views to a
/// single `.gantz` file chosen via an `rfd` file dialog.
pub fn on_export_all_named(
    _trigger: On<ExportAllNamedEvent>,
    registry: Res<Registry>,
    codec: Res<NodeCodecRes>,
) {
    let named_heads: Vec<ca::Head> = registry
        .heads()
        .map(|(name, _)| ca::Head::Branch(name.clone()))
        .collect();

    if named_heads.is_empty() {
        log::info!("ExportAllNamed: no named graphs to export");
        return;
    }

    let text = match gantz_egui::export::export_heads_sexpr(&registry, named_heads.iter(), &codec.0)
    {
        Ok(s) => s,
        Err(e) => {
            log::error!("ExportAllNamed: failed to serialize: {e}");
            return;
        }
    };

    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Export All Named Graphs")
        .set_file_name(&format!("gantz.{}", gantz_egui::export::FILE_EXTENSION))
        .add_filter("Gantz Export", &[gantz_egui::export::FILE_EXTENSION]);
    spawn_save(dialog, text, "all named graphs");
}

/// Handle style export events.
///
/// Writes the GUI's [`gantz_egui::StyleConfig`] to a `.ron` file chosen via an
/// `rfd` file dialog.
pub fn on_export_style(_trigger: On<ExportStyleEvent>, gui_state: Res<GuiState>) {
    let text = match gantz_egui::style::to_ron(&gui_state.style) {
        Ok(s) => s,
        Err(e) => {
            log::error!("ExportStyle: failed to serialize: {e}");
            return;
        }
    };

    let ext = gantz_egui::style::FILE_EXTENSION;
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Export Style")
        .set_file_name(&format!("gantz-style.{ext}"))
        .add_filter("Gantz Style", &[ext]);
    spawn_save(dialog, text, "style");
}

/// Handle style import events by opening a file dialog.
///
/// The chosen file's bytes return to the main world via [`StyleImportTask`],
/// polled by `poll_style_import_task`.
pub fn on_import_style(
    _trigger: On<ImportStyleEvent>,
    task: Option<Res<StyleImportTask>>,
    mut cmds: Commands,
) {
    // Only one dialog at a time.
    if task.is_some() {
        return;
    }
    let ext = gantz_egui::style::FILE_EXTENSION;
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Import Style")
        .add_filter("Gantz Style", &[ext]);
    cmds.insert_resource(StyleImportTask(spawn_pick(dialog)));
}

/// Handle import file events for dropped `.gantz` files.
///
/// Deserializes the export, renames old base names, optionally computes root
/// names, merges into the registry, and opens the unique root head if
/// requested.
pub fn on_import_file(
    trigger: On<ImportFileEvent>,
    sources: Res<base::BaseSources>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut cmds: Commands,
) {
    let event = trigger.event();
    let mut export = match gantz_egui::export::parse_export(&event.bytes, &codec.0) {
        Ok(e) => e,
        Err(e) => {
            log::error!("ImportFile: {e}");
            return;
        }
    };

    // A file written before a base rename names the old base graphs.
    let renames = gantz_egui::base::renames(&sources.0);
    gantz_edit::resync::rename(&mut export, bevy_gantz::reg::timestamp(), &renames);

    // The root name is needed only to open a head. Compute it before the
    // merge consumes the export.
    let root_name = if event.open_head {
        gantz_egui::export::unique_root_name(&export)
    } else {
        None
    };

    let report = registry.merge(export);
    refresh_cache(&registry, &mut cache, &codec.0);
    log::info!(
        "Imported: {} names added, {} replaced",
        report.heads_added.len(),
        report.heads_replaced.len(),
    );

    if let Some(name) = root_name {
        cmds.trigger(head::OpenEvent(ca::Head::Branch(name)));
    }
}

/// Reset a base graph to its original state by re-merging from the base export.
pub fn on_reset_base_graph(
    trigger: On<ResetBaseGraphEvent>,
    sources: Res<base::BaseSources>,
    name_sources: Res<base::BaseNameSources>,
    base_names: Res<BaseNames>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
) {
    let name = &trigger.event().0;
    // Re-parse the source that defined the name, seeded with the loaded base
    // names so the source's cross-source refs resolve. Its own names shadow
    // the seed via the in-document lookup. Every source parses at
    // BASE_TIMESTAMP, so addresses match the startup parse.
    let Some(source) = name_sources
        .0
        .get(&name.to_string())
        .and_then(|source_name| sources.0.iter().find(|s| s.name == *source_name))
    else {
        log::warn!("ResetBaseGraph: no base source recorded for '{name}'");
        return;
    };
    let renames = gantz_egui::base::renames(&sources.0);
    let seed = gantz_egui::base::seed_graph_addrs(&base_names.0, &renames, &registry);
    let export: gantz_ca::Registry = match gantz_egui::export::parse_export_seeded_at(
        source.bytes,
        gantz_egui::base::BASE_TIMESTAMP,
        &seed,
        &codec.0,
    ) {
        Ok(e) => e,
        Err(e) => {
            log::error!(
                "ResetBaseGraph: failed to parse base source `{}`: {e}",
                source.name
            );
            return;
        }
    };
    // Extract just the content reachable from the target name. All parents
    // are included, so merge ancestry survives a reset.
    if let Some(base_commit_ca) = export.head(name) {
        let live = ca::closure_from(&export, [base_commit_ca]);
        let mut subset = ca::export(&export, &live);
        subset.set_head(name.clone(), base_commit_ca);
        registry.merge(subset);
        refresh_cache(&registry, &mut cache, &codec.0);
        log::info!("Reset base graph '{name}' to original version");
    } else {
        log::warn!(
            "ResetBaseGraph: name '{name}' not found in base source `{}`",
            source.name,
        );
    }
}

/// Poll the in-flight import file dialog task.
///
/// When the task completes with file bytes, triggers [`ImportFileEvent`].
/// The resource is removed regardless of whether a file was selected.
pub(crate) fn poll_import_task(task: Option<ResMut<ImportTask>>, mut cmds: Commands) {
    let Some(mut task) = task else { return };
    if let Some(result) = bevy_tasks::futures::check_ready(&mut task.0) {
        cmds.remove_resource::<ImportTask>();
        if let Some(bytes) = result {
            cmds.trigger(ImportFileEvent {
                bytes,
                open_head: true,
            });
        }
    }
}

/// Poll the in-flight style import file dialog task.
///
/// When the task completes with file bytes, replaces the GUI's style config.
/// `gantz_egui::style::apply` picks it up on the next pass. The resource is
/// removed regardless of whether a file was selected.
pub(crate) fn poll_style_import_task(
    task: Option<ResMut<StyleImportTask>>,
    mut gui_state: ResMut<GuiState>,
    mut cmds: Commands,
) {
    let Some(mut task) = task else { return };
    let Some(result) = bevy_tasks::futures::check_ready(&mut task.0) else {
        return;
    };
    cmds.remove_resource::<StyleImportTask>();
    let Some(bytes) = result else { return };
    match std::str::from_utf8(&bytes).map(gantz_egui::style::from_ron) {
        Ok(Ok(style)) => {
            gui_state.style = style;
            log::info!("Imported style");
        }
        Ok(Err(e)) => log::error!("ImportStyle: failed to parse: {e}"),
        Err(e) => log::error!("ImportStyle: invalid UTF-8: {e}"),
    }
}

/// Open a file dialog to import a `.gantz` file, unless one is in flight.
/// [`poll_import_task`] imports the picked file.
pub(crate) fn on_import(
    _trigger: On<ForApp<gantz_egui::Import>>,
    import_task: Option<Res<ImportTask>>,
    mut cmds: Commands,
) {
    if import_task.is_some() {
        return;
    }
    let ext = gantz_egui::export::FILE_EXTENSION;
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Import")
        .add_filter("Gantz Export", &[ext]);
    cmds.insert_resource(ImportTask(spawn_pick(dialog)));
}

/// Move a named graph to another base source. This is durable wherever the
/// per-source write-back runs, as in `update-base`. That rewrites every
/// configured file on the next save, which moves the graph between files.
pub(crate) fn on_set_base_source(
    trigger: On<ForApp<gantz_egui::SetBaseSource>>,
    base_sources: Res<base::BaseSources>,
    mut base_name_sources: ResMut<base::BaseNameSources>,
) {
    let gantz_egui::SetBaseSource { name, source } = &trigger.event().0;
    match base_sources.0.iter().find(|s| s.name == source.as_str()) {
        Some(s) => {
            base_name_sources.0.insert(name.to_string(), s.name);
        }
        None => log::warn!("base source `{source}` is not registered"),
    }
}

/// Reset every `demo.` base graph by re-merging it from its base source.
pub(crate) fn on_reset_all_demos(
    _trigger: On<ForApp<gantz_egui::ResetAllDemos>>,
    base_names: Res<BaseNames>,
    mut cmds: Commands,
) {
    for name in base_names.0.keys() {
        if gantz_egui::widget::graph_select::is_demo(name) {
            cmds.trigger(ResetBaseGraphEvent(name.clone()));
        }
    }
}

/// Write `text` to the file the user picks in `dialog`, off the main thread.
/// `what` names the content in the log, such as "graph".
fn spawn_save(dialog: rfd::AsyncFileDialog, text: String, what: &'static str) {
    bevy_tasks::AsyncComputeTaskPool::get()
        .spawn(async move {
            if let Some(handle) = dialog.save_file().await {
                match handle.write(text.as_bytes()).await {
                    Ok(()) => log::info!("Exported {what} to {}", handle.file_name()),
                    Err(e) => log::error!("Failed to export {what}: {e}"),
                }
            }
        })
        .detach();
}

/// Read the file the user picks in `dialog`, off the main thread. The task
/// yields `None` when the dialog is cancelled.
fn spawn_pick(dialog: rfd::AsyncFileDialog) -> bevy_tasks::Task<Option<Vec<u8>>> {
    bevy_tasks::AsyncComputeTaskPool::get().spawn(async move {
        let handle = dialog.pick_file().await?;
        Some(handle.read().await)
    })
}
