//! Mini-notation tests. The Rust parser's emitted combinator source is
//! evaluated against hand-built combinator expressions on the pin
//! harness.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};
use gantz_core::steel::steel_vm::engine::Engine;
use gantz_pattern::mini::steel_src;

/// Assert the notation's emission and the combinator expression query
/// identically over the span.
fn assert_m_eq(vm: &mut Engine, notation: &str, combinators: &str, span: &str) {
    let emitted = emitted(notation);
    assert_steel_true(
        vm,
        &format!("{notation:?}"),
        &format!(
            "(equal? (pin-events (pat/query {emitted} (pat/span {span})))
                 (pin-events (pat/query {combinators} (pat/span {span}))))",
        ),
    );
}

fn emitted(notation: &str) -> String {
    steel_src(notation).unwrap_or_else(|| panic!("{notation:?} failed to parse"))
}

#[test]
fn notation_matches_combinators() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        ("bd", "(pat/pure 'bd)", "0 3"),
        (
            "bd sn cp td",
            "(pat/fastcat (list (pat/pure 'bd) (pat/pure 'sn) (pat/pure 'cp) (pat/pure 'td)))",
            "0 1",
        ),
        (
            "0 1 2 3 4",
            "(pat/fastcat (list (pat/pure 0) (pat/pure 1) (pat/pure 2) (pat/pure 3) (pat/pure 4)))",
            "0 1",
        ),
        (
            "0 [1 2] 3 [4 5]",
            "(pat/fastcat (list (pat/pure 0)
                                (pat/fastcat (list (pat/pure 1) (pat/pure 2)))
                                (pat/pure 3)
                                (pat/fastcat (list (pat/pure 4) (pat/pure 5)))))",
            "0 1",
        ),
        (
            "bd ~ sn ~",
            "(pat/fastcat (list (pat/pure 'bd) pat/silence (pat/pure 'sn) pat/silence))",
            "0 1",
        ),
        // `_` extends the previous step, assembling via timecat weights.
        (
            "bd _ _ _ sn _",
            "(pat/timecat (list (list 4 (pat/pure 'bd)) (list 2 (pat/pure 'sn))))",
            "0 1",
        ),
        (
            "a b <c d>",
            "(pat/fastcat (list (pat/pure 'a)
                                (pat/pure 'b)
                                (pat/slowcat (list (pat/pure 'c) (pat/pure 'd)))))",
            "0 4",
        ),
        (
            "[bd bd, sn sn sn]",
            "(pat/stack (list (pat/fastcat (list (pat/pure 'bd) (pat/pure 'bd)))
                              (pat/fastcat (list (pat/pure 'sn) (pat/pure 'sn) (pat/pure 'sn)))))",
            "0 1",
        ),
        (
            "bd*2 sn",
            "(pat/fastcat (list (pat/fast 2 (pat/pure 'bd)) (pat/pure 'sn)))",
            "0 1",
        ),
        (
            "[a b]/2",
            "(pat/slow 2 (pat/fastcat (list (pat/pure 'a) (pat/pure 'b))))",
            "0 2",
        ),
        // Modifiers compose with alternation.
        (
            "<a b>*2",
            "(pat/fast 2 (pat/slowcat (list (pat/pure 'a) (pat/pure 'b))))",
            "0 2",
        ),
    ];
    for (notation, combinators, span) in rows {
        assert_m_eq(&mut vm, notation, combinators, span);
    }
}

#[test]
fn equivalent_notations_match() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str)] = &[
        // Redundant nesting collapses to the same events.
        ("[[[bd sn]]]", "bd sn"),
        // `@n` weights a step, equivalent to `_` continuation.
        ("a@3 b", "a _ _ b"),
    ];
    for (a, b) in rows {
        assert_steel_true(
            &mut vm,
            &format!("{a:?} vs {b:?}"),
            &format!(
                "(equal? (pin-events (pat/query {} (pat/span 0 1)))
                     (pin-events (pat/query {} (pat/span 0 1))))",
                emitted(a),
                emitted(b),
            ),
        );
    }
}

#[test]
fn notation_events() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str)] = &[
        // Exact rational atoms survive tokenization with the / between
        // digits.
        (
            "1/4 3/4",
            "(((1 4) ((0 1) (1 2)) ((0 1) (1 2))) \
              ((3 4) ((1 2) (1 1)) ((1 2) (1 1))))",
        ),
        // Euclid application keeps the pattern's values on the mask's
        // onsets.
        (
            "bd(3,8)",
            "((bd ((0 1) (1 8)) ((0 1) (1 8))) \
              (bd ((3 8) (1 2)) ((3 8) (1 2))) \
              (bd ((3 4) (7 8)) ((3 4) (7 8))))",
        ),
        (
            "bd(3,8,1)",
            "((bd ((1 4) (3 8)) ((1 4) (3 8))) \
              (bd ((5 8) (3 4)) ((5 8) (3 4))) \
              (bd ((7 8) (1 1)) ((7 8) (1 1))))",
        ),
    ];
    for (notation, expected) in rows {
        assert_pinned(
            &mut vm,
            &format!("{notation:?}"),
            expected,
            &format!(
                "(pin-events (pat/query {} (pat/span 0 1)))",
                emitted(notation),
            ),
        );
    }
}
