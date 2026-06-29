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
const BYTES: &[u8] = gantz_base::BYTES;

/// Fixed timestamp used to stamp the base's hand-authored (uncommitted) graphs.
///
/// The base is parsed at startup *and* again on demo reset; both must agree on
/// the synthesized commit addresses, otherwise a reset demo's `ref`s point at
/// commits that are absent from the already-loaded registry (its primitives
/// were stamped at startup). A constant makes those addresses reproducible.
pub const BASE_TIMESTAMP: gantz_ca::Timestamp = std::time::Duration::ZERO;

/// Startup system that deserializes the embedded base export and merges it
/// into the registry, populating [`BaseNames`].
pub fn load<N>(
    mut registry: ResMut<Registry<N>>,
    mut base_names: ResMut<BaseNames>,
    mut views: ResMut<crate::Views>,
    mut demos: ResMut<crate::Demos>,
) where
    N: 'static
        + serde::Serialize
        + serde::de::DeserializeOwned
        + gantz_ca::CaHash
        + gantz_format::NodeSugar
        + Send
        + Sync,
{
    let export: gantz_egui::export::Export<Graph<N>> =
        match gantz_egui::export::parse_export_at(BYTES, BASE_TIMESTAMP) {
            Ok(e) => e,
            Err(e) => {
                log::error!("base.gantz: {e}");
                return;
            }
        };
    base_names.0 = export.registry.names().clone();
    registry.merge(export.registry);
    views.0.extend(export.views);
    demos.0.extend(export.demos);
}

/// Path to write the base `.gantz` export to.
///
/// Used by [`export_to_file`] to know where to write. Typically set to
/// point at the `gantz_base` crate's `base.gantz` file so that edits
/// land back in the repo.
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
    demos: Res<crate::Demos>,
) where
    N: 'static
        + serde::Serialize
        + serde::de::DeserializeOwned
        + gantz_core::Node
        + Clone
        + gantz_format::NodeSugar
        + Send
        + Sync,
{
    let Some(text) = export_all_named(&registry, &builtins, &views, &demos) else {
        log::error!("export_to_file: failed to serialize");
        return;
    };
    if let Err(e) = std::fs::write(path.0, text) {
        log::error!("export_to_file: failed to write {}: {e}", path.0);
    }
}

/// Serialize all named graphs to `.gantz` text in the inline-name format.
///
/// This is the base writer for the `update-base` developer workflow, so it uses
/// [`gantz_egui::export::export_heads_sexpr_named`]: graphs named inline, no
/// commits/names tables, references by name - keeping `base.gantz` hand-editable
/// and free of churning addresses. (Other export paths keep the default
/// address-based format.) Returns `None` on serialization failure.
pub fn export_all_named<N>(
    registry: &Registry<N>,
    builtins: &bevy_gantz::BuiltinNodes<N>,
    views: &crate::Views,
    demos: &crate::Demos,
) -> Option<String>
where
    N: 'static
        + serde::Serialize
        + serde::de::DeserializeOwned
        + gantz_core::Node
        + Clone
        + gantz_format::NodeSugar
        + Send
        + Sync,
{
    let node_reg = crate::registry_ref(registry, builtins, demos);
    let get_node = |ca: &gantz_ca::ContentAddr| node_reg.node(ca);

    let named_heads: Vec<gantz_ca::Head> = registry
        .names()
        .keys()
        .map(|name| gantz_ca::Head::Branch(name.clone()))
        .collect();

    gantz_egui::export::export_heads_sexpr_named(
        &get_node,
        registry,
        views,
        &demos.0,
        named_heads.iter(),
    )
    .ok()
}
