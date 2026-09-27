//! Base sources: the `.gantz` files that domains embed with their
//! pre-composed named graphs.

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
