//! Runtime codecs between value types and the tree.
//!
//! A codec converts a runtime value to and from a [`Datum`](gantz_ca::Datum),
//! and composes that with the shared decoder and encoder.

#[cfg(feature = "steel")]
pub mod steel;
