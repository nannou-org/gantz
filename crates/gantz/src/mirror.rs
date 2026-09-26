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
    heads: BTreeMap<ca::Name, ca::CommitAddr>,
    /// Non-sync named references in the file's graphs as last read or
    /// written, by target name. They overlay the seed so a round trip keeps
    /// the pin rather than moving it to the target's current tip.
    pins: BTreeMap<String, ca::GraphAddr>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headless;
    use bevy_gantz_egui::base::BASE_TIMESTAMP;
    use std::time::Duration;

    fn name(s: &str) -> ca::Name {
        s.parse().expect("infallible")
    }

    /// A registry holding the embedded base sources, as a peer starts with.
    fn base_registry() -> ca::Registry {
        headless::load_sources(
            &headless::base_sources(),
            BASE_TIMESTAMP,
            &crate::node::codec(),
        )
        .registry
    }

    fn apply(registry: &mut ca::Registry, file: &mut FileState, text: &str, secs: u64) -> Applied {
        apply_text(
            registry,
            file,
            text.as_bytes(),
            Duration::from_secs(secs),
            &crate::node::codec(),
        )
        .unwrap_or_else(|e| panic!("apply failed: {e}"))
    }

    const G1: &str = "\
(graph g
  (b bang)
  (e (expr (begin $push 1)))
  (-> b e))

(layout g
  (b 0 0)
  (e 100 50)
  (camera 0 0 1))";

    const G2: &str = "\
(graph g
  (b bang)
  (e (expr (begin $push 2)))
  (-> b e))

(layout g
  (b 0 0)
  (e 100 50)
  (camera 0 0 1))";

    #[test]
    fn write_set_groups_nested_under_root() {
        let mut registry = base_registry();
        let mut file = FileState::default();
        apply(
            &mut registry,
            &mut file,
            "(graph a (b bang))\n(graph a:b (c bang))\n(graph c (d bang))",
            1,
        );
        let scope: BTreeSet<ca::Name> = [name("a"), name("a:b"), name("c"), name("missing")]
            .into_iter()
            .collect();
        let set = write_set(&registry, &scope);
        assert_eq!(
            set,
            [
                (name("a"), vec![name("a"), name("a:b")]),
                (name("c"), vec![name("c")]),
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(filename(&name("a")), Some("a.gantz".to_string()));
        assert_eq!(filename(&name("../x")), None);
        assert_eq!(filename(&name(".hidden")), None);
    }

    #[test]
    fn nested_names_round_trip_through_the_named_format() {
        let mut registry = base_registry();
        let mut file = FileState::default();
        apply(
            &mut registry,
            &mut file,
            "(graph a (b bang))\n(graph a:b (c bang))",
            1,
        );
        let text = render(&registry, &[name("a"), name("a:b")], &crate::node::codec()).unwrap();
        assert!(text.contains("(graph a:b"), "{text}");
        let applied = apply(&mut registry, &mut file, &text, 2);
        assert!(applied.is_empty(), "{applied:?}");
    }

    #[test]
    fn apply_text_commits_an_edit_with_the_previous_head_as_parent() {
        let mut registry = base_registry();
        let mut file = FileState::default();
        let first = apply(&mut registry, &mut file, G1, 1);
        assert_eq!(first.committed.len(), 1);
        let (n, c1) = &first.committed[0];
        assert_eq!(*n, name("g"));
        assert_eq!(registry.head(&name("g")), Some(*c1));
        assert!(
            gantz_egui::section::view(&registry, c1).is_some(),
            "layout attached"
        );

        let second = apply(&mut registry, &mut file, G2, 2);
        let (_, c2) = &second.committed[0];
        assert_eq!(registry.commits()[c2].parent, Some(*c1));
        assert!(gantz_egui::section::view(&registry, c2).is_some());
        assert_eq!(file.heads[&name("g")], *c2);
    }

    #[test]
    fn reapplying_rendered_text_is_a_noop() {
        let mut registry = base_registry();
        let mut file = FileState::default();
        apply(&mut registry, &mut file, G1, 1);
        let head = registry.head(&name("g")).unwrap();
        let text = render(&registry, &[name("g")], &crate::node::codec()).unwrap();
        let applied = apply(&mut registry, &mut file, &text, 2);
        assert!(applied.is_empty(), "{applied:?}");
        assert_eq!(registry.head(&name("g")), Some(head));
    }

    #[test]
    fn layout_only_edit_mints_a_layout_commit() {
        let mut registry = base_registry();
        let mut file = FileState::default();
        apply(&mut registry, &mut file, G1, 1);
        let c1 = registry.head(&name("g")).unwrap();
        let moved = G1.replace("(e 100 50)", "(e 200 50)");
        let applied = apply(&mut registry, &mut file, &moved, 2);
        assert!(applied.committed.is_empty());
        assert_eq!(applied.layout_only.len(), 1);
        let c2 = registry.head(&name("g")).unwrap();
        assert_ne!(c1, c2);
        assert_eq!(registry.commits()[&c1].graph, registry.commits()[&c2].graph);
        let view = gantz_egui::section::view(&registry, &c2).unwrap();
        assert!(view.layout.iter().any(|(_, p)| p.x == 200.0), "{view:?}");
    }

    #[test]
    fn non_sync_pins_survive_a_round_trip_and_sync_refs_follow() {
        let mut registry = base_registry();
        let add_before = gantz_egui::reg::head_graph_addr(&registry, &name("add")).unwrap();
        let mut pinned = FileState::default();
        apply(
            &mut registry,
            &mut pinned,
            "(graph pinned (a inlet) (b inlet) (r (ref add)) (-> a (r 0)) (-> b (r 1)))",
            1,
        );
        let mut following = FileState::default();
        apply(
            &mut registry,
            &mut following,
            "(graph following (a inlet) (b inlet) (r (ref add #:sync)) (-> a (r 0)) (-> b (r 1)))",
            2,
        );
        assert_eq!(pinned.pins.get("add"), Some(&add_before));
        assert!(following.pins.is_empty());

        // Move `add` on. The sync referrer follows on the resync.
        let mut add_file = FileState::default();
        let applied = apply(
            &mut registry,
            &mut add_file,
            "(graph add (a inlet) (b inlet) (out outlet) (e (expr (+ $a $b 0))) (-> a (e 0)) (-> b (e 1)) (-> e out))",
            3,
        );
        let add_after = gantz_egui::reg::head_graph_addr(&registry, &name("add")).unwrap();
        assert_ne!(add_before, add_after);
        assert!(
            applied.moved.iter().any(|m| m.name == name("following")),
            "{applied:?}"
        );
        assert!(!applied.moved.iter().any(|m| m.name == name("pinned")));

        // Re-reading the pinned file keeps its pin. Re-reading the following
        // file resolves to the new `add`.
        let text = render(&registry, &[name("pinned")], &crate::node::codec()).unwrap();
        let applied = apply(&mut registry, &mut pinned, &text, 4);
        assert!(applied.is_empty(), "{applied:?}");
        let text = render(&registry, &[name("following")], &crate::node::codec()).unwrap();
        let applied = apply(&mut registry, &mut following, &text, 5);
        assert!(applied.is_empty(), "{applied:?}");
        let graph = registry
            .head_graph(&ca::Head::Branch(name("following")))
            .unwrap();
        let refs: Vec<_> = gantz_egui::sync::named_refs(graph).collect();
        assert_eq!(refs, vec![(name("add"), add_after, true)]);
    }

    #[test]
    fn mirror_writes_reads_and_ignores_own_writes() {
        let dir = std::env::temp_dir().join(format!(
            "gantz-mirror-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut registry = base_registry();
        let mut seed_file = FileState::default();
        apply(&mut registry, &mut seed_file, G1, 1);
        let scope: BTreeSet<ca::Name> = [name("g")].into_iter().collect();
        let mut mirror = Mirror::new(dir.clone(), crate::node::codec());

        let written = mirror.write_all(&registry, &scope).unwrap();
        assert_eq!(written, vec![dir.join("g.gantz")]);
        assert!(
            mirror.write_all(&registry, &scope).unwrap().is_empty(),
            "unchanged"
        );
        // Own write, then no change: nothing to read, even after settling.
        for _ in 0..3 {
            let results = mirror
                .poll(&mut registry, Duration::from_secs(2), false)
                .unwrap();
            assert!(results.is_empty(), "{results:?}");
        }

        // An edit is read once it has held still for one poll.
        let path = dir.join("g.gantz");
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replace("$push 1", "$push 3");
        std::fs::write(&path, &edited).unwrap();
        let head = registry.head(&name("g")).unwrap();
        assert!(
            mirror
                .poll(&mut registry, Duration::from_secs(3), false)
                .unwrap()
                .is_empty()
        );
        let results = mirror
            .poll(&mut registry, Duration::from_secs(3), false)
            .unwrap();
        assert_eq!(results.len(), 1);
        let (p, applied) = &results[0];
        assert_eq!(p, &path);
        assert_eq!(applied.as_ref().unwrap().committed.len(), 1);
        assert_ne!(registry.head(&name("g")), Some(head));
        // The registry now agrees with the file, so nothing is rewritten.
        assert!(mirror.write_all(&registry, &scope).unwrap().is_empty());

        // A broken file is reported and left alone.
        std::fs::write(&path, "(graph g (b bogus))").unwrap();
        mirror
            .poll(&mut registry, Duration::from_secs(4), false)
            .unwrap();
        let results = mirror
            .poll(&mut registry, Duration::from_secs(4), false)
            .unwrap();
        assert!(matches!(&results[..], [(_, Err(_))]), "{results:?}");
        assert!(mirror.write_all(&registry, &scope).unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "(graph g (b bogus))"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
