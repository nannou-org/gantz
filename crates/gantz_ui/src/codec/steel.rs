//! The Steel runtime codec, the v1 backend encoding of the tree.
//!
//! A Steel value becomes a datum through [`gantz_core::datum`]. Symbols are
//! identifiers, and Steel lists and vectors both read as lists. A value within
//! the tree that has no data form, such as a function or a non-finite number,
//! keeps its slot, and diagnostics name it.

use crate::decode::{Decoded, Limits};
use crate::elem::Element;
use gantz_ca::Datum;
use gantz_core::datum;
use steel::SteelVal;

/// The datum of a Steel value. Total. A value with no data form becomes a
/// placeholder that [`crate::summary`] names by what the value was.
pub fn lower(val: &SteelVal) -> Datum {
    datum::of_steel_lossy(val)
}

/// Decode a UI tree from a Steel value. Total. See [`crate::decode::decode`].
pub fn decode(val: &SteelVal, limits: &Limits) -> Decoded {
    crate::decode::decode(lower(val), limits)
}

/// Encode an element as a Steel value in canonical form. See
/// [`crate::encode::encode`].
pub fn encode(elem: &Element) -> SteelVal {
    datum::to_steel(&crate::encode::encode(elem))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::ErrorReason;
    use steel::gc::Gc;
    use steel::steel_vm::engine::Engine;

    fn sym(s: &str) -> SteelVal {
        SteelVal::SymbolV(s.into())
    }

    fn svec(items: Vec<SteelVal>) -> SteelVal {
        let v: steel::Vector<SteelVal> = items.into_iter().collect();
        SteelVal::VectorV(Gc::new(v).into())
    }

    fn list(items: Vec<SteelVal>) -> SteelVal {
        SteelVal::ListV(items.into_iter().collect())
    }

    /// The reason of the error element of the first child of a `col`.
    fn first_child_error(decoded: &Decoded) -> &ErrorReason {
        let Element::Col(col) = &decoded.root else {
            panic!("expected a col: {:?}", decoded.root);
        };
        let Element::Error(e) = &col.children[0] else {
            panic!("expected an error: {:?}", col.children[0]);
        };
        &e.reason
    }

    #[test]
    fn vectors_decode_like_lists() {
        let sep = svec(vec![sym("sep")]);
        let as_vec = svec(vec![sym("col"), sep.clone(), sep]);
        let decoded = decode(&as_vec, &Limits::default());
        let Element::Col(col) = decoded.root else {
            panic!("expected a col");
        };
        assert_eq!(col.children.len(), 2);
    }

    /// A function in the tree is an inline error that names it, and the rest
    /// of the tree decodes.
    #[test]
    fn a_function_keeps_its_slot() {
        let vm = &mut Engine::new_base();
        let f = vm.run("(lambda (x) x)".to_string()).unwrap().pop().unwrap();
        let tree = list(vec![sym("col"), f, list(vec![sym("sep")])]);
        let decoded = decode(&tree, &Limits::default());
        assert_eq!(
            *first_child_error(&decoded),
            ErrorReason::NotAnElement {
                found: "a function".to_string()
            }
        );
    }

    /// An integer beyond 64 bits is a value outside the form.
    #[test]
    fn a_huge_integer_is_named() {
        let tree = list(vec![sym("col"), SteelVal::from(u128::MAX)]);
        let decoded = decode(&tree, &Limits::default());
        assert_eq!(
            *first_child_error(&decoded),
            ErrorReason::NotAnElement {
                found: "an integer beyond 64 bits".to_string()
            }
        );
    }
}
