//! Rate, concatenation, shift and fit-span tests, ported from the cycles
//! crate (`test_rate`, `test_slowcat`, `test_fastcat`, `test_timecat`,
//! `test_shift`, `test_fit_span`).

mod common;

use common::{assert_pinned, assert_steel_true};

// Ported `test_rate`: fast 2 doubles events per cycle, slow 4 of that
// nets a half-speed pattern.
#[test]
fn fast_and_slow() {
    assert_pinned(
        "(((1 1) ((0 1) (1 2)) ((0 1) (1 2))) \
          ((1 1) ((1 2) (1 1)) ((1 2) (1 1))))",
        "(pin-events (pat/query (pat/fast 2 (pat/pure 1)) (pat/span 0 1)))",
    );
    assert_pinned(
        "(((1 1) ((0 1) (2 1)) ((0 1) (2 1))))",
        "(pin-events (pat/query (pat/slow 4 (pat/fast 2 (pat/pure 1))) (pat/span 0 2)))",
    );
}

// A zero rate yields no events rather than dividing by zero.
#[test]
fn rate_zero_is_silent() {
    assert_pinned(
        "()",
        "(pin-events (pat/query (pat/fast 0 (pat/pure 1)) (pat/span 0 1)))",
    );
    assert_pinned(
        "()",
        "(pin-events (pat/query (pat/slow 0 (pat/pure 1)) (pat/span 0 1)))",
    );
}

// Ported `test_slowcat`: one pattern per cycle, wrapping, with the final
// partial cycle keeping its full-cycle whole.
#[test]
fn slowcat() {
    assert_pinned(
        "((a ((0 1) (1 1)) ((0 1) (1 1))) \
          (b ((1 1) (2 1)) ((1 1) (2 1))) \
          (a ((2 1) (5 2)) ((2 1) (3 1))))",
        "(pin-events (pat/query (pat/slowcat (list (pat/pure 'a) (pat/pure 'b))) \
           (pat/span 0 5/2)))",
    );
}

// Ported `test_fastcat`: both patterns fit one cycle.
#[test]
fn fastcat() {
    assert_pinned(
        "((a ((0 1) (1 2)) ((0 1) (1 2))) \
          (b ((1 2) (1 1)) ((1 2) (1 1))) \
          (a ((1 1) (5 4)) ((1 1) (3 2))))",
        "(pin-events (pat/query (pat/fastcat (list (pat/pure 'a) (pat/pure 'b))) \
           (pat/span 0 5/4)))",
    );
}

// Ported `test_timecat`: weighted sub-spans, every event's whole becomes
// its pattern's sub-span.
#[test]
fn timecat() {
    assert_pinned(
        "((a ((1 4) (1 3)) ((0 1) (1 3))) \
          (b ((1 3) (1 1)) ((1 3) (1 1))) \
          (a ((1 1) (4 3)) ((1 1) (4 3))) \
          (b ((4 3) (3 2)) ((4 3) (2 1))))",
        "(pin-events (pat/query (pat/timecat (list (list 1 (pat/pure 'a)) \
                                                   (list 2 (pat/pure 'b)))) \
           (pat/span 1/4 3/2)))",
    );
}

// Ported `test_shift`: the five equivalences and one inequality over a
// single cycle, on the pattern `bd ~ bd ~`.
#[test]
fn shift_equivalences() {
    let pat_a = "(pat/fastcat (list (pat/pure 'bd) pat/silence (pat/pure 'bd) pat/silence))";
    let pat_b = "(pat/fastcat (list pat/silence (pat/pure 'bd) pat/silence (pat/pure 'bd)))";
    let events = |p: String| format!("(pin-events (pat/query {p} (pat/span 0 1)))");
    let eq = |l: String, r: String| format!("(equal? {l} {r})");
    let shift = |amt: &str, p: &str| format!("(pat/shift {amt} {p})");

    assert_steel_true(&eq(events(shift("1/4", pat_a)), events(pat_b.to_string())));
    assert_steel_true(&eq(events(shift("5/4", pat_a)), events(pat_b.to_string())));
    assert_steel_true(&eq(events(pat_a.to_string()), events(shift("-1/4", pat_b))));
    assert_steel_true(&eq(events(pat_a.to_string()), events(shift("-3/4", pat_b))));
    assert_steel_true(&eq(
        events(shift("1/8", pat_a)),
        events(shift("-1/8", pat_b)),
    ));
    // And the inequality: an eighth off is not aligned.
    assert_steel_true(&format!(
        "(equal? #f {})",
        eq(events(shift("1/8", pat_a)), events(pat_b.to_string())),
    ));
}

// Ported `test_fit_span`: a unit-cycle pattern squeezed into [1/2, 3/4),
// and fit-cycle as the unit-src shorthand.
#[test]
fn fit_span_and_fit_cycle() {
    assert_pinned(
        "((a ((1 2) (3 4)) ((1 2) (3 4))))",
        "(pin-events (pat/query (pat/fit-span (pat/span 0 1) (pat/span 1/2 3/4) (pat/pure 'a)) \
           (pat/span 1/2 3/4)))",
    );
    assert_steel_true(
        "(equal? (pin-events (pat/query (pat/fit-span (pat/span 0 1) (pat/span 1/2 3/4) (pat/pure 'a)) (pat/span 0 4)))
                 (pin-events (pat/query (pat/fit-cycle (pat/span 1/2 3/4) (pat/pure 'a)) (pat/span 0 4))))",
    );
}

// stack layers patterns, query order stable at equal starts.
#[test]
fn stack_layers() {
    assert_pinned(
        "((a ((0 1) (1 1)) ((0 1) (1 1))) \
          (b ((0 1) (1 2)) ((0 1) (1 2))) \
          (b ((1 2) (1 1)) ((1 2) (1 1))))",
        "(pin-events (pat/query (pat/stack (list (pat/pure 'a) (pat/fast 2 (pat/pure 'b)))) \
           (pat/span 0 1)))",
    );
}

// rationalize snaps floats to the 1/1920 grid and passes exacts through.
#[test]
fn rationalize() {
    assert_pinned("(1 2)", "(pin-num (pat/rationalize 0.5))");
    assert_pinned("(3 2)", "(pin-num (pat/rationalize 1.5))");
    assert_pinned("(1 3)", "(pin-num (pat/rationalize 1/3))");
    assert_pinned("(2 1)", "(pin-num (pat/rationalize 2))");
}
