//! A human- and LLM-readable text format for gantz graph registries.
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

mod error;
mod lower;
mod model;
mod node_value;
mod parse;
mod raise;
mod sugar;
mod writer;

pub mod sexpr;

pub use error::{ErrorKind, FormatError, Span};
pub use lower::{Loaded, Lowerable};
pub use model::{Addr, Document, Form};
pub use raise::{Dumped, GraphLabels};

use gantz_ca::{Registry, Timestamp};
use gantz_core::node::graph::Graph;

/// Parse a `.gantz` document into its [`Loaded`] registry, resolution context
/// and preserved extra forms.
///
/// `now` provides the timestamp for any graph the `(commits ...)` table does not
/// describe (hand-authored graphs with no commit entry).
pub fn from_str<N>(text: &str, now: Timestamp) -> Result<Loaded<N>, FormatError>
where
    N: Lowerable,
{
    let doc = parse::parse(text)?;
    lower::lower(doc, now)
}

/// Serialize a registry to `.gantz` text, returning the text along with the
/// per-graph label context an extender needs to emit its own forms.
pub fn to_string<N>(registry: &Registry<Graph<N>>) -> Result<Dumped, FormatError>
where
    N: Lowerable,
{
    raise::raise(registry)
}
