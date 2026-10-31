//! Importers, which turn the bytes of a file into a blob for a node.
//!
//! A node that holds the content of a file, such as an audio sample, refers
//! to the content as a blob. A `(file)` control in the view of the node names
//! an [`Importer`]. The host lets the user pick a file, runs the importer on
//! the bytes of the file and stores the blob that the importer gives. Then it
//! sends the edit of the control to the node, with the value that
//! [`imported`] reads: the address of the blob and the data of the importer.
//! The blob lives while a node refers to it.

use gantz_ca::{ContentAddr, Datum};
use std::sync::Arc;

/// An importer of a domain. See the module docs.
#[derive(Clone)]
pub struct Importer {
    /// The name that a `(file)` control gives.
    pub name: String,
    /// What the importer reads, for the file dialog of the host.
    pub description: String,
    /// The extensions of the files that it reads, such as `wav`.
    pub extensions: Vec<String>,
    /// The section of the registry that holds its blobs.
    pub section: String,
    /// Reads the bytes of a file into the blob and the data that the node
    /// needs beside the blob, such as the number of channels of a sample.
    pub import: Arc<ImportFn>,
}

/// Reads the bytes of a file. See [`Importer::import`].
pub type ImportFn = dyn Fn(&[u8]) -> Result<(Vec<u8>, Datum), String> + Send + Sync;

/// The key of the address of the blob in the value of an import edit.
pub const ADDR: &str = "addr";
/// The key of the data of the importer in the value of an import edit.
pub const DATA: &str = "data";

/// The value of an import edit: the address of the blob and the data of the
/// importer.
pub fn edit_value(addr: ContentAddr, data: Datum) -> Datum {
    Datum::Map(vec![
        (ADDR.to_string(), Datum::Str(addr.to_string())),
        (DATA.to_string(), data),
    ])
}

/// The address of the blob and the data of the importer in the value of an
/// import edit, as [`edit_value`] writes them.
pub fn imported(value: &Datum) -> Option<(ContentAddr, &Datum)> {
    let addr = value.get(ADDR)?.as_str()?.parse().ok()?;
    Some((addr, value.get(DATA)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_values_round_trip() {
        let addr = ContentAddr([9; 32]);
        let data = Datum::Map(vec![("channels".into(), Datum::I64(2))]);
        let value = edit_value(addr, data.clone());
        assert_eq!(imported(&value), Some((addr, &data)));
        assert_eq!(imported(&Datum::Null), None);
    }
}
