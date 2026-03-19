//! Base nodes - pre-composed graphs that ship with the binary.
//!
//! Base nodes are named graphs authored as `.gantz` files and embedded at
//! compile time via `include_bytes!`. On every startup, [`load`] deserializes
//! the export and merges it into the user's registry so base nodes are always
//! available. Because the merge replaces existing names, base nodes are
//! authoritative - they reset to their original form on each launch. Users
//! who want to customize a base node should duplicate it under a new name.
//!
//! The set of base node names is tracked in [`BaseNames`] so the UI can
//! distinguish them (e.g. `[base]` prefix, no delete button).

use bevy_ecs::prelude::*;
use bevy_gantz::reg::Registry;
use bevy_log as log;
use gantz_core::node::graph::Graph;

use crate::BaseNames;

/// Raw bytes of the baked-in base `.gantz` export, embedded at compile time.
const BYTES: &[u8] = include_bytes!("../../../base/base.gantz");

/// Startup system that deserializes the embedded base export and merges it
/// into the registry, populating [`BaseNames`].
pub fn load<N>(
    mut registry: ResMut<Registry<N>>,
    mut base_names: ResMut<BaseNames>,
    mut views: ResMut<crate::Views>,
) where
    N: serde::de::DeserializeOwned + Send + Sync + 'static,
{
    let text = match std::str::from_utf8(BYTES) {
        Ok(s) => s,
        Err(e) => {
            log::error!("base.gantz: invalid UTF-8: {e}");
            return;
        }
    };
    let export: gantz_egui::export::Export<Graph<N>> = match ron::from_str(text) {
        Ok(e) => e,
        Err(e) => {
            log::error!("base.gantz: failed to deserialize: {e}");
            return;
        }
    };
    base_names.0 = export.registry.names().clone();
    registry.merge(export.registry);
    views.0.extend(export.views);
}

/// Path to write the base `.gantz` export to.
///
/// Used by [`export_to_file`] to know where to write. Typically set to
/// `concat!(env!("CARGO_MANIFEST_DIR"), "/../../base/base.gantz")` so
/// that edits land back in the repo.
#[derive(Resource)]
pub struct ExportPath(pub &'static str);

/// System that exports all named graphs to the file at [`ExportPath`].
///
/// Intended for the `update-base` developer binary. Pair with
/// `DebouncedInputEvent` so it runs on save.
pub fn export_to_file<N>(
    path: Res<ExportPath>,
    registry: Res<Registry<N>>,
    builtins: Res<bevy_gantz::BuiltinNodes<N>>,
    views: Res<crate::Views>,
) where
    N: gantz_core::Node + Clone + serde::Serialize + Send + Sync + 'static,
{
    let Some(ron_str) = export_all_named_ron(&registry, &builtins, &views) else {
        log::error!("export_to_file: failed to serialize");
        return;
    };
    if let Err(e) = std::fs::write(path.0, ron_str) {
        log::error!("export_to_file: failed to write {}: {e}", path.0);
    }
}

/// Serialize all named graphs to a RON [`gantz_egui::export::Export`] string.
///
/// Useful for the `update-base` developer workflow. Returns `None` on
/// serialization failure.
pub fn export_all_named_ron<N>(
    registry: &Registry<N>,
    builtins: &bevy_gantz::BuiltinNodes<N>,
    views: &crate::Views,
) -> Option<String>
where
    N: gantz_core::Node + Clone + serde::Serialize + Send + Sync + 'static,
{
    let node_reg = crate::registry_ref(registry, builtins);
    let get_node = |ca: &gantz_ca::ContentAddr| node_reg.node(ca);

    let named_heads: Vec<gantz_ca::Head> = registry
        .names()
        .keys()
        .map(|name| gantz_ca::Head::Branch(name.clone()))
        .collect();

    let export_registry = gantz_core::reg::export_heads(&get_node, registry, named_heads.iter());
    let export = gantz_egui::export::export_with_views(export_registry, views);

    ron::ser::to_string_pretty(&export, ron::ser::PrettyConfig::default()).ok()
}
