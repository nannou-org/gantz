//! A self-describing serde value and codec over reader-valid Steel datums.
//!
//! [`Datum`] mirrors the serde data model the way `serde_json::Value` does, but
//! its text form is reader-valid Steel rather than JSON. Node types cross the
//! format boundary through this single seam: [`to_datum`]/[`from_datum`] are a
//! `serde` [`Serializer`]/[`Deserializer`] pair built directly on `Datum`
//! (mirroring `serde_json`'s own `Value` codec), so every node's own
//! `Serialize`/`Deserialize` runs unchanged and arbitrary serde types - not
//! just `typetag` trait objects - are supported. [`datum_text`] and
//! [`datum_from_expr`] map a `Datum` to and from Steel text.
//!
//! The deserializer is *self-describing*: [`Datum::deserialize_any`] dispatches
//! each datum kind to the matching `visit_*`, which is what `typetag`'s content
//! buffering and `#[serde(tag = ...)]` ride on.
//!
//! The one deliberate divergence from `serde_json::Value` is that `char` and
//! `bytes` keep dedicated variants ([`Datum::Char`]/[`Datum::Bytes`]) rather
//! than collapsing to a string/array, so the full serde data model round-trips
//! faithfully. For parity, `deserialize_str`/`deserialize_string` still accept a
//! `Char` and `deserialize_bytes` still accepts a `Seq`.

use crate::sexpr::{self, list_args, quote, span_src};
use serde::de::{
    self, Deserialize, DeserializeOwned, DeserializeSeed, EnumAccess, Expected, IntoDeserializer,
    MapAccess, SeqAccess, Unexpected, VariantAccess, Visitor,
};
use serde::ser::{self, Serialize};
use std::fmt;
use std::vec;
use steel::parser::ast::{Atom, ExprKind};
use steel::parser::tokens::TokenType;

/// A self-describing value mirroring the serde data model; the bridge between
/// node types and reader-valid Steel text.
#[derive(Clone, Debug, PartialEq)]
pub enum Datum {
    /// `null` / unit / `None` -> the `null` symbol.
    Null,
    /// A boolean -> `#t` / `#f`.
    Bool(bool),
    /// A signed integer -> a decimal literal.
    I64(i64),
    /// An unsigned integer -> a decimal literal.
    U64(u64),
    /// A finite float -> a decimal literal (always with a `.` or exponent).
    F64(f64),
    /// A character -> a Steel character literal (`#\c`).
    Char(char),
    /// A string -> a string literal.
    Str(String),
    /// A byte buffer -> a Steel bytevector (`#u8(...)`).
    Bytes(Vec<u8>),
    /// A sequence (seq / tuple) -> a Steel vector (`#(...)`).
    Seq(Vec<Datum>),
    /// A map (map / struct / struct variant) -> a list of pairs (`((k v)...)`).
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

// -- constructors / accessors ------------------------------------------------

impl Datum {
    /// Build a node datum: a `type` field (the typetag tag) prepended to
    /// `fields`. The single canonical way the format constructs a tagged map.
    pub(crate) fn tagged(tag: &str, fields: Vec<(String, Datum)>) -> Datum {
        let mut entries = Vec::with_capacity(fields.len() + 1);
        entries.push(("type".to_string(), Datum::Str(tag.to_string())));
        entries.extend(fields);
        Datum::Map(entries)
    }

    /// The value of the map entry `key`, if this is a map containing it.
    pub(crate) fn get(&self, key: &str) -> Option<&Datum> {
        match self {
            Datum::Map(entries) => entries
                .iter()
                .find(|(k, _)| k.as_str() == key)
                .map(|(_, v)| v),
            _ => None,
        }
    }

    /// The contents of a string datum.
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Datum::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The value of a boolean datum.
    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self {
            Datum::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The value of an integer datum (signed or unsigned, if it fits in `i64`).
    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            Datum::I64(n) => Some(*n),
            Datum::U64(n) => i64::try_from(*n).ok(),
            _ => None,
        }
    }

    /// The value of a float datum, coercing integer datums to `f64`.
    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Datum::F64(n) => Some(*n),
            Datum::I64(n) => Some(*n as f64),
            Datum::U64(n) => Some(*n as f64),
            _ => None,
        }
    }

    /// The elements of a sequence datum.
    pub(crate) fn as_seq(&self) -> Option<&[Datum]> {
        match self {
            Datum::Seq(items) => Some(items),
            _ => None,
        }
    }
}

// -- error -------------------------------------------------------------------

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

// -- serializer --------------------------------------------------------------

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
        // Mirror `Number::from_f64`: a non-finite float has no representation.
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

/// Map serialization sorts keys for deterministic output, since map iteration
/// order (e.g. `HashMap`) is unspecified. Struct field order is preserved.
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

/// Serializes a map key to a `String`, mirroring `serde_json`'s map-key rules
/// (only stringy/scalar keys are allowed).
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

// -- deserializer ------------------------------------------------------------

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

/// Dispatch a numeric datum to the visitor by its concrete kind; non-numbers
/// are a type error (mirrors `serde_json`'s number deserialization).
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
            // An enum is encoded as a single-key map (variant -> payload)...
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
            // Parity: a `char` can satisfy a string target.
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
            // Parity: a seq of byte-valued numbers can satisfy a bytes target.
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
        self.deserialize_unit(visitor)
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

/// Deserializer for a map key: a `String` that can also satisfy numeric, bool
/// and unit-variant-enum targets (mirrors `serde_json`'s map keys).
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

// -- text <-> datum ----------------------------------------------------------

/// Render a [`Datum`] as a reader-valid Steel datum.
pub fn datum_text(d: &Datum) -> String {
    match d {
        Datum::Null => "null".to_string(),
        Datum::Bool(true) => "#t".to_string(),
        Datum::Bool(false) => "#f".to_string(),
        Datum::I64(n) => n.to_string(),
        Datum::U64(n) => n.to_string(),
        Datum::F64(x) => float_text(*x),
        Datum::Char(c) => char_text(*c),
        Datum::Str(s) => quote(s),
        Datum::Bytes(b) => {
            let inner = b.iter().map(u8::to_string).collect::<Vec<_>>().join(" ");
            format!("#u8({inner})")
        }
        Datum::Seq(items) => {
            let inner = items.iter().map(datum_text).collect::<Vec<_>>().join(" ");
            format!("#({inner})")
        }
        Datum::Map(entries) => {
            let inner = entries
                .iter()
                .map(|(k, v)| format!("({} {})", key_text(k), datum_text(v)))
                .collect::<Vec<_>>()
                .join(" ");
            format!("({inner})")
        }
    }
}

/// Read a [`Datum`] from a Steel datum expression. Numbers are read from their
/// verbatim source slice (via `src`); seqs are vectors (`#(...)`) and maps are
/// bare lists of `(key value)` pairs.
pub fn datum_from_expr(e: &ExprKind, src: &str) -> Datum {
    match e {
        ExprKind::Vector(v) if v.bytes => {
            Datum::Bytes(v.args.iter().filter_map(|a| byte_of(a, src)).collect())
        }
        ExprKind::Vector(v) => Datum::Seq(v.args.iter().map(|a| datum_from_expr(a, src)).collect()),
        ExprKind::List(list) => Datum::Map(
            list.args
                .iter()
                .filter_map(|item| map_entry(item, src))
                .collect(),
        ),
        ExprKind::Atom(a) => atom_datum(a, e, src),
        _ => Datum::Null,
    }
}

fn map_entry(item: &ExprKind, src: &str) -> Option<(String, Datum)> {
    let args = list_args(item)?;
    if args.len() != 2 {
        return None;
    }
    let key = sexpr::as_symbol(&args[0]).or_else(|| sexpr::as_string(&args[0]))?;
    Some((key, datum_from_expr(&args[1], src)))
}

fn byte_of(e: &ExprKind, src: &str) -> Option<u8> {
    u8::try_from(sexpr::as_i64(e, src)?).ok()
}

fn atom_datum(a: &Atom, e: &ExprKind, src: &str) -> Datum {
    match &a.syn.ty {
        TokenType::StringLiteral(s) => Datum::Str(s.to_string()),
        TokenType::BooleanLiteral(b) => Datum::Bool(*b),
        TokenType::CharacterLiteral(c) => Datum::Char(*c),
        TokenType::Number(_) => number_datum(e, src),
        TokenType::Identifier(s) => match s.resolve() {
            "null" => Datum::Null,
            "true" => Datum::Bool(true),
            "false" => Datum::Bool(false),
            other => Datum::Str(other.to_string()),
        },
        TokenType::Keyword(s) => Datum::Str(s.resolve().to_string()),
        _ => Datum::Null,
    }
}

fn number_datum(e: &ExprKind, src: &str) -> Datum {
    let Some(text) = span_src(e, src) else {
        return Datum::Null;
    };
    if let Ok(i) = text.parse::<i64>() {
        Datum::I64(i)
    } else if let Ok(u) = text.parse::<u64>() {
        Datum::U64(u)
    } else if let Ok(f) = text.parse::<f64>() {
        Datum::F64(f)
    } else {
        Datum::Str(text.to_string())
    }
}

/// Render a float with a guaranteed decimal point (or exponent), so it never
/// reads back as an integer. `{:?}` gives the shortest round-tripping form.
fn float_text(x: f64) -> String {
    let s = format!("{x:?}");
    if s.bytes().any(|b| matches!(b, b'.' | b'e' | b'E')) {
        s
    } else {
        format!("{s}.0")
    }
}

/// Render a `char` exactly as Steel's own reader displays it, so it round-trips.
fn char_text(c: char) -> String {
    match c {
        ' ' => "#\\space".to_string(),
        '\0' => "#\\null".to_string(),
        '\t' => "#\\tab".to_string(),
        '\n' => "#\\newline".to_string(),
        '\r' => "#\\return".to_string(),
        _ if c.escape_debug().count() == 1 => format!("#\\{c}"),
        _ => format!("#\\u{:04x}", c as u32),
    }
}

/// A map key is rendered as a bare symbol when it is identifier-safe (the common
/// case of struct field names), else as a quoted string.
fn key_text(k: &str) -> String {
    let safe = !k.is_empty()
        && k.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if safe { k.to_string() } else { quote(k) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Inner {
        flag: bool,
        ratio: f64,
        tags: Vec<String>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "type")]
    enum MyNode {
        Scalar {
            count: u32,
            offset: i64,
            label: String,
        },
        Nested {
            inner: Inner,
            maybe: Option<u8>,
            extra: Vec<i32>,
        },
        Unit,
    }

    /// Reading a datum from text and rendering it back is stable.
    fn text_roundtrip(d: &Datum) -> Datum {
        let text = datum_text(d);
        let exprs = sexpr::read(&text).expect("read");
        assert_eq!(exprs.len(), 1, "one datum from `{text}`");
        datum_from_expr(&exprs[0], &text)
    }

    #[test]
    fn text_stable_scalars_round_trip() {
        let cases = [
            Datum::Null,
            Datum::Bool(true),
            Datum::Bool(false),
            Datum::I64(-42),
            Datum::F64(3.5),
            Datum::F64(3.0),
            Datum::Char('q'),
            Datum::Str("hello world".to_string()),
        ];
        for d in cases {
            assert_eq!(text_roundtrip(&d), d, "text round-trip for {d:?}");
        }
    }

    /// A non-negative integer renders as bare digits, so it reads back as `I64`
    /// regardless of whether it was a `U64`. This is harmless: a node's field
    /// `Deserialize` produces the same value either way (the `faithful_serde_*`
    /// tests, which compare nodes, are the real guard).
    #[test]
    fn nonnegative_int_normalizes_to_i64() {
        assert_eq!(text_roundtrip(&Datum::U64(42)), Datum::I64(42));
        assert_eq!(text_roundtrip(&Datum::I64(42)), Datum::I64(42));
        // A value beyond i64::MAX still round-trips as U64.
        let big = Datum::U64(u64::MAX);
        assert_eq!(text_roundtrip(&big), big);
    }

    #[test]
    fn empty_map_and_seq_are_distinct() {
        assert_eq!(text_roundtrip(&Datum::Map(vec![])), Datum::Map(vec![]));
        assert_eq!(text_roundtrip(&Datum::Seq(vec![])), Datum::Seq(vec![]));
        assert_eq!(datum_text(&Datum::Map(vec![])), "()");
        assert_eq!(datum_text(&Datum::Seq(vec![])), "#()");
    }

    #[test]
    fn float_valued_integer_stays_a_float() {
        // `3.0` must render with a decimal point so it does not read back as I64.
        assert_eq!(datum_text(&Datum::F64(3.0)), "3.0");
        assert_eq!(text_roundtrip(&Datum::F64(3.0)), Datum::F64(3.0));
    }

    #[test]
    fn bytes_round_trip() {
        let d = Datum::Bytes(vec![0, 1, 255]);
        assert_eq!(datum_text(&d), "#u8(0 1 255)");
        assert_eq!(text_roundtrip(&d), d);
    }

    #[test]
    fn nested_map_and_seq_round_trip() {
        let d = Datum::Map(vec![
            ("a".to_string(), Datum::I64(1)),
            (
                "b".to_string(),
                Datum::Seq(vec![Datum::I64(2), Datum::Str("x".into())]),
            ),
            (
                "c".to_string(),
                Datum::Map(vec![("k".to_string(), Datum::Bool(true))]),
            ),
        ]);
        assert_eq!(text_roundtrip(&d), d);
    }

    #[test]
    fn faithful_serde_struct_variant() {
        let node = MyNode::Nested {
            inner: Inner {
                flag: true,
                ratio: 0.25,
                tags: vec!["a".into(), "b".into()],
            },
            maybe: Some(7),
            extra: vec![-1, 0, 1],
        };
        // In-memory codec is exact.
        let datum = to_datum(&node).expect("to_datum");
        let back: MyNode = from_datum(datum.clone()).expect("from_datum");
        assert_eq!(node, back);
        // And it survives a text round-trip.
        let via_text: MyNode = from_datum(text_roundtrip(&datum)).expect("from text");
        assert_eq!(node, via_text);
    }

    #[test]
    fn faithful_serde_unit_and_scalar_variants() {
        for node in [
            MyNode::Unit,
            MyNode::Scalar {
                count: 3,
                offset: -9,
                label: "hi".into(),
            },
        ] {
            let datum = to_datum(&node).expect("to_datum");
            let via_text: MyNode = from_datum(text_roundtrip(&datum)).expect("from text");
            assert_eq!(node, via_text, "round-trip for {node:?}");
        }
    }

    #[test]
    fn char_specials_round_trip() {
        for c in [' ', '\n', '\t', '\r', '\0', 'a', '✓', '\u{7}'] {
            let d = Datum::Char(c);
            assert_eq!(text_roundtrip(&d), d, "char round-trip for {c:?}");
        }
    }

    // -- additional edge cases -----------------------------------------------

    /// Round-trip a serde value through the codec *and* a text round-trip.
    fn serde_text_roundtrip<T>(value: &T) -> T
    where
        T: Serialize + DeserializeOwned,
    {
        let datum = to_datum(value).expect("to_datum");
        from_datum(text_roundtrip(&datum)).expect("from_datum after text")
    }

    /// Strings whose contents look like another datum kind must stay strings -
    /// quoting is what disambiguates them from `null`/`#t`/numbers on read.
    #[test]
    fn strings_that_look_like_other_datums_stay_strings() {
        for s in [
            "null", "true", "false", "#t", "#f", "42", "-7", "3.0", "-1.5", "1e9", "", " ",
        ] {
            let d = Datum::Str(s.to_string());
            assert_eq!(text_roundtrip(&d), d, "{s:?} must round-trip as a string");
        }
    }

    /// Strings containing characters significant to the reader (quotes, escapes,
    /// parens, comment/keyword markers, unicode) survive quoting and re-reading.
    #[test]
    fn string_escaping_round_trips() {
        for s in [
            "a\"b",         // embedded double quote
            "a\\b",         // embedded backslash
            "line1\nline2", // newline
            "tab\there",    // tab
            "ret\rhere",    // carriage return
            "(not a list)", // parens inside a string
            "semi;colon",   // comment char
            "#:keyword",    // keyword marker
            "✓ unicode ☃",
            "",
        ] {
            let d = Datum::Str(s.to_string());
            assert_eq!(
                text_roundtrip(&d),
                d,
                "escaped string {s:?} must round-trip"
            );
        }
    }

    /// Floats survive a text round-trip bit-exactly, including fractional,
    /// negative, very large/small, and full-precision values.
    #[test]
    fn floats_round_trip_exactly() {
        let cases = [
            0.0,
            -0.5,
            0.5,
            -3.5,
            0.1,
            0.1 + 0.2,
            1.0 / 3.0,
            1e-10,
            1e10,
            1e20,
            1e-20,
            123456.789,
            f64::MIN_POSITIVE,
        ];
        for x in cases {
            match text_roundtrip(&Datum::F64(x)) {
                Datum::F64(y) => assert_eq!(
                    x.to_bits(),
                    y.to_bits(),
                    "float {x} must round-trip exactly (rendered {:?})",
                    float_text(x),
                ),
                other => panic!(
                    "float {x} read back as {other:?} (rendered {:?})",
                    float_text(x)
                ),
            }
        }
    }

    /// Integer boundary values round-trip with the correct variant; a value just
    /// past `i64::MAX` reads back as `U64`, not an overflow.
    #[test]
    fn integer_boundaries_round_trip() {
        assert_eq!(text_roundtrip(&Datum::I64(i64::MIN)), Datum::I64(i64::MIN));
        assert_eq!(text_roundtrip(&Datum::I64(i64::MAX)), Datum::I64(i64::MAX));
        let just_past = Datum::U64(i64::MAX as u64 + 1);
        assert_eq!(text_roundtrip(&just_past), just_past);
    }

    /// Empty collections nested inside collections stay distinct: a seq holding
    /// an empty map (`#(())`) is not a seq holding an empty seq (`#(#())`).
    #[test]
    fn nested_empty_collections_are_distinguished() {
        let seq_of_empty_map = Datum::Seq(vec![Datum::Map(vec![])]);
        let seq_of_empty_seq = Datum::Seq(vec![Datum::Seq(vec![])]);
        assert_ne!(seq_of_empty_map, seq_of_empty_seq);
        assert_eq!(text_roundtrip(&seq_of_empty_map), seq_of_empty_map);
        assert_eq!(text_roundtrip(&seq_of_empty_seq), seq_of_empty_seq);
        let mixed = Datum::Map(vec![
            ("e_seq".into(), Datum::Seq(vec![])),
            ("e_map".into(), Datum::Map(vec![])),
        ]);
        assert_eq!(text_roundtrip(&mixed), mixed);
    }

    /// An empty bytevector renders as `#u8()` and round-trips.
    #[test]
    fn empty_bytes_round_trips() {
        let d = Datum::Bytes(vec![]);
        assert_eq!(datum_text(&d), "#u8()");
        assert_eq!(text_roundtrip(&d), d);
    }

    /// A deeply mixed structure (maps in seqs in maps, nulls/bytes/strings with
    /// reader-significant characters interleaved) round-trips.
    #[test]
    fn deeply_mixed_nesting_round_trips() {
        let d = Datum::Map(vec![
            (
                "rows".into(),
                Datum::Seq(vec![
                    Datum::Map(vec![
                        ("id".into(), Datum::I64(1)),
                        (
                            "vals".into(),
                            Datum::Seq(vec![Datum::F64(1.5), Datum::Null]),
                        ),
                    ]),
                    Datum::Map(vec![
                        ("id".into(), Datum::I64(2)),
                        ("vals".into(), Datum::Seq(vec![])),
                    ]),
                ]),
            ),
            (
                "grid".into(),
                Datum::Seq(vec![
                    Datum::Seq(vec![Datum::I64(0), Datum::I64(1)]),
                    Datum::Seq(vec![Datum::Bool(true), Datum::Str("x".into())]),
                ]),
            ),
            ("raw".into(), Datum::Bytes(vec![1, 2, 3])),
            ("note".into(), Datum::Str("(parens) and \"quotes\"".into())),
        ]);
        assert_eq!(text_roundtrip(&d), d);
    }

    /// Characters significant to the reader (parens, brackets, quotes, hash,
    /// comment char, a digit) round-trip via Steel's character syntax.
    #[test]
    fn reader_significant_chars_round_trip() {
        for c in ['(', ')', '[', ']', '"', '\\', '#', ';', '5', '\''] {
            let d = Datum::Char(c);
            assert_eq!(
                text_roundtrip(&d),
                d,
                "char {c:?} must round-trip (rendered {:?})",
                char_text(c),
            );
        }
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Meters(f64);

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Pair(i32, i32);

    /// An externally tagged enum (serde's default) - the single-key-map / bare
    /// string encoding, distinct from the internally tagged path `MyNode` takes.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    enum Shape {
        Dot,
        Tag(String),
        Span(i32, i32),
        Rect { w: u32, h: u32, fill: bool },
    }

    /// Every externally tagged variant shape round-trips, exercising the
    /// enum/variant-access paths (unit, newtype, tuple and struct variants).
    #[test]
    fn externally_tagged_enum_variants_round_trip() {
        for value in [
            Shape::Dot,
            Shape::Tag("hi".into()),
            Shape::Span(-1, 2),
            Shape::Rect {
                w: 4,
                h: 5,
                fill: true,
            },
        ] {
            assert_eq!(serde_text_roundtrip(&value), value, "variant {value:?}");
        }
    }

    /// Tuple structs, newtype structs and tuples round-trip as sequences.
    #[test]
    fn tuple_and_newtype_serde_shapes_round_trip() {
        assert_eq!(serde_text_roundtrip(&Meters(2.5)), Meters(2.5));
        assert_eq!(serde_text_roundtrip(&Pair(-3, 7)), Pair(-3, 7));
        let tuple = (1u8, "two".to_string(), 3.5f64);
        assert_eq!(serde_text_roundtrip(&tuple), tuple);
    }

    /// Maps with non-identifier string keys and with numeric keys round-trip,
    /// exercising the map-key serializer and deserializer.
    #[test]
    fn map_keys_round_trip() {
        use std::collections::BTreeMap;
        let str_keys: BTreeMap<String, i32> = [
            ("plain".to_string(), 1),
            ("with space".to_string(), 2),
            ("123".to_string(), 3), // digit-leading: must be quoted
            (String::new(), 4),     // empty key: must be quoted
            ("dash-ok".to_string(), 5),
        ]
        .into_iter()
        .collect();
        assert_eq!(serde_text_roundtrip(&str_keys), str_keys);

        let num_keys: BTreeMap<i32, String> = [(-2, "neg".to_string()), (7, "pos".to_string())]
            .into_iter()
            .collect();
        assert_eq!(serde_text_roundtrip(&num_keys), num_keys);
    }

    /// `None` (the null datum) round-trips and stays distinct from a present
    /// value.
    #[test]
    fn option_none_round_trips() {
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        struct Holder {
            a: Option<i32>,
            b: Option<String>,
        }
        let with_none = Holder {
            a: None,
            b: Some("x".into()),
        };
        assert_eq!(serde_text_roundtrip(&with_none), with_none);
        let other = Holder {
            a: Some(0),
            b: None,
        };
        assert_eq!(serde_text_roundtrip(&other), other);
    }

    /// Deserializing a datum into an incompatible type fails cleanly rather than
    /// silently coercing.
    #[test]
    fn type_mismatch_is_an_error() {
        assert!(from_datum::<u8>(Datum::I64(300)).is_err()); // out of range
        assert!(from_datum::<i64>(Datum::Str("nope".into())).is_err());
        assert!(from_datum::<String>(Datum::I64(1)).is_err());
        assert!(from_datum::<bool>(Datum::Null).is_err());
    }
}
