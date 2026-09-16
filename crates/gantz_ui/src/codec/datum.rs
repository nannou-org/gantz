//! The `Datum` codec, the storage and interchange encoding of the tree.
//!
//! `Datum` has no symbol variant, so identifiers and strings both map to
//! [`Datum::Str`]. The decoder's ident-or-string tolerance rule exists for
//! exactly this. Raising is therefore not injective. A datum round trip
//! normalizes identifiers to strings while decoding to the same tree.
//! Nulls, characters, byte buffers and maps lower to [`SExpr::Other`] so the
//! decoder can report them by name.

use crate::decode::{Decoded, Limits};
use crate::elem::Element;
use crate::sexpr::SExpr;
use gantz_core::datum::Datum;

/// Lower a datum into the abstract model. Total.
pub fn lower(datum: &Datum) -> SExpr {
    match datum {
        Datum::Bool(b) => SExpr::Bool(*b),
        Datum::I64(i) => SExpr::Int(*i),
        Datum::U64(u) => match i64::try_from(*u) {
            Ok(i) => SExpr::Int(i),
            Err(_) => SExpr::Other("an out-of-range integer".to_string()),
        },
        Datum::F64(f) if f.is_finite() => SExpr::Float(*f),
        Datum::F64(_) => SExpr::Other("a non-finite number".to_string()),
        Datum::Str(s) => SExpr::Str(s.clone()),
        Datum::Seq(items) => SExpr::List(items.iter().map(lower).collect()),
        Datum::Null => SExpr::Other("null".to_string()),
        Datum::Char(_) => SExpr::Other("a character".to_string()),
        Datum::Bytes(_) => SExpr::Other("a byte buffer".to_string()),
        Datum::Map(_) => SExpr::Other("a map".to_string()),
    }
}

/// Raise an abstract value into a datum.
///
/// Total. Identifiers and strings both become [`Datum::Str`]. `Other` never
/// comes out of the encoder. It raises to its description string for
/// completeness.
pub fn raise(expr: &SExpr) -> Datum {
    match expr {
        SExpr::Ident(s) | SExpr::Str(s) => Datum::Str(s.clone()),
        SExpr::Bool(b) => Datum::Bool(*b),
        SExpr::Int(i) => Datum::I64(*i),
        SExpr::Float(f) => Datum::F64(*f),
        SExpr::List(items) => Datum::Seq(items.iter().map(raise).collect()),
        SExpr::Other(s) => Datum::Str(s.clone()),
    }
}

/// Decode a UI tree from a datum. Total. See [`crate::decode::decode`].
pub fn decode(datum: &Datum, limits: &Limits) -> Decoded {
    crate::decode::decode(lower(datum), limits)
}

/// Encode an element as a datum in canonical form. See
/// [`crate::encode::encode`].
pub fn encode(elem: &Element) -> Datum {
    raise(&crate::encode::encode(elem))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lower_maps_atoms() {
        assert_eq!(lower(&Datum::Bool(true)), SExpr::Bool(true));
        assert_eq!(lower(&Datum::I64(-3)), SExpr::Int(-3));
        assert_eq!(lower(&Datum::U64(3)), SExpr::Int(3));
        assert_eq!(
            lower(&Datum::U64(u64::MAX)),
            SExpr::Other("an out-of-range integer".to_string())
        );
        assert_eq!(lower(&Datum::F64(1.5)), SExpr::Float(1.5));
        assert_eq!(
            lower(&Datum::Str("hi".into())),
            SExpr::Str("hi".to_string())
        );
        assert_eq!(lower(&Datum::Null), SExpr::Other("null".to_string()));
        assert_eq!(
            lower(&Datum::Char('x')),
            SExpr::Other("a character".to_string())
        );
        assert_eq!(
            lower(&Datum::Bytes(vec![1])),
            SExpr::Other("a byte buffer".to_string())
        );
        assert_eq!(
            lower(&Datum::Map(vec![])),
            SExpr::Other("a map".to_string())
        );
    }

    #[test]
    fn raise_normalizes_identifiers_to_strings() {
        assert_eq!(
            raise(&SExpr::Ident("col".to_string())),
            Datum::Str("col".to_string())
        );
        assert_eq!(
            raise(&SExpr::Str("col".to_string())),
            Datum::Str("col".to_string())
        );
        let expr = SExpr::List(vec![SExpr::Ident("gap".to_string()), SExpr::Int(4)]);
        assert_eq!(
            lower(&raise(&expr)),
            SExpr::List(vec![SExpr::Str("gap".to_string()), SExpr::Int(4)])
        );
    }
}
