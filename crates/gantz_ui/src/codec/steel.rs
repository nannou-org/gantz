//! The Steel runtime codec, the v1 backend encoding of the tree.
//!
//! Symbols lower to identifiers and strings to strings. Steel lists and
//! vectors both lower to lists, which matches how the rest of gantz treats
//! the two interchangeably. Non-finite numbers and values outside the
//! abstract model lower to [`SExpr::Other`] so the decoder can report them
//! by name.

use crate::decode::{Decoded, Limits};
use crate::elem::Element;
use crate::sexpr::SExpr;
use steel::SteelVal;

/// Lower a Steel value into the abstract model. Total.
pub fn lower(val: &SteelVal) -> SExpr {
    match val {
        SteelVal::SymbolV(s) => SExpr::Ident(s.to_string()),
        SteelVal::StringV(s) => SExpr::Str(s.to_string()),
        SteelVal::BoolV(b) => SExpr::Bool(*b),
        SteelVal::IntV(i) => SExpr::Int(*i as i64),
        // Steel stores integers beyond `isize` as big integers. On 32 bit
        // targets that includes much of the i64 range.
        SteelVal::BigNum(n) => match i64::try_from(n.as_ref()) {
            Ok(i) => SExpr::Int(i),
            Err(_) => SExpr::Other("an integer beyond the i64 range".to_string()),
        },
        SteelVal::NumV(f) if f.is_finite() => SExpr::Float(*f),
        SteelVal::NumV(_) => SExpr::Other("a non-finite number".to_string()),
        SteelVal::ListV(items) => SExpr::List(items.iter().map(lower).collect()),
        SteelVal::VectorV(items) => SExpr::List(items.iter().map(lower).collect()),
        SteelVal::CharV(_) => SExpr::Other("a character".to_string()),
        SteelVal::Void => SExpr::Other("void".to_string()),
        SteelVal::HashMapV(_) => SExpr::Other("a hash map".to_string()),
        SteelVal::HashSetV(_) => SExpr::Other("a hash set".to_string()),
        SteelVal::FuncV(_) | SteelVal::MutFunc(_) | SteelVal::BoxedFunction(_) => {
            SExpr::Other("a function".to_string())
        }
        SteelVal::Closure(_) => SExpr::Other("a function".to_string()),
        _ => SExpr::Other("an unsupported runtime value".to_string()),
    }
}

/// Raise an abstract value into a Steel value.
///
/// Total. `Other` never comes out of the encoder. It raises to its
/// description string for completeness. Integers beyond the `isize` range
/// raise to big integers, as Steel stores them.
pub fn raise(expr: &SExpr) -> SteelVal {
    match expr {
        SExpr::Ident(s) => SteelVal::SymbolV(s.as_str().into()),
        SExpr::Bool(b) => SteelVal::BoolV(*b),
        SExpr::Int(i) => SteelVal::from(*i),
        SExpr::Float(f) => SteelVal::NumV(*f),
        SExpr::Str(s) => SteelVal::StringV(s.as_str().into()),
        SExpr::List(items) => SteelVal::ListV(items.iter().map(raise).collect()),
        SExpr::Other(s) => SteelVal::StringV(s.as_str().into()),
    }
}

/// Decode a UI tree from a Steel value. Total. See [`crate::decode::decode`].
pub fn decode(val: &SteelVal, limits: &Limits) -> Decoded {
    crate::decode::decode(lower(val), limits)
}

/// Encode an element as a Steel value in canonical form. See
/// [`crate::encode::encode`].
pub fn encode(elem: &Element) -> SteelVal {
    raise(&crate::encode::encode(elem))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lower_maps_atoms() {
        assert_eq!(
            lower(&SteelVal::SymbolV("col".into())),
            SExpr::Ident("col".to_string())
        );
        assert_eq!(
            lower(&SteelVal::StringV("hi".into())),
            SExpr::Str("hi".to_string())
        );
        assert_eq!(lower(&SteelVal::BoolV(true)), SExpr::Bool(true));
        assert_eq!(lower(&SteelVal::IntV(3)), SExpr::Int(3));
        assert_eq!(lower(&SteelVal::NumV(1.5)), SExpr::Float(1.5));
        assert_eq!(
            lower(&SteelVal::NumV(f64::NAN)),
            SExpr::Other("a non-finite number".to_string())
        );
        assert_eq!(
            lower(&SteelVal::CharV('x')),
            SExpr::Other("a character".to_string())
        );
    }

    #[test]
    fn raise_then_lower_is_identity_on_the_model() {
        let exprs = [
            SExpr::Ident("col".to_string()),
            SExpr::Bool(false),
            SExpr::Int(-7),
            SExpr::Float(2.25),
            SExpr::Str("hi".to_string()),
            SExpr::List(vec![SExpr::Ident("gap".to_string()), SExpr::Int(4)]),
        ];
        for expr in exprs {
            assert_eq!(lower(&raise(&expr)), expr);
        }
    }

    // The full i64 range round-trips. On 32 bit targets the extremes pass
    // through big integers.
    #[test]
    fn i64_extremes_round_trip() {
        for i in [i64::MIN, i64::MAX] {
            assert_eq!(lower(&raise(&SExpr::Int(i))), SExpr::Int(i));
        }
    }

    // A big integer lowers to an int when it fits i64, and to `Other` when
    // it does not.
    #[test]
    fn lower_maps_big_integers() {
        let big = |i: i64| SteelVal::BigNum(steel::gc::Gc::new(i.into()));
        assert_eq!(lower(&big(-5)), SExpr::Int(-5));
        assert_eq!(lower(&big(i64::MAX)), SExpr::Int(i64::MAX));
        assert_eq!(
            lower(&SteelVal::from(u128::MAX)),
            SExpr::Other("an integer beyond the i64 range".to_string())
        );
    }
}
