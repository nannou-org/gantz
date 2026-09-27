//! A working copy: a directory of `.gantz` files mirroring a set of named
//! graphs, read back as commits.
//!
//! Each top-level name gets `<dir>/<name>.gantz`, holding that graph and its
//! nested `name:child` graphs in the inline-name format. References to other
//! names are written by name. A saved edit is parsed seeded with the
//! registry's names and committed onto the name's current head, so history
//! stays linear. Referrers then follow through a reference resync.
//!
//! Nothing here touches the network. The registry is the caller's.

use gantz_ca as ca;
use gantz_egui::export::ParseExportError;
use gantz_egui::node::NodeCodec;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::SystemTime;

/// A directory mirroring named graphs, one file per top-level name.
pub struct Mirror {
    dir: PathBuf,
    codec: NodeCodec,
    files: BTreeMap<PathBuf, FileState>,
}

/// What the mirror knows about one file.
#[derive(Default)]
pub struct FileState {
    /// Bytes last written. A read that yields these is our own write.
    written: Option<Vec<u8>>,
    /// The disk stamp as of the last read or write.
    seen: Option<Stamp>,
    /// A stamp observed on the previous poll but not yet read. A read waits
    /// for the file to hold still for one poll, so a half-written save is
    /// not parsed.
    observed: Option<Stamp>,
    /// The head per name as of the last read or write. The file is rewritten
    /// only when the registry disagrees.
    pub(crate) heads: BTreeMap<ca::Name, ca::CommitAddr>,
    /// Non-sync named references in the file's graphs as last read or
    /// written, by target name. They overlay the seed so a round trip keeps
    /// the pin rather than moving it to the target's current tip.
    pub(crate) pins: BTreeMap<String, ca::GraphAddr>,
    /// The last read failed to parse. The file is left alone until it
    /// changes on disk.
    broken: bool,
    /// The last read failed on a missing dependency. It is read again once
    /// the registry changes.
    retry: bool,
}

/// What reading a file did to the registry.
#[derive(Debug, Default)]
pub struct Applied {
    /// Names whose graph changed, with their new commit.
    pub committed: Vec<(ca::Name, ca::CommitAddr)>,
    /// Names whose layout alone changed, with their new commit.
    pub layout_only: Vec<(ca::Name, ca::CommitAddr)>,
    /// Referrers that followed.
    pub moved: Vec<gantz_egui::sync::Moved>,
}

/// A file's modification time and length.
type Stamp = (SystemTime, u64);

impl Applied {
    pub fn is_empty(&self) -> bool {
        self.committed.is_empty() && self.layout_only.is_empty() && self.moved.is_empty()
    }
}

impl Mirror {
    pub fn new(dir: PathBuf, codec: NodeCodec) -> Self {
        Self {
            dir,
            codec,
            files: BTreeMap::new(),
        }
    }

    /// Write every file whose names moved in the registry. Returns the paths
    /// written.
    pub fn write_all(
        &mut self,
        registry: &ca::Registry,
        scope: &BTreeSet<ca::Name>,
    ) -> std::io::Result<Vec<PathBuf>> {
        let mut written = Vec::new();
        for (root, names) in write_set(registry, scope) {
            let Some(filename) = filename(&root) else {
                tracing::warn!("mirror: `{root}` is not a usable file name; not mirrored");
                continue;
            };
            let path = self.dir.join(filename);
            let file = self.files.entry(path.clone()).or_default();
            if file.broken {
                continue;
            }
            let heads: BTreeMap<ca::Name, ca::CommitAddr> = names
                .iter()
                .filter_map(|n| registry.head(n).map(|ca| (n.clone(), ca)))
                .collect();
            if file.heads == heads {
                continue;
            }
            let text = render(registry, &names, &self.codec).map_err(std::io::Error::other)?;
            let tmp = path.with_extension("gantz.tmp");
            std::fs::write(&tmp, &text)?;
            std::fs::rename(&tmp, &path)?;
            file.seen = Some(stamp(&std::fs::metadata(&path)?));
            file.observed = None;
            file.written = Some(text.into_bytes());
            file.pins = pins(
                names
                    .iter()
                    .filter_map(|n| registry.head_graph(&ca::Head::Branch(n.clone()))),
            );
            file.heads = heads;
            file.retry = false;
            written.push(path);
        }
        Ok(written)
    }

    /// Read every `.gantz` file in the directory that changed on disk, and
    /// commit what it defines. Own writes are recognised by content. A file
    /// that failed on a missing dependency is read again when
    /// `registry_changed`.
    pub fn poll(
        &mut self,
        registry: &mut ca::Registry,
        now: ca::Timestamp,
        registry_changed: bool,
    ) -> std::io::Result<Vec<(PathBuf, Result<Applied, ParseExportError>)>> {
        let mut results = Vec::new();
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&self.dir)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| gantz_egui::export::is_gantz_path(path))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let current = stamp(&meta);
            let file = self.files.entry(path.clone()).or_default();
            let read = if file.seen != Some(current) {
                let settled = file.observed == Some(current);
                file.observed = Some(current);
                settled
            } else {
                file.retry && registry_changed
            };
            if !read {
                continue;
            }
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!("mirror: {}: {e}", path.display());
                    continue;
                }
            };
            file.seen = Some(current);
            file.observed = None;
            if file.written.as_deref() == Some(bytes.as_slice()) {
                file.broken = false;
                continue;
            }
            let result = apply_text(registry, file, &bytes, now, &self.codec);
            match &result {
                Ok(_) => {
                    file.broken = false;
                    file.retry = false;
                }
                Err(e) => {
                    file.retry = is_missing_dependency(e);
                    file.broken = !file.retry;
                }
            }
            results.push((path, result));
        }
        Ok(results)
    }
}

/// The names to write per top-level name: the root and its nested names, in
/// name order. Names with no head are skipped.
pub fn write_set(
    registry: &ca::Registry,
    scope: &BTreeSet<ca::Name>,
) -> BTreeMap<ca::Name, Vec<ca::Name>> {
    let mut set: BTreeMap<ca::Name, Vec<ca::Name>> = BTreeMap::new();
    for name in scope {
        if registry.head(name).is_none() {
            continue;
        }
        let root = ca::Name::root(name.segments()[0].clone());
        set.entry(root).or_default().push(name.clone());
    }
    set
}

/// The file name for a top-level name, or `None` when the name cannot be a
/// file name.
pub fn filename(root: &ca::Name) -> Option<String> {
    let s = root.to_string();
    let unusable = s.is_empty()
        || s.starts_with('.')
        || s.contains(['/', '\\', '\0'])
        || s == ".."
        || s == ".";
    (!unusable).then(|| format!("{s}.{}", gantz_egui::export::FILE_EXTENSION))
}

/// The inline-name text of exactly `names`, with references to other names
/// by name.
pub fn render(
    registry: &ca::Registry,
    names: &[ca::Name],
    codec: &NodeCodec,
) -> Result<String, gantz_format::FormatError> {
    let names: Vec<String> = names.iter().map(ToString::to_string).collect();
    gantz_egui::export::export_names_sexpr_named(registry, &names, codec)
}

/// Parse `bytes` seeded with the registry's names and the file's pins, and
/// commit each name it defines onto that name's current head. Layout-only
/// changes mint a layout commit. Referrers then follow.
///
/// The parsed registry is never merged. A parsed inline-name file yields
/// parentless root commits, which would break the lineage the session
/// converges on.
pub fn apply_text(
    registry: &mut ca::Registry,
    file: &mut FileState,
    bytes: &[u8],
    now: ca::Timestamp,
    codec: &NodeCodec,
) -> Result<Applied, ParseExportError> {
    let seed = seed(registry, file);
    let parsed = gantz_egui::export::parse_export_seeded_at(bytes, now, &seed, codec)?;
    let newest = registry
        .commits()
        .values()
        .map(|c| c.timestamp)
        .max()
        .unwrap_or_default();
    let ts = ca::sync::monotonic_timestamp(now, newest);
    let mut applied = Applied::default();
    let mut heads = BTreeMap::new();
    let mut graphs = Vec::new();
    for (name, parsed_ca) in parsed.heads() {
        let Some(graph) = parsed.commit_graph_ref(&parsed_ca) else {
            continue;
        };
        let view = gantz_egui::section::view(&parsed, &parsed_ca);
        let ga = registry.add_graph(graph.clone());
        let mut head = ca::Head::Branch(name.clone());
        let new = if gantz_egui::reg::head_graph_addr(registry, name) == Some(ga) {
            let current = registry.head(name).expect("the name has a head");
            let layout = view
                .as_ref()
                .and_then(|v| gantz_egui::ops::commit_layout(registry, ts, &mut head, v));
            match (layout, &view) {
                (Some(new), Some(v)) => {
                    gantz_egui::section::set_view(registry, new, v);
                    applied.layout_only.push((name.clone(), new));
                    new
                }
                // No baseline view yet. Seed one so the next edit compares.
                (None, Some(v)) if gantz_egui::section::view(registry, &current).is_none() => {
                    gantz_egui::section::seed_view(registry, current, v);
                    current
                }
                _ => current,
            }
        } else {
            let new = registry.commit_graph_to_name(
                ts,
                ga,
                || unreachable!("the graph was added above"),
                name,
            );
            if let Some(v) = &view {
                gantz_egui::section::seed_view(registry, new, v);
            }
            applied.committed.push((name.clone(), new));
            new
        };
        heads.insert(name.clone(), new);
        graphs.push(graph.clone());
    }
    applied.moved = gantz_collab_sync::resync_headless(registry, ts);
    for m in &applied.moved {
        if let Some(head) = heads.get_mut(&m.name) {
            *head = m.new_commit;
        }
    }
    file.heads = heads;
    file.pins = pins(graphs.iter());
    Ok(applied)
}

/// The seed for a file's parse: every name's head graph, overlaid by the
/// file's pins.
fn seed(registry: &ca::Registry, file: &FileState) -> BTreeMap<String, ca::GraphAddr> {
    let mut seed: BTreeMap<String, ca::GraphAddr> = registry
        .heads()
        .filter_map(|(name, ca)| {
            let commit = registry.commits().get(&ca)?;
            Some((name.to_string(), commit.graph))
        })
        .collect();
    seed.extend(file.pins.iter().map(|(n, ga)| (n.clone(), *ga)));
    seed
}

/// The non-sync named references across `graphs`, by target name.
fn pins<'a>(graphs: impl Iterator<Item = &'a ca::DataGraph>) -> BTreeMap<String, ca::GraphAddr> {
    graphs
        .flat_map(gantz_egui::sync::named_refs)
        .filter(|(_, _, sync)| !sync)
        .map(|(name, ga, _)| (name.to_string(), ga))
        .collect()
}

fn stamp(meta: &std::fs::Metadata) -> Stamp {
    (
        meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        meta.len(),
    )
}

fn is_missing_dependency(e: &ParseExportError) -> bool {
    matches!(
        e,
        ParseExportError::Format(e)
            if matches!(e.kind, gantz_format::ErrorKind::MissingDependency(_))
    )
}
