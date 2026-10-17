//! Base sources: the `.gantz` files that domains embed with their
//! pre-composed named graphs.

use crate::export::ParseExportError;
use crate::reg::Names;
use std::collections::BTreeMap;

/// One domain's baked-in base `.gantz` export.
pub struct BaseSource {
    /// The source name, for example `"gantz"`. Used in logs and diagnostics,
    /// and to attribute each base name to its source.
    pub name: &'static str,
    /// The `.gantz` bytes of the domain's base file.
    pub bytes: &'static [u8],
}

/// The fixed timestamp used to stamp the base's hand-authored graphs.
///
/// Every base source is parsed at startup and again on demo reset. Both must
/// agree on the synthesized commit addresses. Otherwise a reset demo's `ref`s
/// point at commits absent from the loaded registry. A constant makes those
/// addresses reproducible.
pub const BASE_TIMESTAMP: gantz_ca::Timestamp = std::time::Duration::ZERO;

/// The name to head graph address seed for a seeded base parse. Each known
/// base name resolves to its head commit's graph in the given registry. See
/// [`crate::export::parse_export_seeded_at`].
pub fn seed_graph_addrs(
    names: &Names,
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
            let seed = seed_graph_addrs(&names, &registry);
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
}
