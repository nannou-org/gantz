//! Base sources: the `.gantz` files that domains embed with their
//! pre-composed named graphs.

use crate::export::ParseExportError;
use gantz_ca::{CommitAddr, GraphAddr, Name};
use std::collections::BTreeMap;

/// One domain's baked-in base `.gantz` export.
pub struct BaseSource {
    /// The source name, for example `"gantz"`. Used in logs and diagnostics,
    /// and to attribute each base name to its source.
    pub name: &'static str,
    /// The `.gantz` bytes of the domain's base file.
    pub bytes: &'static [u8],
    /// Earlier names of the source's graphs, each as `(old, new)`. A host
    /// renames stored graphs and references with these, and a seeded parse
    /// resolves an old name to its new name's graph. See [`renames`].
    pub renames: &'static [(&'static str, &'static str)],
}

/// Named heads, ordered by name.
pub type Names = BTreeMap<Name, CommitAddr>;

/// The fixed timestamp used to stamp the base's hand-authored graphs.
///
/// Every base source is parsed at startup and again on demo reset. Both must
/// agree on the synthesized commit addresses. Otherwise a reset demo's `ref`s
/// point at commits absent from the loaded registry. A constant makes those
/// addresses reproducible.
pub const BASE_TIMESTAMP: gantz_ca::Timestamp = std::time::Duration::ZERO;

/// The [`BaseSource::renames`] of every source, in source order.
pub fn renames<'a>(sources: impl IntoIterator<Item = &'a BaseSource>) -> Vec<(Name, Name)> {
    sources
        .into_iter()
        .flat_map(|source| source.renames)
        .map(|(old, new)| (name(old), name(new)))
        .collect()
}

/// The name to head graph address seed for a seeded base parse. Each known
/// base name resolves to its head commit's graph in the given registry, and
/// so does each of its old names. See [`with_old_names`] and
/// [`crate::export::parse_export_seeded_at`].
pub fn seed_graph_addrs(
    names: &Names,
    renames: &[(Name, Name)],
    registry: &gantz_ca::Registry,
) -> BTreeMap<String, GraphAddr> {
    let seed = names
        .iter()
        .filter_map(|(name, ca)| {
            let commit = registry.commits().get(ca)?;
            Some((name.to_string(), commit.graph))
        })
        .collect();
    with_old_names(seed, renames)
}

/// Add the old name of each seeded name that `renames` renamed, nested names
/// included. An old name resolves to its new name's graph, so a file written
/// before a rename still parses. A name the seed already has keeps its graph.
pub fn with_old_names(
    mut seed: BTreeMap<String, GraphAddr>,
    renames: &[(Name, Name)],
) -> BTreeMap<String, GraphAddr> {
    let old_names: Vec<(String, GraphAddr)> = seed
        .iter()
        .filter_map(|(seeded, ga)| {
            let seeded = name(seeded);
            let old = renames
                .iter()
                .find_map(|(old, new)| seeded.replace_prefix(new, old))?;
            Some((old.to_string(), *ga))
        })
        .collect();
    for (old, ga) in old_names {
        seed.entry(old).or_insert(ga);
    }
    seed
}

fn name(s: &str) -> Name {
    let Ok(name) = s.parse();
    name
}

/// Load `n` sources to a fixpoint. Returns the sources that failed, by index.
///
/// `load(ix)` parses source `ix`, seeded with the names loaded so far, and
/// keeps the result. A source may reference names another source defines, so
/// a source whose references do not resolve yet is retried in the next round.
/// Once a round loads nothing, the sources still waiting keep their missing
/// dependency error. That round's seed did not change, so the error is final.
pub fn load_to_fixpoint(
    n: usize,
    mut load: impl FnMut(usize) -> Result<(), ParseExportError>,
) -> Vec<(usize, ParseExportError)> {
    let mut failed = vec![];
    let mut pending: Vec<usize> = (0..n).collect();
    loop {
        let mut waiting = vec![];
        for &ix in &pending {
            match load(ix) {
                Ok(()) => (),
                Err(e) if e.is_missing_dependency() => waiting.push((ix, e)),
                Err(e) => failed.push((ix, e)),
            }
        }
        if waiting.len() == pending.len() || waiting.is_empty() {
            failed.extend(waiting);
            break;
        }
        pending = waiting.into_iter().map(|(ix, _)| ix).collect();
    }
    failed.sort_by_key(|(ix, _)| *ix);
    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source that references another source's graph loads once that source
    /// has, whatever their order. A source whose reference never resolves
    /// fails with its missing dependency.
    #[test]
    fn sources_load_to_a_fixpoint() {
        let codec = crate::test_node::codec();
        let sources = [
            "(graph user (r (ref lib)))",
            "(graph lib (x (expr 1)))",
            "(graph lost (r (ref nowhere)))",
        ];
        let mut names = Names::new();
        let mut registry = gantz_ca::Registry::default();
        let mut loaded = vec![];
        let failed = load_to_fixpoint(sources.len(), |ix| {
            let seed = seed_graph_addrs(&names, &[], &registry);
            let bytes = sources[ix].as_bytes();
            let parsed =
                crate::export::parse_export_seeded_at(bytes, BASE_TIMESTAMP, &seed, &codec)?;
            names.extend(parsed.heads().map(|(n, ca)| (n.clone(), ca)));
            registry.merge(parsed);
            loaded.push(ix);
            Ok(())
        });
        assert_eq!(loaded, [1, 0]);
        let failed: Vec<_> = failed
            .iter()
            .map(|(ix, e)| (*ix, e.is_missing_dependency()))
            .collect();
        assert_eq!(failed, [(2, true)]);
    }

    /// Old names resolve to their new names' graphs, nested names included.
    /// A name the seed already has keeps its graph.
    #[test]
    fn old_names_seed_their_new_names_graphs() {
        let ga = |b: u8| GraphAddr::from(gantz_ca::ContentAddr::from([b; 32]));
        let seed = BTreeMap::from([
            ("ui.row".to_string(), ga(1)),
            ("ui.row:1".to_string(), ga(2)),
            ("ui.col".to_string(), ga(3)),
            ("uicol".to_string(), ga(4)),
        ]);
        let renames = [
            (name("uirow"), name("ui.row")),
            (name("uicol"), name("ui.col")),
        ];
        let seed = with_old_names(seed, &renames);
        let get = |n: &str| seed.get(n).copied();
        assert_eq!(get("uirow"), Some(ga(1)));
        assert_eq!(get("uirow:1"), Some(ga(2)));
        assert_eq!(get("uicol"), Some(ga(4)));
        assert_eq!(seed.len(), 6);
    }
}
