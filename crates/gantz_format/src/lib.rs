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
//! and [`to_string`] use [`DefaultSugar`] (gantz's built-ins); the `_with`
//! variants accept any `&dyn Sugar` so other node sets can supply their own
//! first-class keywords while still falling back to the generic `(node ...)`
//! form. Any node type that is `Serialize + DeserializeOwned + CaHash` works.

mod datum;
mod error;
mod lower;
mod model;
mod parse;
mod raise;
mod sugar;
mod writer;

pub mod sexpr;

pub use datum::{Datum, DatumError, datum_from_expr, datum_text, from_datum, to_datum};
pub use error::{ErrorKind, FormatError, Span};
pub use lower::{Loaded, Lowerable};
pub use model::{Addr, Document, Form};
pub use raise::{Dumped, GraphLabels};
pub use sugar::{DefaultSugar, Sugar, Sugars};

use gantz_ca::{Registry, Timestamp};
use gantz_core::node::graph::Graph;

/// Parse a `.gantz` document (with gantz's built-in node keywords) into its
/// [`Loaded`] registry, resolution context and preserved extra forms.
///
/// `now` provides the timestamp for any graph the `(commits ...)` table does not
/// describe (hand-authored graphs with no commit entry).
pub fn from_str<N>(text: &str, now: Timestamp) -> Result<Loaded<N>, FormatError>
where
    N: Lowerable,
{
    from_str_with(text, now, &DefaultSugar)
}

/// Parse a `.gantz` document using a custom keyword [`Sugar`] (compose with
/// [`DefaultSugar`] via [`Sugars`] to keep the built-ins).
pub fn from_str_with<N>(
    text: &str,
    now: Timestamp,
    sugar: &dyn Sugar,
) -> Result<Loaded<N>, FormatError>
where
    N: Lowerable,
{
    let doc = parse::parse(text, sugar)?;
    lower::lower(doc, now)
}

/// Serialize a registry to `.gantz` text (with gantz's built-in node keywords),
/// returning the text along with the per-graph label context an extender needs
/// to emit its own forms.
pub fn to_string<N>(registry: &Registry<Graph<N>>) -> Result<Dumped, FormatError>
where
    N: Lowerable,
{
    to_string_with(registry, &DefaultSugar)
}

/// Serialize a registry to `.gantz` text using a custom keyword [`Sugar`].
pub fn to_string_with<N>(
    registry: &Registry<Graph<N>>,
    sugar: &dyn Sugar,
) -> Result<Dumped, FormatError>
where
    N: Lowerable,
{
    raise::raise(registry, sugar)
}
