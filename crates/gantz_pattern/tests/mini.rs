//! Mini-notation tests: the Rust parser's emitted combinator source is
//! evaluated against hand-built combinator expressions on the pin
//! harness.

mod common;

use common::{assert_pinned, assert_steel_true};
use gantz_pattern::mini::steel_src;

/// Assert the notation's emission and the combinator expression query
/// identically over the span.
fn assert_m_eq(notation: &str, combinators: &str, span: &str) {
    let emitted = steel_src(notation).unwrap_or_else(|| panic!("{notation:?} failed to parse"));
    assert_steel_true(&format!(
        "(equal? (pin-events (pat/query {emitted} (pat/span {span})))
                 (pin-events (pat/query {combinators} (pat/span {span}))))",
    ));
}

fn emitted(notation: &str) -> String {
    steel_src(notation).unwrap_or_else(|| panic!("{notation:?} failed to parse"))
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
        &format!(
            "(pin-events (pat/query {} (pat/span 0 1)))",
            emitted("1/4 3/4"),
        ),
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
    assert_steel_true(&format!(
        "(equal? (pin-events (pat/query {} (pat/span 0 1)))
                 (pin-events (pat/query {} (pat/span 0 1))))",
        emitted("[[[bd sn]]]"),
        emitted("bd sn"),
    ));
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
    assert_steel_true(&format!(
        "(equal? (pin-events (pat/query {} (pat/span 0 1)))
                 (pin-events (pat/query {} (pat/span 0 1))))",
        emitted("a@3 b"),
        emitted("a _ _ b"),
    ));
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
    assert_m_eq(
        "[a b]/2",
        "(pat/slow 2 (pat/fastcat (list (pat/pure 'a) (pat/pure 'b))))",
        "0 2",
    );
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
        &format!(
            "(pin-events (pat/query {} (pat/span 0 1)))",
            emitted("bd(3,8)"),
        ),
    );
    assert_pinned(
        "((bd ((1 4) (3 8)) ((1 4) (3 8))) \
          (bd ((5 8) (3 4)) ((5 8) (3 4))) \
          (bd ((7 8) (1 1)) ((7 8) (1 1))))",
        &format!(
            "(pin-events (pat/query {} (pat/span 0 1)))",
            emitted("bd(3,8,1)"),
        ),
    );
}
