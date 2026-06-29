//! A human-readable text format for gantz graph registries.
//!
//! `gantz_format` is the layout-agnostic core of the `.gantz` format: it reads
//! and writes a [`gantz_ca::Registry`] of graphs as S-expression text that is
//! reader-valid Steel (so embedded node code needs no escaping and tooling can
//! reuse Steel's reader), without requiring the author to know any content
//! addresses.
//!
//! It recognises only the registry forms - `(graph ...)`, `(commits ...)` and
//! `(names ...)`. Unrecognised top-level forms are preserved (see [`Form`]),
//! not errored, so richer layers can extend the format - e.g. a GUI adding
//! `(layout ...)` - using the [`sexpr`] toolkit together with the resolution
//! context returned by [`from_str`]/[`to_string`].
//!
//! Node keywords (`expr`, `inlet`, ...) are pluggable [`Sugar`]: [`from_str`]
//! and [`to_string`] read the node set's composite via [`NodeSugar`]
//! (`N::sugar()`), so each crate owns the sugar for its own nodes ([`CoreSugar`]
//! covers `gantz_core`'s). The `_with` variants accept any `&dyn Sugar`
//! explicitly (compose with [`Sugars`]), still falling back to the generic
//! `(node ...)` form. Any node type that is `Serialize + DeserializeOwned +
//! CaHash` works.

mod datum;
mod error;
mod lower;
mod model;
mod parse;
mod raise;
mod sugar;
mod writer;

pub mod sexpr;

pub use datum::{Datum, DatumError, datum_from_expr, datum_text, from_datum, node_datum, to_datum};
pub use error::{ErrorKind, FormatError, Span};
pub use lower::Loaded;
pub use model::{Addr, Document, Form};
pub use raise::{Dumped, GraphLabels};
pub use sugar::{CoreSugar, NodeSugar, Sugar, SugarArgs, Sugars};

use gantz_ca::{CaHash, Registry, Timestamp};
use gantz_core::node::graph::Graph;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Parse a `.gantz` document (using the node set's composite [`NodeSugar`]) into
/// its [`Loaded`] registry, resolution context and preserved extra forms.
///
/// `now` provides the timestamp for any graph the `(commits ...)` table does not
/// describe (hand-authored graphs with no commit entry).
pub fn from_str<N>(text: &str, now: Timestamp) -> Result<Loaded<N>, FormatError>
where
    N: Serialize + DeserializeOwned + CaHash + NodeSugar + 'static,
{
    from_str_with(text, now, &N::sugar())
}

/// Parse a `.gantz` document using a custom keyword [`Sugar`] (compose with
/// [`CoreSugar`] via [`Sugars`] to keep `gantz_core`'s built-ins).
pub fn from_str_with<N>(
    text: &str,
    now: Timestamp,
    sugar: &dyn Sugar,
) -> Result<Loaded<N>, FormatError>
where
    N: Serialize + DeserializeOwned + CaHash + 'static,
{
    let doc = parse::parse(text, sugar)?;
    lower::lower(doc, now)
}

/// Serialize a registry to `.gantz` text (with gantz's built-in node keywords),
/// returning the text along with the per-graph label context an extender needs
/// to emit its own forms.
pub fn to_string<N>(registry: &Registry<Graph<N>>) -> Result<Dumped, FormatError>
where
    N: Serialize + DeserializeOwned + NodeSugar,
{
    to_string_with(registry, &N::sugar())
}

/// Serialize a registry to `.gantz` text using a custom keyword [`Sugar`].
pub fn to_string_with<N>(
    registry: &Registry<Graph<N>>,
    sugar: &dyn Sugar,
) -> Result<Dumped, FormatError>
where
    N: Serialize + DeserializeOwned,
{
    raise::raise(registry, sugar)
}

/// Serialize a registry in the inline-name format: each named graph is emitted
/// under its registry name, with no `(commits ...)` / `(names ...)` tables and
/// references resolved by name. Intended for hand-editable, churn-free files
/// such as the baked-in base.
pub fn to_string_named<N>(registry: &Registry<Graph<N>>) -> Result<Dumped, FormatError>
where
    N: Serialize + DeserializeOwned + NodeSugar,
{
    to_string_named_with(registry, &N::sugar())
}

/// As [`to_string_named`], with a custom keyword [`Sugar`].
pub fn to_string_named_with<N>(
    registry: &Registry<Graph<N>>,
    sugar: &dyn Sugar,
) -> Result<Dumped, FormatError>
where
    N: Serialize + DeserializeOwned,
{
    raise::raise_named(registry, sugar)
}

#[cfg(test)]
mod tests {
    //! `NodeSugar` and `Sugar` are both entirely optional for a downstream
    //! node-set type. The `_with` entry points carry no `NodeSugar` bound, and a
    //! node whose tag no sugar recognises simply round-trips through the generic
    //! `(node "Tag" ...)` form. This guards that property.

    use super::*;
    use gantz_ca::{CaHash, Hasher};
    use serde::{Deserialize, Serialize};

    // A self-contained node-set with one node type that implements neither
    // `NodeSugar` nor any `Sugar` - it carries no first-class keyword at all.
    #[typetag::serde(tag = "type")]
    trait Widget: CaHash {}

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Knob {
        value: i64,
    }

    impl CaHash for Knob {
        fn hash(&self, hasher: &mut Hasher) {
            self.value.hash(hasher);
        }
    }

    #[typetag::serde]
    impl Widget for Knob {}

    // `Box<dyn Widget>` is the node-set type `N`: typetag supplies its
    // Serialize/Deserialize, and `gantz_ca`'s blanket `CaHash for Box<T>` covers
    // the rest. It implements no `NodeSugar`.

    #[test]
    fn the_with_variants_need_no_node_sugar() {
        // A graph with one node written in the generic form. `Knob` is unknown to
        // every sugar, so it must round-trip via `(node "Knob" ...)`.
        let text = "(graph g (k (node \"Knob\" (value 7))))";

        // Both `_with` calls compile and run even though `Box<dyn Widget>`
        // implements neither `NodeSugar` nor `Sugar`. (The convenience
        // `from_str`/`to_string` would instead require a `NodeSugar` impl.)
        let loaded = from_str_with::<Box<dyn Widget>>(text, std::time::Duration::ZERO, &CoreSugar)
            .expect("parse without NodeSugar");
        let dumped = to_string_with(&loaded.registry, &CoreSugar).expect("write without NodeSugar");

        // The node survived the round-trip through the generic form.
        assert!(
            dumped.text.contains("(node \"Knob\""),
            "expected generic node form, got:\n{}",
            dumped.text,
        );
        assert!(dumped.text.contains("(value 7)"));

        // And the reparse is stable.
        let reloaded =
            from_str_with::<Box<dyn Widget>>(&dumped.text, std::time::Duration::ZERO, &CoreSugar)
                .expect("reparse");
        assert_eq!(
            to_string_with(&reloaded.registry, &CoreSugar)
                .expect("rewrite")
                .text,
            dumped.text,
        );
    }
}
