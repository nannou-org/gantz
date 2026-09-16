//! A self-describing serde value. It is the bridge between node types and
//! any self-describing representation of them.
//!
//! [`Datum`] mirrors the serde data model the way `serde_json::Value` does.
//! Node types cross format boundaries through this single seam. [`to_datum`]
//! and [`from_datum`] are a serde Serializer and Deserializer pair built
//! directly on `Datum`. Every node's own `Serialize` and `Deserialize` runs
//! unchanged, and arbitrary serde types are supported, not just erased node
//! sets. The `gantz_format` crate maps a `Datum` to and from reader-valid
//! Steel text.
//!
//! The deserializer is self-describing. `Datum`'s `deserialize_any`
//! dispatches each datum kind to the matching `visit_*`. Tag-dispatched
//! node-set serde in `gantz_format::impl_node_set_serde!` and
//! `#[serde(tag = ...)]` derives rely on this.
//!
//! The one divergence from `serde_json::Value` is that `char` and `bytes`
//! keep dedicated variants, [`Datum::Char`] and [`Datum::Bytes`], rather
//! than collapsing to a string or array. The full serde data model then
//! round-trips faithfully. For parity, `deserialize_str` and
//! `deserialize_string` still accept a `Char`, and `deserialize_bytes` still
//! accepts a `Seq`.

use serde::de::{
    self, Deserialize, DeserializeOwned, DeserializeSeed, EnumAccess, Expected, IntoDeserializer,
    MapAccess, SeqAccess, Unexpected, VariantAccess, Visitor,
};
use serde::ser::{self, Serialize};
use std::fmt;
use std::vec;

/// A self-describing value mirroring the serde data model. It is the bridge
/// between node types and reader-valid Steel text.
///
/// `Datum` has bit-level value semantics. `PartialEq`, `Eq` and `Hash`
/// compare [`Datum::F64`] via `to_bits`, so equality is total, reflexive and
/// hash-consistent. This diverges from `f64` semantics only for values that
/// `to_datum` never produces. NaN compares equal to itself and `0.0 != -0.0`.
#[derive(Clone, Debug)]
pub enum Datum {
    /// `null`, unit or `None`. Written as the `null` symbol.
    Null,
    /// A boolean. Written as `#t` or `#f`.
    Bool(bool),
    /// A signed integer. Written as a decimal literal.
    I64(i64),
    /// An unsigned integer. Written as a decimal literal.
    U64(u64),
    /// A finite float. Written as a decimal literal, always with a `.` or an
    /// exponent.
    F64(f64),
    /// A character. Written as a Steel character literal such as `#\c`.
    Char(char),
    /// A string. Written as a string literal.
    Str(String),
    /// A byte buffer. Written as a Steel bytevector, `#u8(...)`.
    Bytes(Vec<u8>),
    /// A sequence or tuple. Written as a Steel vector, `#(...)`.
    Seq(Vec<Datum>),
    /// A map, struct or struct variant. Written as a list of pairs,
    /// `((k v)...)`.
    Map(Vec<(String, Datum)>),
}

/// An error produced by the [`Datum`] serde codec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatumError(String);

/// Serialize any [`Serialize`] value into a [`Datum`].
pub fn to_datum<T>(value: &T) -> Result<Datum, DatumError>
where
    T: Serialize + ?Sized,
{
    value.serialize(Serializer)
}

/// Deserialize any owned [`Deserialize`] value from a [`Datum`].
pub fn from_datum<T>(datum: Datum) -> Result<T, DatumError>
where
    T: DeserializeOwned,
{
    T::deserialize(datum)
}

impl Datum {
    /// Build a node datum. A `type` field holding the node's wire tag is
    /// prepended to `fields`. This is the single way the format constructs a
    /// tagged map. `gantz_format::Sugar` impls usually want the `&str`-keyed
    /// [`node_datum`] convenience instead.
    pub fn tagged(tag: &str, fields: Vec<(String, Datum)>) -> Datum {
        let mut entries = Vec::with_capacity(fields.len() + 1);
        entries.push(("type".to_string(), Datum::Str(tag.to_string())));
        entries.extend(fields);
        Datum::Map(entries)
    }

    /// The value of the map entry `key`, if this is a map containing it.
    pub fn get(&self, key: &str) -> Option<&Datum> {
        match self {
            Datum::Map(entries) => entries
                .iter()
                .find(|(k, _)| k.as_str() == key)
                .map(|(_, v)| v),
            _ => None,
        }
    }

    /// The contents of a string datum.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Datum::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The value of a boolean datum.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Datum::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The value of an integer datum, signed or unsigned, when it fits in
    /// `i64`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Datum::I64(n) => Some(*n),
            Datum::U64(n) => i64::try_from(*n).ok(),
            _ => None,
        }
    }

    /// The value of a float datum, coercing integer datums to `f64`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Datum::F64(n) => Some(*n),
            Datum::I64(n) => Some(*n as f64),
            Datum::U64(n) => Some(*n as f64),
            _ => None,
        }
    }

    /// The elements of a sequence datum.
    pub fn as_seq(&self) -> Option<&[Datum]> {
        match self {
            Datum::Seq(items) => Some(items),
            _ => None,
        }
    }

    /// Recursively sort map entries by key, in place.
    ///
    /// [`to_datum`] preserves a struct's field declaration order, while the
    /// codec's free-form map serialization sorts keys. The same logical value
    /// can therefore take two shapes. Contexts that treat datums as identity,
    /// such as content addressing of ref extension data, canonicalize first
    /// so one logical value has exactly one form.
    pub fn canonicalize(&mut self) {
        match self {
            Datum::Seq(items) => items.iter_mut().for_each(Self::canonicalize),
            Datum::Map(entries) => {
                entries.iter_mut().for_each(|(_, v)| v.canonicalize());
                entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            }
            _ => (),
        }
    }
}

impl PartialEq for Datum {
    fn eq(&self, other: &Self) -> bool {
        use Datum::*;
        match (self, other) {
            (Null, Null) => true,
            (Bool(a), Bool(b)) => a == b,
            (I64(a), I64(b)) => a == b,
            (U64(a), U64(b)) => a == b,
            (F64(a), F64(b)) => a.to_bits() == b.to_bits(),
            (Char(a), Char(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (Bytes(a), Bytes(b)) => a == b,
            (Seq(a), Seq(b)) => a == b,
            (Map(a), Map(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Datum {}

impl std::hash::Hash for Datum {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Datum::Null => (),
            Datum::Bool(b) => b.hash(state),
            Datum::I64(n) => n.hash(state),
            Datum::U64(n) => n.hash(state),
            Datum::F64(x) => x.to_bits().hash(state),
            Datum::Char(c) => c.hash(state),
            Datum::Str(s) => s.hash(state),
            Datum::Bytes(b) => b.hash(state),
            Datum::Seq(items) => items.hash(state),
            Datum::Map(entries) => entries.hash(state),
        }
    }
}

impl crate::CaHash for Datum {
    /// Content-address folding over the datum's structure.
    ///
    /// A variant marker byte plus length-prefixed variable-size contents keep
    /// distinct values from colliding through adjacency. For example,
    /// `Str("ab")` followed by another value must differ from `Str("abc")`.
    /// `F64` folds its bit pattern, matching the bitwise `Eq` and `Hash`
    /// semantics. Identity-sensitive callers hash the canonical form so one
    /// logical value has exactly one address. See [`Datum::canonicalize`].
    fn hash(&self, hasher: &mut crate::Hasher) {
        fn len(hasher: &mut crate::Hasher, n: usize) {
            hasher.update(&(n as u64).to_be_bytes());
        }
        match self {
            Datum::Null => {
                hasher.update(&[0]);
            }
            Datum::Bool(b) => {
                hasher.update(&[1, *b as u8]);
            }
            Datum::I64(n) => {
                hasher.update(&[2]);
                hasher.update(&n.to_be_bytes());
            }
            Datum::U64(n) => {
                hasher.update(&[3]);
                hasher.update(&n.to_be_bytes());
            }
            Datum::F64(x) => {
                hasher.update(&[4]);
                hasher.update(&x.to_bits().to_be_bytes());
            }
            Datum::Char(c) => {
                hasher.update(&[5]);
                hasher.update(&(*c as u32).to_be_bytes());
            }
            Datum::Str(s) => {
                hasher.update(&[6]);
                len(hasher, s.len());
                hasher.update(s.as_bytes());
            }
            Datum::Bytes(b) => {
                hasher.update(&[7]);
                len(hasher, b.len());
                hasher.update(b);
            }
            Datum::Seq(items) => {
                hasher.update(&[8]);
                len(hasher, items.len());
                for item in items {
                    crate::CaHash::hash(item, hasher);
                }
            }
            Datum::Map(entries) => {
                hasher.update(&[9]);
                len(hasher, entries.len());
                for (k, v) in entries {
                    len(hasher, k.len());
                    hasher.update(k.as_bytes());
                    crate::CaHash::hash(v, hasher);
                }
            }
        }
    }
}

impl Serialize for Datum {
    /// Serializes the represented value. `Str("x")` serializes as a string
    /// and `Map` as a map. A datum embedded in a larger `Serialize` type thus
    /// round-trips through any self-describing format. Note that
    /// `to_datum(&datum) == datum` holds only for canonical datums, since
    /// this codec's own map serialization sorts keys. See
    /// [`Datum::canonicalize`].
    fn serialize<S: ser::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap as _, SerializeSeq as _};
        match self {
            Datum::Null => s.serialize_unit(),
            Datum::Bool(b) => s.serialize_bool(*b),
            Datum::I64(n) => s.serialize_i64(*n),
            Datum::U64(n) => s.serialize_u64(*n),
            Datum::F64(x) => s.serialize_f64(*x),
            Datum::Char(c) => s.serialize_char(*c),
            Datum::Str(v) => s.serialize_str(v),
            Datum::Bytes(b) => s.serialize_bytes(b),
            Datum::Seq(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Datum::Map(entries) => {
                let mut map = s.serialize_map(Some(entries.len()))?;
                for (k, v) in entries {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

/// Build a node datum from a node tag and ordered `&str`-keyed fields.
///
/// A `gantz_format::Sugar` uses this to construct the value its `read_spec`
/// returns, without depending on `Datum`'s internal map shape.
pub fn node_datum(tag: &str, fields: Vec<(&str, Datum)>) -> Datum {
    Datum::tagged(
        tag,
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

impl fmt::Display for DatumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DatumError {}

impl ser::Error for DatumError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        DatumError(msg.to_string())
    }
}

impl de::Error for DatumError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        DatumError(msg.to_string())
    }
}

struct Serializer;

impl ser::Serializer for Serializer {
    type Ok = Datum;
    type Error = DatumError;

    type SerializeSeq = SerializeSeq;
    type SerializeTuple = SerializeSeq;
    type SerializeTupleStruct = SerializeSeq;
    type SerializeTupleVariant = SerializeTupleVariant;
    type SerializeMap = SerializeMap;
    type SerializeStruct = SerializeStruct;
    type SerializeStructVariant = SerializeStructVariant;

    fn serialize_bool(self, v: bool) -> Result<Datum, DatumError> {
        Ok(Datum::Bool(v))
    }

    fn serialize_i8(self, v: i8) -> Result<Datum, DatumError> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i16(self, v: i16) -> Result<Datum, DatumError> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i32(self, v: i32) -> Result<Datum, DatumError> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i64(self, v: i64) -> Result<Datum, DatumError> {
        Ok(Datum::I64(v))
    }

    fn serialize_i128(self, v: i128) -> Result<Datum, DatumError> {
        if let Ok(v) = u64::try_from(v) {
            Ok(Datum::U64(v))
        } else if let Ok(v) = i64::try_from(v) {
            Ok(Datum::I64(v))
        } else {
            Err(DatumError("i128 out of range".into()))
        }
    }

    fn serialize_u8(self, v: u8) -> Result<Datum, DatumError> {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u16(self, v: u16) -> Result<Datum, DatumError> {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u32(self, v: u32) -> Result<Datum, DatumError> {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u64(self, v: u64) -> Result<Datum, DatumError> {
        Ok(Datum::U64(v))
    }

    fn serialize_u128(self, v: u128) -> Result<Datum, DatumError> {
        match u64::try_from(v) {
            Ok(v) => Ok(Datum::U64(v)),
            Err(_) => Err(DatumError("u128 out of range".into())),
        }
    }

    fn serialize_f32(self, v: f32) -> Result<Datum, DatumError> {
        self.serialize_f64(f64::from(v))
    }

    fn serialize_f64(self, v: f64) -> Result<Datum, DatumError> {
        // Mirror `Number::from_f64`. A non-finite float has no representation.
        Ok(if v.is_finite() {
            Datum::F64(v)
        } else {
            Datum::Null
        })
    }

    fn serialize_char(self, v: char) -> Result<Datum, DatumError> {
        Ok(Datum::Char(v))
    }

    fn serialize_str(self, v: &str) -> Result<Datum, DatumError> {
        Ok(Datum::Str(v.to_owned()))
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Datum, DatumError> {
        Ok(Datum::Bytes(v.to_vec()))
    }

    fn serialize_none(self) -> Result<Datum, DatumError> {
        Ok(Datum::Null)
    }

    fn serialize_some<T>(self, value: &T) -> Result<Datum, DatumError>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Datum, DatumError> {
        Ok(Datum::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Datum, DatumError> {
        Ok(Datum::Null)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Datum, DatumError> {
        Ok(Datum::Str(variant.to_owned()))
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Datum, DatumError>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Datum, DatumError>
    where
        T: Serialize + ?Sized,
    {
        Ok(Datum::Map(vec![(variant.to_owned(), to_datum(value)?)]))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<SerializeSeq, DatumError> {
        Ok(SerializeSeq {
            vec: Vec::with_capacity(len.unwrap_or(0)),
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<SerializeSeq, DatumError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<SerializeSeq, DatumError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<SerializeTupleVariant, DatumError> {
        Ok(SerializeTupleVariant {
            name: variant.to_owned(),
            vec: Vec::with_capacity(len),
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<SerializeMap, DatumError> {
        Ok(SerializeMap {
            entries: Vec::new(),
            next_key: None,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<SerializeStruct, DatumError> {
        Ok(SerializeStruct {
            entries: Vec::with_capacity(len),
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<SerializeStructVariant, DatumError> {
        Ok(SerializeStructVariant {
            name: variant.to_owned(),
            entries: Vec::with_capacity(len),
        })
    }

    fn collect_str<T>(self, value: &T) -> Result<Datum, DatumError>
    where
        T: fmt::Display + ?Sized,
    {
        Ok(Datum::Str(value.to_string()))
    }
}

struct SerializeSeq {
    vec: Vec<Datum>,
}

impl ser::SerializeSeq for SerializeSeq {
    type Ok = Datum;
    type Error = DatumError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), DatumError>
    where
        T: Serialize + ?Sized,
    {
        self.vec.push(to_datum(value)?);
        Ok(())
    }

    fn end(self) -> Result<Datum, DatumError> {
        Ok(Datum::Seq(self.vec))
    }
}

impl ser::SerializeTuple for SerializeSeq {
    type Ok = Datum;
    type Error = DatumError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), DatumError>
    where
        T: Serialize + ?Sized,
    {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Datum, DatumError> {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleStruct for SerializeSeq {
    type Ok = Datum;
    type Error = DatumError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), DatumError>
    where
        T: Serialize + ?Sized,
    {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Datum, DatumError> {
        ser::SerializeSeq::end(self)
    }
}

struct SerializeTupleVariant {
    name: String,
    vec: Vec<Datum>,
}

impl ser::SerializeTupleVariant for SerializeTupleVariant {
    type Ok = Datum;
    type Error = DatumError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), DatumError>
    where
        T: Serialize + ?Sized,
    {
        self.vec.push(to_datum(value)?);
        Ok(())
    }

    fn end(self) -> Result<Datum, DatumError> {
        Ok(Datum::Map(vec![(self.name, Datum::Seq(self.vec))]))
    }
}

/// Map serialization sorts keys for deterministic output. Map iteration
/// order, such as a `HashMap`'s, is unspecified. Struct field order is
/// preserved.
struct SerializeMap {
    entries: Vec<(String, Datum)>,
    next_key: Option<String>,
}

impl ser::SerializeMap for SerializeMap {
    type Ok = Datum;
    type Error = DatumError;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), DatumError>
    where
        T: Serialize + ?Sized,
    {
        self.next_key = Some(key.serialize(MapKeySerializer)?);
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), DatumError>
    where
        T: Serialize + ?Sized,
    {
        let key = self
            .next_key
            .take()
            .expect("serialize_value called before serialize_key");
        self.entries.push((key, to_datum(value)?));
        Ok(())
    }

    fn end(mut self) -> Result<Datum, DatumError> {
        self.entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Datum::Map(self.entries))
    }
}

struct SerializeStruct {
    entries: Vec<(String, Datum)>,
}

impl ser::SerializeStruct for SerializeStruct {
    type Ok = Datum;
    type Error = DatumError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), DatumError>
    where
        T: Serialize + ?Sized,
    {
        self.entries.push((key.to_owned(), to_datum(value)?));
        Ok(())
    }

    fn end(self) -> Result<Datum, DatumError> {
        Ok(Datum::Map(self.entries))
    }
}

struct SerializeStructVariant {
    name: String,
    entries: Vec<(String, Datum)>,
}

impl ser::SerializeStructVariant for SerializeStructVariant {
    type Ok = Datum;
    type Error = DatumError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), DatumError>
    where
        T: Serialize + ?Sized,
    {
        self.entries.push((key.to_owned(), to_datum(value)?));
        Ok(())
    }

    fn end(self) -> Result<Datum, DatumError> {
        Ok(Datum::Map(vec![(self.name, Datum::Map(self.entries))]))
    }
}

/// Serializes a map key to a `String`, mirroring `serde_json`'s map-key
/// rules. Only string and scalar keys are allowed.
struct MapKeySerializer;

fn key_must_be_a_string() -> DatumError {
    DatumError("map key must be a string".into())
}

impl ser::Serializer for MapKeySerializer {
    type Ok = String;
    type Error = DatumError;

    type SerializeSeq = ser::Impossible<String, DatumError>;
    type SerializeTuple = ser::Impossible<String, DatumError>;
    type SerializeTupleStruct = ser::Impossible<String, DatumError>;
    type SerializeTupleVariant = ser::Impossible<String, DatumError>;
    type SerializeMap = ser::Impossible<String, DatumError>;
    type SerializeStruct = ser::Impossible<String, DatumError>;
    type SerializeStructVariant = ser::Impossible<String, DatumError>;

    fn serialize_bool(self, v: bool) -> Result<String, DatumError> {
        Ok(if v { "true" } else { "false" }.to_owned())
    }

    fn serialize_i8(self, v: i8) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_i16(self, v: i16) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_i32(self, v: i32) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_i64(self, v: i64) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_i128(self, v: i128) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_u8(self, v: u8) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_u16(self, v: u16) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_u32(self, v: u32) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_u64(self, v: u64) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_u128(self, v: u128) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_f32(self, v: f32) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_f64(self, v: f64) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_char(self, v: char) -> Result<String, DatumError> {
        Ok(v.to_string())
    }

    fn serialize_str(self, v: &str) -> Result<String, DatumError> {
        Ok(v.to_owned())
    }

    fn serialize_bytes(self, _v: &[u8]) -> Result<String, DatumError> {
        Err(key_must_be_a_string())
    }

    fn serialize_none(self) -> Result<String, DatumError> {
        Err(key_must_be_a_string())
    }

    fn serialize_some<T>(self, _value: &T) -> Result<String, DatumError>
    where
        T: Serialize + ?Sized,
    {
        Err(key_must_be_a_string())
    }

    fn serialize_unit(self) -> Result<String, DatumError> {
        Err(key_must_be_a_string())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<String, DatumError> {
        Err(key_must_be_a_string())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<String, DatumError> {
        Ok(variant.to_owned())
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<String, DatumError>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<String, DatumError>
    where
        T: Serialize + ?Sized,
    {
        Err(key_must_be_a_string())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, DatumError> {
        Err(key_must_be_a_string())
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, DatumError> {
        Err(key_must_be_a_string())
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, DatumError> {
        Err(key_must_be_a_string())
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, DatumError> {
        Err(key_must_be_a_string())
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, DatumError> {
        Err(key_must_be_a_string())
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, DatumError> {
        Err(key_must_be_a_string())
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, DatumError> {
        Err(key_must_be_a_string())
    }

    fn collect_str<T>(self, value: &T) -> Result<String, DatumError>
    where
        T: fmt::Display + ?Sized,
    {
        Ok(value.to_string())
    }
}

/// Transcode into a `Datum` from any self-describing format, mirroring
/// `serde_json::Value`'s `Deserialize`. The input is buffered as a `Datum`,
/// which can then re-drive a concrete type's `Deserialize` via
/// [`from_datum`]. `gantz_format::impl_node_set_serde!` buffers node fields
/// this way when they precede the `type` tag.
impl<'de> Deserialize<'de> for Datum {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct DatumVisitor;

        impl<'de> Visitor<'de> for DatumVisitor {
            type Value = Datum;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any valid datum")
            }

            fn visit_bool<E>(self, b: bool) -> Result<Datum, E> {
                Ok(Datum::Bool(b))
            }

            fn visit_i64<E>(self, n: i64) -> Result<Datum, E> {
                Ok(Datum::I64(n))
            }

            fn visit_u64<E>(self, n: u64) -> Result<Datum, E> {
                Ok(Datum::U64(n))
            }

            fn visit_f64<E>(self, n: f64) -> Result<Datum, E> {
                Ok(Datum::F64(n))
            }

            fn visit_char<E>(self, c: char) -> Result<Datum, E> {
                Ok(Datum::Char(c))
            }

            fn visit_str<E>(self, s: &str) -> Result<Datum, E> {
                Ok(Datum::Str(s.to_string()))
            }

            fn visit_string<E>(self, s: String) -> Result<Datum, E> {
                Ok(Datum::Str(s))
            }

            fn visit_bytes<E>(self, b: &[u8]) -> Result<Datum, E> {
                Ok(Datum::Bytes(b.to_vec()))
            }

            fn visit_byte_buf<E>(self, b: Vec<u8>) -> Result<Datum, E> {
                Ok(Datum::Bytes(b))
            }

            fn visit_none<E>(self) -> Result<Datum, E> {
                Ok(Datum::Null)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Datum, D::Error>
            where
                D: de::Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer)
            }

            fn visit_unit<E>(self) -> Result<Datum, E> {
                Ok(Datum::Null)
            }

            fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Datum, D::Error>
            where
                D: de::Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Datum, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(4096));
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Datum::Seq(items))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Datum, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0).min(4096));
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Datum::Map(entries))
            }
        }

        deserializer.deserialize_any(DatumVisitor)
    }
}

impl Datum {
    fn invalid_type<E>(&self, exp: &dyn Expected) -> E
    where
        E: de::Error,
    {
        de::Error::invalid_type(self.unexpected(), exp)
    }

    fn unexpected(&self) -> Unexpected<'_> {
        match self {
            Datum::Null => Unexpected::Unit,
            Datum::Bool(b) => Unexpected::Bool(*b),
            Datum::I64(n) => Unexpected::Signed(*n),
            Datum::U64(n) => Unexpected::Unsigned(*n),
            Datum::F64(n) => Unexpected::Float(*n),
            Datum::Char(c) => Unexpected::Char(*c),
            Datum::Str(s) => Unexpected::Str(s),
            Datum::Bytes(b) => Unexpected::Bytes(b),
            Datum::Seq(_) => Unexpected::Seq,
            Datum::Map(_) => Unexpected::Map,
        }
    }
}

/// Dispatch a numeric datum to the visitor by its concrete kind. Non-numbers
/// are a type error. This mirrors `serde_json`'s number deserialization.
fn deserialize_number<'de, V>(datum: Datum, visitor: V) -> Result<V::Value, DatumError>
where
    V: Visitor<'de>,
{
    match datum {
        Datum::I64(n) => visitor.visit_i64(n),
        Datum::U64(n) => visitor.visit_u64(n),
        Datum::F64(n) => visitor.visit_f64(n),
        other => Err(other.invalid_type(&visitor)),
    }
}

macro_rules! deserialize_number_method {
    ($method:ident) => {
        fn $method<V>(self, visitor: V) -> Result<V::Value, DatumError>
        where
            V: Visitor<'de>,
        {
            deserialize_number(self, visitor)
        }
    };
}

impl<'de> de::Deserializer<'de> for Datum {
    type Error = DatumError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            Datum::Null => visitor.visit_unit(),
            Datum::Bool(v) => visitor.visit_bool(v),
            Datum::I64(n) => visitor.visit_i64(n),
            Datum::U64(n) => visitor.visit_u64(n),
            Datum::F64(n) => visitor.visit_f64(n),
            Datum::Char(c) => visitor.visit_char(c),
            Datum::Str(s) => visitor.visit_string(s),
            Datum::Bytes(b) => visitor.visit_byte_buf(b),
            Datum::Seq(v) => visit_seq(v, visitor),
            Datum::Map(m) => visit_map(m, visitor),
        }
    }

    deserialize_number_method!(deserialize_i8);
    deserialize_number_method!(deserialize_i16);
    deserialize_number_method!(deserialize_i32);
    deserialize_number_method!(deserialize_i64);
    deserialize_number_method!(deserialize_i128);
    deserialize_number_method!(deserialize_u8);
    deserialize_number_method!(deserialize_u16);
    deserialize_number_method!(deserialize_u32);
    deserialize_number_method!(deserialize_u64);
    deserialize_number_method!(deserialize_u128);
    deserialize_number_method!(deserialize_f32);
    deserialize_number_method!(deserialize_f64);

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            Datum::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            // An enum is encoded as a single-key map from variant to payload...
            Datum::Map(mut entries) if entries.len() == 1 => {
                let (variant, value) = entries.pop().expect("len == 1");
                visitor.visit_enum(EnumDeserializer {
                    variant,
                    value: Some(value),
                })
            }
            Datum::Map(_) => Err(de::Error::invalid_value(
                Unexpected::Map,
                &"map with a single key",
            )),
            // ...or a bare string for a unit variant.
            Datum::Str(variant) => visitor.visit_enum(EnumDeserializer {
                variant,
                value: None,
            }),
            other => Err(de::Error::invalid_type(
                other.unexpected(),
                &"string or map",
            )),
        }
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            Datum::Bool(v) => visitor.visit_bool(v),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            Datum::Char(c) => visitor.visit_char(c),
            // Parity with `serde_json`, which encodes `char` as a string.
            Datum::Str(s) => visitor.visit_string(s),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        self.deserialize_string(visitor)
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            Datum::Str(s) => visitor.visit_string(s),
            // For parity, a `char` can satisfy a string target.
            Datum::Char(c) => visitor.visit_string(c.to_string()),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        self.deserialize_byte_buf(visitor)
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            Datum::Bytes(b) => visitor.visit_byte_buf(b),
            Datum::Str(s) => visitor.visit_string(s),
            // For parity, a seq of byte-valued numbers can satisfy a bytes
            // target.
            Datum::Seq(v) => visit_seq(v, visitor),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            Datum::Null => visitor.visit_unit(),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            // A unit-struct node's datum is an empty map once its `type` tag
            // is split off. This mirrors `serde`'s own internally-tagged
            // special case for newtype variants around unit structs.
            Datum::Map(ref entries) if entries.is_empty() => visitor.visit_unit(),
            _ => self.deserialize_unit(visitor),
        }
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            Datum::Seq(v) => visit_seq(v, visitor),
            Datum::Bytes(b) => visit_seq(
                b.into_iter().map(|b| Datum::U64(u64::from(b))).collect(),
                visitor,
            ),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            Datum::Map(m) => visit_map(m, visitor),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self {
            Datum::Map(m) => visit_map(m, visitor),
            Datum::Seq(v) => visit_seq(v, visitor),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        self.deserialize_string(visitor)
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }
}

impl<'de> IntoDeserializer<'de, DatumError> for Datum {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

fn visit_seq<'de, V>(seq: Vec<Datum>, visitor: V) -> Result<V::Value, DatumError>
where
    V: Visitor<'de>,
{
    let len = seq.len();
    let mut de = SeqDeserializer {
        iter: seq.into_iter(),
    };
    let value = visitor.visit_seq(&mut de)?;
    if de.iter.len() == 0 {
        Ok(value)
    } else {
        Err(de::Error::invalid_length(
            len,
            &"fewer elements in sequence",
        ))
    }
}

fn visit_map<'de, V>(map: Vec<(String, Datum)>, visitor: V) -> Result<V::Value, DatumError>
where
    V: Visitor<'de>,
{
    let len = map.len();
    let mut de = MapDeserializer {
        iter: map.into_iter(),
        value: None,
    };
    let value = visitor.visit_map(&mut de)?;
    if de.iter.len() == 0 {
        Ok(value)
    } else {
        Err(de::Error::invalid_length(len, &"fewer elements in map"))
    }
}

struct SeqDeserializer {
    iter: vec::IntoIter<Datum>,
}

impl<'de> SeqAccess<'de> for SeqDeserializer {
    type Error = DatumError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, DatumError>
    where
        T: DeserializeSeed<'de>,
    {
        match self.iter.next() {
            Some(value) => seed.deserialize(value).map(Some),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

struct MapDeserializer {
    iter: vec::IntoIter<(String, Datum)>,
    value: Option<Datum>,
}

impl<'de> MapAccess<'de> for MapDeserializer {
    type Error = DatumError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, DatumError>
    where
        K: DeserializeSeed<'de>,
    {
        match self.iter.next() {
            Some((key, value)) => {
                self.value = Some(value);
                seed.deserialize(MapKeyDeserializer { key }).map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, DatumError>
    where
        V: DeserializeSeed<'de>,
    {
        let value = self.value.take().expect("next_value before next_key");
        seed.deserialize(value)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

/// Deserializer for a map key. A `String` that can also satisfy numeric,
/// bool and unit-variant-enum targets. This mirrors `serde_json`'s map keys.
struct MapKeyDeserializer {
    key: String,
}

fn expected_numeric_key<T>() -> Result<T, DatumError> {
    Err(DatumError("expected a numeric map key".into()))
}

macro_rules! deserialize_numeric_key {
    ($method:ident, $visit:ident, $ty:ty) => {
        fn $method<V>(self, visitor: V) -> Result<V::Value, DatumError>
        where
            V: Visitor<'de>,
        {
            match self.key.parse::<$ty>() {
                Ok(n) => visitor.$visit(n),
                Err(_) => expected_numeric_key(),
            }
        }
    };
}

impl<'de> de::Deserializer<'de> for MapKeyDeserializer {
    type Error = DatumError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_string(self.key)
    }

    deserialize_numeric_key!(deserialize_i8, visit_i8, i8);
    deserialize_numeric_key!(deserialize_i16, visit_i16, i16);
    deserialize_numeric_key!(deserialize_i32, visit_i32, i32);
    deserialize_numeric_key!(deserialize_i64, visit_i64, i64);
    deserialize_numeric_key!(deserialize_i128, visit_i128, i128);
    deserialize_numeric_key!(deserialize_u8, visit_u8, u8);
    deserialize_numeric_key!(deserialize_u16, visit_u16, u16);
    deserialize_numeric_key!(deserialize_u32, visit_u32, u32);
    deserialize_numeric_key!(deserialize_u64, visit_u64, u64);
    deserialize_numeric_key!(deserialize_u128, visit_u128, u128);
    deserialize_numeric_key!(deserialize_f32, visit_f32, f32);
    deserialize_numeric_key!(deserialize_f64, visit_f64, f64);

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self.key.as_str() {
            "true" => visitor.visit_bool(true),
            "false" => visitor.visit_bool(false),
            _ => Err(de::Error::invalid_type(
                Unexpected::Str(&self.key),
                &visitor,
            )),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        self.key
            .into_deserializer()
            .deserialize_enum(name, variants, visitor)
    }

    serde::forward_to_deserialize_any! {
        char str string bytes byte_buf unit unit_struct seq tuple tuple_struct
        map struct identifier ignored_any
    }
}

struct EnumDeserializer {
    variant: String,
    value: Option<Datum>,
}

impl<'de> EnumAccess<'de> for EnumDeserializer {
    type Error = DatumError;
    type Variant = VariantDeserializer;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, VariantDeserializer), DatumError>
    where
        V: DeserializeSeed<'de>,
    {
        let variant = self.variant.into_deserializer();
        let visitor = VariantDeserializer { value: self.value };
        seed.deserialize(variant).map(|v| (v, visitor))
    }
}

struct VariantDeserializer {
    value: Option<Datum>,
}

impl<'de> VariantAccess<'de> for VariantDeserializer {
    type Error = DatumError;

    fn unit_variant(self) -> Result<(), DatumError> {
        match self.value {
            Some(value) => Deserialize::deserialize(value),
            None => Ok(()),
        }
    }

    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, DatumError>
    where
        T: DeserializeSeed<'de>,
    {
        match self.value {
            Some(value) => seed.deserialize(value),
            None => Err(de::Error::invalid_type(
                Unexpected::UnitVariant,
                &"newtype variant",
            )),
        }
    }

    fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Some(Datum::Seq(v)) if v.is_empty() => visitor.visit_unit(),
            Some(Datum::Seq(v)) => visit_seq(v, visitor),
            Some(other) => Err(de::Error::invalid_type(
                other.unexpected(),
                &"tuple variant",
            )),
            None => Err(de::Error::invalid_type(
                Unexpected::UnitVariant,
                &"tuple variant",
            )),
        }
    }

    fn struct_variant<V>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DatumError>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Some(Datum::Map(m)) => visit_map(m, visitor),
            Some(other) => Err(de::Error::invalid_type(
                other.unexpected(),
                &"struct variant",
            )),
            None => Err(de::Error::invalid_type(
                Unexpected::UnitVariant,
                &"struct variant",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: &[(&str, Datum)]) -> Datum {
        Datum::Map(
            entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    /// Distinct datums fold to distinct addresses. Variant markers and length
    /// prefixes prevent adjacency and cross-variant collisions.
    #[test]
    fn ca_hash_distinctness() {
        fn ca(d: &Datum) -> crate::ContentAddr {
            crate::content_addr(d)
        }
        // Same numeric value, different variant.
        assert_ne!(ca(&Datum::I64(1)), ca(&Datum::U64(1)));
        // Empty containers are distinct.
        assert_ne!(ca(&Datum::Map(vec![])), ca(&Datum::Seq(vec![])));
        // Adjacent strings must not collide via length ambiguity.
        assert_ne!(
            ca(&Datum::Seq(vec![
                Datum::Str("ab".into()),
                Datum::Str("c".into())
            ])),
            ca(&Datum::Seq(vec![
                Datum::Str("a".into()),
                Datum::Str("bc".into())
            ])),
        );
        // Key vs value content must not blur.
        assert_ne!(
            ca(&map(&[("ab", Datum::Str("c".into()))])),
            ca(&map(&[("a", Datum::Str("bc".into()))])),
        );
    }

    /// Pin the fold so accidental scheme changes are caught. Ext data is
    /// content-addressed, so this scheme is part of the wire format.
    #[test]
    fn ca_hash_stability_pin() {
        let d = map(&[
            ("flag", Datum::Bool(true)),
            ("ratio", Datum::F64(1.5)),
            (
                "tags",
                Datum::Seq(vec![Datum::Str("a".into()), Datum::Null]),
            ),
        ]);
        assert_eq!(
            crate::content_addr(&d).to_string(),
            "62ff57c18727d269cad8e0cfd62856b57efbecec761aadc2caad0073445dd09d",
            "Datum CaHash scheme changed - this breaks existing ext-carrying addresses",
        );
    }

    /// `Eq` and `Hash` are bitwise for floats. They are total, reflexive and
    /// hash-consistent.
    #[test]
    fn value_semantics_are_bitwise() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        fn h(d: &Datum) -> u64 {
            let mut s = DefaultHasher::new();
            d.hash(&mut s);
            s.finish()
        }
        let nan = Datum::F64(f64::NAN);
        assert_eq!(nan, nan.clone());
        assert_eq!(h(&nan), h(&nan.clone()));
        assert_ne!(Datum::F64(0.0), Datum::F64(-0.0));
        assert_eq!(Datum::F64(1.5), Datum::F64(1.5));
        assert_ne!(Datum::I64(1), Datum::U64(1));
    }

    /// A canonical datum round-trips through `to_datum` unchanged, and
    /// canonicalization recursively sorts map keys.
    #[test]
    fn canonical_datum_roundtrips_through_to_datum() {
        let mut d = map(&[
            (
                "b",
                Datum::Seq(vec![map(&[("z", Datum::Null), ("a", Datum::Bool(true))])]),
            ),
            ("a", Datum::I64(1)),
        ]);
        // Non-canonical. `to_datum` sorts the maps, so the value changes shape.
        assert_ne!(to_datum(&d).unwrap(), d);
        d.canonicalize();
        assert_eq!(
            d,
            map(&[
                ("a", Datum::I64(1)),
                (
                    "b",
                    Datum::Seq(vec![map(&[("a", Datum::Bool(true)), ("z", Datum::Null)])])
                ),
            ])
        );
        assert_eq!(to_datum(&d).unwrap(), d);
        let rt: Datum = from_datum(d.clone()).unwrap();
        assert_eq!(rt, d);
    }
}
