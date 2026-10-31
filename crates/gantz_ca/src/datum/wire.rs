//! A fixed-shape encoding of a [`Datum`] for formats that are not
//! self-describing, such as postcard.
//!
//! The `Serialize` and `Deserialize` of `Datum` follow the serde data model,
//! and the deserializer needs `deserialize_any`. This encoding writes a datum
//! as an enum with one variant per datum variant, which any serde format can
//! carry. Use it on a field with `#[serde(with = "gantz_ca::datum::wire")]`, or
//! on a `Vec<Datum>` field with `#[serde(with = "gantz_ca::datum::wire::seq")]`.

use super::Datum;
use serde::Deserialize;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeSeq, Serializer};
use std::fmt;

/// The fixed-shape encoding of a sequence of datums. See the module docs.
pub mod seq {
    use super::{Items, Owned, datum_of, invalid_ratio};
    use crate::datum::Datum;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Write `items` in the fixed-shape encoding.
    pub fn serialize<S: Serializer>(items: &[Datum], s: S) -> Result<S::Ok, S::Error> {
        Items(items).serialize(s)
    }

    /// Read a sequence of datums in the fixed-shape encoding.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Datum>, D::Error> {
        let items = Vec::<Owned>::deserialize(d)?;
        let items = items.into_iter().map(datum_of).collect::<Option<_>>();
        items.ok_or_else(invalid_ratio)
    }
}

/// A datum, as the encoding writes it.
struct Wire<'a>(&'a Datum);

/// The items of a sequence datum.
struct Items<'a>(&'a [Datum]);

/// The entries of a map datum.
struct Entries<'a>(&'a [(String, Datum)]);

/// The bytes of a byte buffer datum.
struct Bytes<'a>(&'a [u8]);

/// A datum, as the encoding reads it. The variants match the indices that
/// [`Wire`] writes.
#[derive(Deserialize)]
#[serde(rename = "Datum")]
enum Owned {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Char(char),
    Str(String),
    Bytes(ByteBuf),
    Seq(Vec<Owned>),
    Map(Vec<(String, Owned)>),
    Sym(String),
    Ratio((i64, i64)),
}

/// An owned byte buffer, read from bytes or from a sequence of bytes.
struct ByteBuf(Vec<u8>);

/// The enum name of the encoding.
const NAME: &str = "Datum";

impl Serialize for Wire<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Datum::Null => s.serialize_unit_variant(NAME, 0, "Null"),
            Datum::Bool(b) => s.serialize_newtype_variant(NAME, 1, "Bool", b),
            Datum::I64(n) => s.serialize_newtype_variant(NAME, 2, "I64", n),
            Datum::U64(n) => s.serialize_newtype_variant(NAME, 3, "U64", n),
            Datum::F64(x) => s.serialize_newtype_variant(NAME, 4, "F64", x),
            Datum::Char(c) => s.serialize_newtype_variant(NAME, 5, "Char", c),
            Datum::Str(v) => s.serialize_newtype_variant(NAME, 6, "Str", v),
            Datum::Bytes(b) => s.serialize_newtype_variant(NAME, 7, "Bytes", &Bytes(b)),
            Datum::Seq(items) => s.serialize_newtype_variant(NAME, 8, "Seq", &Items(items)),
            Datum::Map(entries) => s.serialize_newtype_variant(NAME, 9, "Map", &Entries(entries)),
            Datum::Sym(name) => s.serialize_newtype_variant(NAME, 10, "Sym", name),
            Datum::Ratio(r) => {
                let parts = (r.numer(), r.denom());
                s.serialize_newtype_variant(NAME, 11, "Ratio", &parts)
            }
        }
    }
}

impl Serialize for Items<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for item in self.0 {
            seq.serialize_element(&Wire(item))?;
        }
        seq.end()
    }
}

impl Serialize for Entries<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for (k, v) in self.0 {
            seq.serialize_element(&(k, Wire(v)))?;
        }
        seq.end()
    }
}

impl Serialize for Bytes<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(self.0)
    }
}

impl<'de> Deserialize<'de> for ByteBuf {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ByteBufVisitor;

        impl<'de> Visitor<'de> for ByteBufVisitor {
            type Value = ByteBuf;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("bytes")
            }

            fn visit_bytes<E>(self, b: &[u8]) -> Result<ByteBuf, E> {
                Ok(ByteBuf(b.to_vec()))
            }

            fn visit_byte_buf<E>(self, b: Vec<u8>) -> Result<ByteBuf, E> {
                Ok(ByteBuf(b))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ByteBuf, A::Error> {
                let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(4096));
                while let Some(b) = seq.next_element()? {
                    bytes.push(b);
                }
                Ok(ByteBuf(bytes))
            }
        }

        d.deserialize_byte_buf(ByteBufVisitor)
    }
}

/// Write `datum` in the fixed-shape encoding. See the module docs.
pub fn serialize<S: Serializer>(datum: &Datum, s: S) -> Result<S::Ok, S::Error> {
    Wire(datum).serialize(s)
}

/// Read a datum in the fixed-shape encoding. See the module docs.
pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Datum, D::Error> {
    datum_of(Owned::deserialize(d)?).ok_or_else(invalid_ratio)
}

/// The error of a ratio over zero, or of one that does not fit.
fn invalid_ratio<E: de::Error>() -> E {
    E::custom("a ratio over zero, or one that does not fit")
}

/// The datum of `owned`, or `None` when it holds a ratio over zero, or one
/// that does not fit. A ratio takes its canonical form.
fn datum_of(owned: Owned) -> Option<Datum> {
    let datum = match owned {
        Owned::Null => Datum::Null,
        Owned::Bool(b) => Datum::Bool(b),
        Owned::I64(n) => Datum::I64(n),
        Owned::U64(n) => Datum::from_u64(n),
        Owned::F64(x) => Datum::F64(x),
        Owned::Char(c) => Datum::Char(c),
        Owned::Str(s) => Datum::Str(s),
        Owned::Bytes(b) => Datum::Bytes(b.0),
        Owned::Seq(items) => Datum::Seq(items.into_iter().map(datum_of).collect::<Option<_>>()?),
        Owned::Map(entries) => Datum::Map(
            entries
                .into_iter()
                .map(|(k, v)| Some((k, datum_of(v)?)))
                .collect::<Option<_>>()?,
        ),
        Owned::Sym(name) => Datum::Sym(name),
        Owned::Ratio((n, d)) => Datum::ratio(n, d)?,
    };
    Some(datum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Envelope {
        #[serde(with = "crate::datum::wire")]
        value: Datum,
    }

    /// Every variant round-trips through postcard, which is not
    /// self-describing.
    #[test]
    fn every_variant_round_trips_through_postcard() {
        let value = Datum::Seq(vec![
            Datum::Null,
            Datum::Bool(true),
            Datum::I64(-3),
            Datum::U64(u64::MAX),
            Datum::F64(0.5),
            Datum::Char('g'),
            Datum::Str("text".into()),
            Datum::Bytes(vec![0, 255]),
            Datum::Map(vec![("k".into(), Datum::Sym("v".into()))]),
            Datum::ratio(-1, 3).unwrap(),
        ]);
        let envelope = Envelope { value };
        let bytes = postcard::to_allocvec(&envelope).unwrap();
        let back: Envelope = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, envelope);
    }

    /// A ratio takes its canonical form, and a ratio over zero is refused.
    /// Postcard writes the variant index, then each part as a zigzag varint.
    #[test]
    fn a_ratio_is_canonical_and_never_over_zero() {
        let read = |bytes: &[u8]| deserialize(&mut postcard::Deserializer::from_bytes(bytes));
        assert_eq!(read(&[11, 4, 8]).unwrap(), Datum::ratio(1, 2).unwrap());
        assert_eq!(read(&[11, 8, 4]).unwrap(), Datum::I64(2));
        assert!(read(&[11, 2, 0]).is_err());
    }
}
