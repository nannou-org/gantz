//! Base nodes - pre-composed graphs that ship with the binary.
//!
//! Base nodes are named graphs authored as `.gantz` files and embedded at
//! compile time via `include_bytes!`. Each domain contributes its file as a
//! [`BaseSource`] pushed into [`BaseSources`] from its plugin's `build`
//! (`GantzEguiPlugin` contributes the core `gantz_base` source). On every
//! startup, [`load`] deserializes each source in order and merges it into
//! the user's registry so base nodes are always available. Because the merge
//! replaces existing names, base nodes are authoritative - they reset to
//! their original form on each launch. Users who want to customize a base
//! node should duplicate it under a new name.
//!
//! A source may reference names another source defines: loading runs to a
//! fixpoint, parsing each source seeded with the names loaded so far (see
//! [`load`]), so e.g. a domain's base graph can compose the core source's
//! graphs. Its file writes those refs by name without embedding the foreign
//! graphs (see [`export_to_file`]).
//!
//! The set of base node names is tracked in [`BaseNames`] so the UI can
//! distinguish them (e.g. `[base]` prefix, no delete button), and each
//! name's owning source in [`BaseNameSources`] so `update-base` can write
//! every source back to its own file and demo reset can re-parse the right
//! source.

use crate::reg::{GraphCache, refresh_cache};
use bevy_ecs::prelude::*;
use bevy_gantz::Registry;
use bevy_log as log;
use gantz_ca::Name;
use std::collections::{BTreeMap, HashMap};

use crate::{BaseNames, NodeCodecRes};

/// One domain's baked-in base `.gantz` export.
pub struct BaseSource {
    /// Identifies the source (e.g. `"gantz"`, `"plyphon"`) in logs, name
    /// attribution ([`BaseNameSources`]) and `update-base` write-back routing.
    pub name: &'static str,
    /// The `.gantz` bytes (an `include_bytes!` of the domain's base file).
    pub bytes: &'static [u8],
}

/// The base sources to load, in load order.
///
/// Domain plugins contribute their source via `get_resource_or_init` + push
/// from `Plugin::build`, so plugin order does not matter for correctness
/// (name collisions across sources are resolved last-wins, loudly - see
/// [`load`]).
#[derive(Default, Resource)]
pub struct BaseSources(pub Vec<BaseSource>);

/// Which source each base name came from (name to [`BaseSource::name`]),
/// recorded by [`load`].
///
/// `update-base` uses it to write each source's names back to that source's
/// own file, and demo reset uses it to re-parse the owning source.
#[derive(Default, Resource)]
pub struct BaseNameSources(pub HashMap<String, &'static str>);

/// Paths to write each base source back to (a [`BaseSource::name`] to file
/// path map), plus the source that receives names with no recorded
/// attribution (graphs created during the session).
///
/// Used by [`export_to_file`]. The paths typically point at each source
/// crate's `base.gantz` file so that edits land back in the repo. This lives
/// in the developer tool's configuration, not on [`BaseSource`]: shipped
/// binaries must not bake dev-tree write paths.
#[derive(Resource)]
pub struct ExportPaths {
    /// Where each source's names are written ([`BaseSource::name`] to path).
    pub paths: HashMap<&'static str, &'static str>,
    /// The source that receives unattributed (session-created) names.
    pub default_source: &'static str,
}

/// Fixed timestamp used to stamp the base's hand-authored (uncommitted) graphs.
///
/// Every base source is parsed at startup *and* again on demo reset; both must
/// agree on the synthesized commit addresses, otherwise a reset demo's `ref`s
/// point at commits that are absent from the already-loaded registry (its
/// primitives were stamped at startup). A constant makes those addresses
/// reproducible.
pub const BASE_TIMESTAMP: gantz_ca::Timestamp = std::time::Duration::ZERO;

/// Startup system that deserializes each embedded base source and merges it
/// into the registry, populating [`BaseNames`] and [`BaseNameSources`].
///
/// A source may reference names another source defines, so loading runs to a
/// fixpoint: each round parses every still-pending source seeded with the
/// names loaded so far, deferring sources whose references do not resolve
/// yet. Push order therefore does not matter - sources load in dependency
/// order. A source whose references never resolve (or that fails to parse
/// outright) is logged and dropped.
pub fn load(
    sources: Res<BaseSources>,
    mut registry: ResMut<Registry>,
    mut cache: ResMut<GraphCache>,
    codec: Res<NodeCodecRes>,
    mut base_names: ResMut<BaseNames>,
    mut name_sources: ResMut<BaseNameSources>,
) {
    let mut pending: Vec<&BaseSource> = sources.0.iter().collect();
    loop {
        let mut deferred: Vec<&BaseSource> = Vec::new();
        for source in pending.iter().copied() {
            let seed = seed_graph_addrs(&base_names.0, &registry);
            let parsed: gantz_ca::Registry = match gantz_egui::export::parse_export_seeded_at(
                source.bytes,
                BASE_TIMESTAMP,
                &seed,
                &codec.0,
            ) {
                Ok(e) => e,
                // An unresolved reference may resolve once another
                // source loads - retry next round.
                Err(gantz_egui::export::ParseExportError::Format(e))
                    if matches!(e.kind, gantz_format::ErrorKind::MissingDependency(_)) =>
                {
                    deferred.push(source);
                    continue;
                }
                Err(e) => {
                    log::error!("base source `{}`: {e}", source.name);
                    continue;
                }
            };
            for (name, ca) in parsed.heads() {
                let display = name.to_string();
                if let Some(prev) = name_sources.0.get(&display) {
                    log::warn!(
                        "base source `{}` redefines `{name}` from source `{prev}` \
                         (last source wins)",
                        source.name,
                    );
                }
                name_sources.0.insert(display, source.name);
                base_names.0.insert(name.clone(), ca);
            }
            // The base's GUI metadata (demos, views) must win over the user's
            // persisted entries: the demo/view sections merge KeepExisting, so
            // reinsert the parsed entries explicitly after the merge.
            let demos: Vec<_> = gantz_egui::section::demos(&parsed).collect();
            let views: Vec<_> = gantz_egui::section::views(&parsed).collect();
            // NOTE: the merge's `heads_replaced` is deliberately not logged -
            // base names replacing a user's persisted edits on launch is the
            // by-design authoritative reset, not a collision. Source-vs-source
            // collisions are the ones worth warning about, caught above.
            registry.merge(parsed);
            for (name, demo) in demos {
                gantz_egui::section::set_demo(&mut registry.0, name, demo);
            }
            for (ca, view) in views {
                gantz_egui::section::set_view(&mut registry.0, ca, &view);
            }
        }
        // Done, or stuck: no deferred source can make progress once a full
        // round loads nothing new.
        if deferred.is_empty() {
            break;
        }
        if deferred.len() == pending.len() {
            let seed = seed_graph_addrs(&base_names.0, &registry);
            for source in deferred {
                if let Err(err) = gantz_egui::export::parse_export_seeded_at(
                    source.bytes,
                    BASE_TIMESTAMP,
                    &seed,
                    &codec.0,
                ) {
                    log::error!(
                        "base source `{}` has unresolvable references: {err}",
                        source.name,
                    );
                }
            }
            break;
        }
        pending = deferred;
    }
    // The merged base graphs must be reified before any typed reads.
    refresh_cache(&registry, &mut cache, &codec.0);
}

/// System that exports every named graph back to its owning source's file
/// (see [`ExportPaths`] and [`BaseNameSources`]).
///
/// Intended for the `update-base` developer binary. Pair with
/// `DebouncedInputEvent` so it runs on save.
pub fn export_to_file(
    paths: Res<ExportPaths>,
    name_sources: Res<BaseNameSources>,
    registry: Res<Registry>,
    codec: Res<NodeCodecRes>,
) {
    let names: Vec<Name> = registry.heads().map(|(name, _)| name.clone()).collect();
    let partitioned = partition_names(&names, &name_sources, paths.default_source);
    for (source, names) in &partitioned {
        let Some(path) = paths.paths.get(source) else {
            log::warn!(
                "export_to_file: no path configured for base source `{source}` \
                 ({} names skipped)",
                names.len(),
            );
            continue;
        };
        // Exactly this source's names, with refs into other sources written
        // by name only (no transitive closure) - loading resolves them via
        // the seeded parse.
        let names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
        match gantz_egui::export::export_names_sexpr_named(&registry, &names, &codec.0) {
            Ok(text) => {
                if let Err(e) = std::fs::write(path, text) {
                    log::error!("export_to_file: failed to write {path}: {e}");
                }
            }
            Err(e) => log::error!("export_to_file: failed to serialize `{source}`: {e}"),
        }
    }
}

/// The name -> head graph address seed for a seeded base parse: each known
/// base name resolved to its head commit's graph in the given registry (see
/// [`gantz_egui::export::parse_export_seeded_at`]).
pub fn seed_graph_addrs(
    names: &gantz_egui::reg::Names,
    registry: &gantz_ca::Registry,
) -> BTreeMap<String, gantz_ca::GraphAddr> {
    names
        .iter()
        .filter_map(|(name, ca)| {
            let commit = registry.commits().get(ca)?;
            Some((name.to_string(), commit.graph))
        })
        .collect()
}

/// Partition base names by their owning source for per-source write-back.
///
/// A nested name (`parent:child`) with no recorded source follows its
/// parent's attribution, recursing to the outermost prefix - a nested graph
/// belongs in the same file as the graph that nests it. Other unrecorded
/// names (e.g. graphs created during an `update-base` session) are attributed
/// to `default_source`.
pub fn partition_names<'a>(
    names: impl IntoIterator<Item = &'a Name>,
    name_sources: &BaseNameSources,
    default_source: &'static str,
) -> std::collections::BTreeMap<&'static str, Vec<Name>> {
    fn source_of(
        name: &Name,
        name_sources: &BaseNameSources,
        default_source: &'static str,
    ) -> &'static str {
        if let Some(&source) = name_sources.0.get(&name.to_string()) {
            return source;
        }
        match name.parent() {
            Some(parent) => source_of(&parent, name_sources, default_source),
            None => default_source,
        }
    }
    let mut partitioned = std::collections::BTreeMap::<&'static str, Vec<Name>>::new();
    for name in names {
        let source = source_of(name, name_sources, default_source);
        partitioned.entry(source).or_default().push(name.clone());
    }
    partitioned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str) -> Name {
        s.parse().unwrap()
    }

    /// Attributed names route to their source, unattributed names to the
    /// default source.
    #[test]
    fn partition_names_routes_by_attribution() {
        let mut name_sources = BaseNameSources::default();
        name_sources.0.insert("add".to_string(), "gantz");
        name_sources.0.insert("demo-sine".to_string(), "plyphon");
        let names = [name("add"), name("demo-sine"), name("new")];
        let partitioned = partition_names(names.iter(), &name_sources, "gantz");
        assert_eq!(
            partitioned.get("gantz"),
            Some(&vec![name("add"), name("new")]),
        );
        assert_eq!(partitioned.get("plyphon"), Some(&vec![name("demo-sine")]));
    }

    /// An unrecorded nested name follows its parent's attribution, however
    /// deep, before falling back to the default.
    #[test]
    fn partition_names_nested_follow_their_parent() {
        let mut name_sources = BaseNameSources::default();
        name_sources.0.insert("demo-sine".to_string(), "plyphon");
        let names = [
            name("demo-sine:child"),
            name("demo-sine:child:grandchild"),
            name("orphan:child"),
        ];
        let partitioned = partition_names(names.iter(), &name_sources, "gantz");
        assert_eq!(
            partitioned.get("plyphon"),
            Some(&vec![
                name("demo-sine:child"),
                name("demo-sine:child:grandchild"),
            ]),
        );
        assert_eq!(partitioned.get("gantz"), Some(&vec![name("orphan:child")]));
    }
}
