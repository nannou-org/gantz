//! Rate, concatenation, shift and fit-span tests.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};

#[test]
fn rate_and_concatenation_events() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // fast 2 doubles events per cycle and slow 4 of that nets a
        // half-speed pattern.
        (
            "fast",
            "(((1 1) ((0 1) (1 2)) ((0 1) (1 2))) \
              ((1 1) ((1 2) (1 1)) ((1 2) (1 1))))",
            "(pin-events (pat/query (pat/fast 2 (pat/pure 1)) (pat/span 0 1)))",
        ),
        (
            "slow_of_fast",
            "(((1 1) ((0 1) (2 1)) ((0 1) (2 1))))",
            "(pin-events (pat/query (pat/slow 4 (pat/fast 2 (pat/pure 1))) (pat/span 0 2)))",
        ),
        // A zero rate yields no events rather than dividing by zero.
        (
            "fast_zero_is_silent",
            "()",
            "(pin-events (pat/query (pat/fast 0 (pat/pure 1)) (pat/span 0 1)))",
        ),
        (
            "slow_zero_is_silent",
            "()",
            "(pin-events (pat/query (pat/slow 0 (pat/pure 1)) (pat/span 0 1)))",
        ),
        // One pattern per cycle, wrapping, with the final partial cycle
        // keeping its full-cycle whole.
        (
            "slowcat",
            "((a ((0 1) (1 1)) ((0 1) (1 1))) \
              (b ((1 1) (2 1)) ((1 1) (2 1))) \
              (a ((2 1) (5 2)) ((2 1) (3 1))))",
            "(pin-events (pat/query (pat/slowcat (list (pat/pure 'a) (pat/pure 'b))) \
               (pat/span 0 5/2)))",
        ),
        // Both patterns fit within one cycle.
        (
            "fastcat",
            "((a ((0 1) (1 2)) ((0 1) (1 2))) \
              (b ((1 2) (1 1)) ((1 2) (1 1))) \
              (a ((1 1) (5 4)) ((1 1) (3 2))))",
            "(pin-events (pat/query (pat/fastcat (list (pat/pure 'a) (pat/pure 'b))) \
               (pat/span 0 5/4)))",
        ),
        // Weighted sub-spans, where every event's whole becomes its
        // pattern's sub-span.
        (
            "timecat",
            "((a ((1 4) (1 3)) ((0 1) (1 3))) \
              (b ((1 3) (1 1)) ((1 3) (1 1))) \
              (a ((1 1) (4 3)) ((1 1) (4 3))) \
              (b ((4 3) (3 2)) ((4 3) (2 1))))",
            "(pin-events (pat/query (pat/timecat (list (list 1 (pat/pure 'a)) \
                                                       (list 2 (pat/pure 'b)))) \
               (pat/span 1/4 3/2)))",
        ),
        // A unit-cycle pattern squeezed into [1/2, 3/4).
        (
            "fit_span",
            "((a ((1 2) (3 4)) ((1 2) (3 4))))",
            "(pin-events (pat/query (pat/fit-span (pat/span 0 1) (pat/span 1/2 3/4) (pat/pure 'a)) \
               (pat/span 1/2 3/4)))",
        ),
        // stack layers patterns, query order stable at equal starts.
        (
            "stack_layers",
            "((a ((0 1) (1 1)) ((0 1) (1 1))) \
              (b ((0 1) (1 2)) ((0 1) (1 2))) \
              (b ((1 2) (1 1)) ((1 2) (1 1))))",
            "(pin-events (pat/query (pat/stack (list (pat/pure 'a) (pat/fast 2 (pat/pure 'b)))) \
               (pat/span 0 1)))",
        ),
        // rationalize snaps floats to the 1/1920 grid and passes exacts
        // through.
        (
            "rationalize_half",
            "(1 2)",
            "(pin-num (pat/rationalize 0.5))",
        ),
        (
            "rationalize_three_halves",
            "(3 2)",
            "(pin-num (pat/rationalize 1.5))",
        ),
        (
            "rationalize_exact_third",
            "(1 3)",
            "(pin-num (pat/rationalize 1/3))",
        ),
        ("rationalize_int", "(2 1)", "(pin-num (pat/rationalize 2))"),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

#[test]
fn shift_and_fit_equivalences() {
    let mut vm = new_pin_engine();
    // Shift equivalences over a single cycle on the pattern `bd ~ bd ~`.
    let pat_a = "(pat/fastcat (list (pat/pure 'bd) pat/silence (pat/pure 'bd) pat/silence))";
    let pat_b = "(pat/fastcat (list pat/silence (pat/pure 'bd) pat/silence (pat/pure 'bd)))";
    let events = |p: String| format!("(pin-events (pat/query {p} (pat/span 0 1)))");
    let eq = |l: String, r: String| format!("(equal? {l} {r})");
    let shift = |amt: &str, p: &str| format!("(pat/shift {amt} {p})");
    let rows = [
        (
            "shift 1/4 a = b",
            eq(events(shift("1/4", pat_a)), events(pat_b.to_string())),
        ),
        (
            "shift 5/4 a = b",
            eq(events(shift("5/4", pat_a)), events(pat_b.to_string())),
        ),
        (
            "a = shift -1/4 b",
            eq(events(pat_a.to_string()), events(shift("-1/4", pat_b))),
        ),
        (
            "a = shift -3/4 b",
            eq(events(pat_a.to_string()), events(shift("-3/4", pat_b))),
        ),
        (
            "shift 1/8 a = shift -1/8 b",
            eq(events(shift("1/8", pat_a)), events(shift("-1/8", pat_b))),
        ),
        // The inequality too. An eighth off is not aligned.
        (
            "shift 1/8 a != b",
            format!(
                "(equal? #f {})",
                eq(events(shift("1/8", pat_a)), events(pat_b.to_string())),
            ),
        ),
        // fit-cycle is the unit-src shorthand for fit-span.
        (
            "fit_cycle",
            "(equal? (pin-events (pat/query (pat/fit-span (pat/span 0 1) (pat/span 1/2 3/4) (pat/pure 'a)) (pat/span 0 4)))
                     (pin-events (pat/query (pat/fit-cycle (pat/span 1/2 3/4) (pat/pure 'a)) (pat/span 0 4))))"
                .to_string(),
        ),
    ];
    for (case, snippet) in &rows {
        assert_steel_true(&mut vm, case, snippet);
    }
}
