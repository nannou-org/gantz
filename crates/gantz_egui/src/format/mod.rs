//! A human- and LLM-readable text representation for gantz graphs.
//!
//! This is the `.gantz` save format: a sequence of S-expression forms that is
//! reader-valid Steel (so embedded node code needs no escaping and tooling can
//! reuse Steel's reader), yet describes a whole [`Export`](crate::export::Export)
//! - named graphs, their commit history, references between them, and view
//! layout - without requiring the author to know any content addresses.
//!
//! The pipeline is `text -> [parse] -> File AST -> [lower] -> Export` and back
//! via `Export -> [raise] -> File AST -> [writer] -> text`. Node payloads cross
//! the typetag boundary through a self-describing `serde_json::Value`
//! ([`node_value`]).

mod error;
mod lower;
mod model;
mod node_value;
mod parse;
mod raise;
mod sugar;
mod writer;

pub use error::{ErrorKind, FormatError, Span};
pub use lower::Lowerable;
pub use model::*;
pub use parse::parse_file;

use crate::export::Export;
use gantz_ca::Timestamp;
use gantz_core::node::graph::Graph;

/// Parse a `.gantz` document into an [`Export`].
///
/// `now` provides the timestamp for any commit that the document does not
/// describe explicitly (i.e. hand-authored graphs with no `(history ...)`).
pub fn from_str<N>(text: &str, now: Timestamp) -> Result<Export<Graph<N>>, FormatError>
where
    N: Lowerable,
{
    let file = parse_file(text)?;
    lower::lower(file, now)
}

/// Serialize an [`Export`] to a `.gantz` document.
pub fn to_string<N>(export: &Export<Graph<N>>) -> Result<String, FormatError>
where
    N: Lowerable,
{
    let file = raise::raise(export)?;
    Ok(writer::write_file(&file))
}
