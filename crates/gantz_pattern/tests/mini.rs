//! Mini-notation tests, comparing parsed patterns against their
//! combinator-built equivalents.

mod common;

use common::{assert_pinned, assert_steel_true};

/// Assert the notation and the combinator expression query identically
/// over the span.
fn assert_m_eq(notation: &str, combinators: &str, span: &str) {
    assert_steel_true(&format!(
        "(equal? (pin-events (pat/query (pat/m {notation:?}) (pat/span {span})))
                 (pin-events (pat/query {combinators} (pat/span {span}))))",
    ));
}

#[test]
fn single_atom() {
    assert_m_eq("bd", "(pat/pure 'bd)", "0 3");
}

#[test]
fn sequence() {
    assert_m_eq(
        "bd sn cp td",
        "(pat/fastcat (list (pat/pure 'bd) (pat/pure 'sn) (pat/pure 'cp) (pat/pure 'td)))",
        "0 1",
    );
}

#[test]
fn numeric_atoms() {
    assert_m_eq(
        "0 1 2 3 4",
        "(pat/fastcat (list (pat/pure 0) (pat/pure 1) (pat/pure 2) (pat/pure 3) (pat/pure 4)))",
        "0 1",
    );
}

// Exact rational atoms survive tokenization (the / between digits).
#[test]
fn rational_atoms() {
    assert_pinned(
        "(((1 4) ((0 1) (1 2)) ((0 1) (1 2))) \
          ((3 4) ((1 2) (1 1)) ((1 2) (1 1))))",
        "(pin-events (pat/query (pat/m \"1/4 3/4\") (pat/span 0 1)))",
    );
}

#[test]
fn nested_groups() {
    assert_m_eq(
        "0 [1 2] 3 [4 5]",
        "(pat/fastcat (list (pat/pure 0)
                            (pat/fastcat (list (pat/pure 1) (pat/pure 2)))
                            (pat/pure 3)
                            (pat/fastcat (list (pat/pure 4) (pat/pure 5)))))",
        "0 1",
    );
}

// Redundant nesting collapses to the same events.
#[test]
fn redundant_nesting() {
    assert_steel_true(
        "(equal? (pin-events (pat/query (pat/m \"[[[bd sn]]]\") (pat/span 0 1)))
                 (pin-events (pat/query (pat/m \"bd sn\") (pat/span 0 1))))",
    );
}

#[test]
fn rests() {
    assert_m_eq(
        "bd ~ sn ~",
        "(pat/fastcat (list (pat/pure 'bd) pat/silence (pat/pure 'sn) pat/silence))",
        "0 1",
    );
}

// `_` extends the previous step, assembling via timecat weights.
#[test]
fn elongation_by_continuation() {
    assert_m_eq(
        "bd _ _ _ sn _",
        "(pat/timecat (list (list 4 (pat/pure 'bd)) (list 2 (pat/pure 'sn))))",
        "0 1",
    );
}

// `@n` weights a step, equivalent to `_` continuation.
#[test]
fn elongation_by_weight() {
    assert_steel_true(
        "(equal? (pin-events (pat/query (pat/m \"a@3 b\") (pat/span 0 1)))
                 (pin-events (pat/query (pat/m \"a _ _ b\") (pat/span 0 1))))",
    );
}

#[test]
fn alternation() {
    assert_m_eq(
        "a b <c d>",
        "(pat/fastcat (list (pat/pure 'a)
                            (pat/pure 'b)
                            (pat/slowcat (list (pat/pure 'c) (pat/pure 'd)))))",
        "0 4",
    );
}

#[test]
fn stacks() {
    assert_m_eq(
        "[bd bd, sn sn sn]",
        "(pat/stack (list (pat/fastcat (list (pat/pure 'bd) (pat/pure 'bd)))
                          (pat/fastcat (list (pat/pure 'sn) (pat/pure 'sn) (pat/pure 'sn)))))",
        "0 1",
    );
}

#[test]
fn fast_and_slow_modifiers() {
    assert_m_eq(
        "bd*2 sn",
        "(pat/fastcat (list (pat/fast 2 (pat/pure 'bd)) (pat/pure 'sn)))",
        "0 1",
    );
    assert_m_eq("[a b]/2", "(pat/slow 2 (pat/m \"a b\"))", "0 2");
    // Modifiers compose with alternation.
    assert_m_eq(
        "<a b>*2",
        "(pat/fast 2 (pat/slowcat (list (pat/pure 'a) (pat/pure 'b))))",
        "0 2",
    );
}

// Euclid application keeps the pattern's values on the mask's onsets.
#[test]
fn euclid_application() {
    assert_pinned(
        "((bd ((0 1) (1 8)) ((0 1) (1 8))) \
          (bd ((3 8) (1 2)) ((3 8) (1 2))) \
          (bd ((3 4) (7 8)) ((3 4) (7 8))))",
        "(pin-events (pat/query (pat/m \"bd(3,8)\") (pat/span 0 1)))",
    );
    assert_pinned(
        "((bd ((1 4) (3 8)) ((1 4) (3 8))) \
          (bd ((5 8) (3 4)) ((5 8) (3 4))) \
          (bd ((7 8) (1 1)) ((7 8) (1 1))))",
        "(pin-events (pat/query (pat/m \"bd(3,8,1)\") (pat/span 0 1)))",
    );
}

// Malformed, empty and non-string input all parse to silence.
#[test]
fn malformed_input_is_silent() {
    for src in [
        "\"bd [\"", "\"*2\"", "\"<a\"", "\"bd(3\"", "\")\"", "\"\"", "\"   \"", "\"_ bd\"",
        "\"<>\"", "7", "'sym",
    ] {
        assert_pinned(
            "()",
            &format!("(pin-events (pat/query (pat/m {src}) (pat/span 0 2)))"),
        );
    }
}
