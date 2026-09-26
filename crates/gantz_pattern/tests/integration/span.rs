//! Span-algebra and event-representation tests for the `gantz/pattern`
//! Steel module.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};

#[test]
fn span_algebra() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // `Span::cycles` splits at whole-cycle boundaries.
        (
            "span_cycles_whole",
            "(((0 1) (1 1)) ((1 1) (2 1)) ((2 1) (3 1)))",
            "(pin-spans (pat/span-cycles (pat/span 0 3)))",
        ),
        // Partial leading and trailing cycles keep their fractional bounds.
        (
            "span_cycles_partial",
            "(((1 4) (1 1)) ((1 1) (2 1)) ((2 1) (3 1)) ((3 1) (7 2)))",
            "(pin-spans (pat/span-cycles (pat/span 1/4 7/2)))",
        ),
        // Empty and negative spans yield no cycles.
        (
            "span_cycles_empty",
            "()",
            "(pin-spans (pat/span-cycles (pat/span 1/2 1/2)))",
        ),
        (
            "span_cycles_negative",
            "()",
            "(pin-spans (pat/span-cycles (pat/span 3/2 1/2)))",
        ),
        // Intersections clip to the overlap and reject disjoint spans.
        (
            "span_intersect_overlap",
            "((1 4) (3 4))",
            "(pin-span (pat/span-intersect (pat/span 0 3/4) (pat/span 1/4 1)))",
        ),
        (
            "span_intersect_disjoint",
            "#f",
            "(pin-span (pat/span-intersect (pat/span 0 1/4) (pat/span 3/4 1)))",
        ),
        // A zero-length span intersects nothing.
        (
            "span_intersect_zero_length",
            "#f",
            "(pin-span (pat/span-intersect (pat/span 1/2 1/2) (pat/span 0 1)))",
        ),
        (
            "span_len",
            "(3 4)",
            "(pin-num (pat/span-len (pat/span 1/4 1)))",
        ),
        (
            "span_map",
            "((1 2) (1 1))",
            "(pin-span (pat/span-map (lambda (x) (* x 2)) (pat/span 1/4 1/2)))",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

// Event construction, accessors, span/value mapping and printing.
#[test]
fn event_representation() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        (
            "event_without_whole",
            "(bd ((0 1) (1 2)) #f)",
            "(pin-event (pat/event 'bd (pat/span 0 1/2) #f))",
        ),
        (
            "event_with_whole",
            "(bd ((0 1) (1 2)) ((0 1) (1 1)))",
            "(pin-event (pat/event 'bd (pat/span 0 1/2) (pat/span 0 1)))",
        ),
        // whole-or-active falls back to active when whole is #f.
        (
            "whole_or_active_without_whole",
            "((1 4) (1 2))",
            "(pin-span (pat/event-whole-or-active (pat/event 1 (pat/span 1/4 1/2) #f)))",
        ),
        (
            "whole_or_active_with_whole",
            "((0 1) (1 1))",
            "(pin-span (pat/event-whole-or-active \
               (pat/event 1 (pat/span 1/4 1/2) (pat/span 0 1))))",
        ),
        // map-value leaves spans untouched.
        (
            "map_value",
            "((11 1) ((0 1) (1 2)) #f)",
            "(pin-event (pat/event-map-value (lambda (v) (+ v 10)) \
               (pat/event 1 (pat/span 0 1/2) #f)))",
        ),
        // map-spans maps active and whole, passing a #f whole through.
        (
            "map_spans_with_whole",
            "((1 1) ((0 1) (1 4)) ((0 1) (1 2)))",
            "(pin-event (pat/event-map-spans (lambda (s) (pat/span-map (lambda (x) (/ x 2)) s)) \
               (pat/event 1 (pat/span 0 1/2) (pat/span 0 1))))",
        ),
        (
            "map_spans_without_whole",
            "((1 1) ((0 1) (1 4)) #f)",
            "(pin-event (pat/event-map-spans (lambda (s) (pat/span-map (lambda (x) (/ x 2)) s)) \
               (pat/event 1 (pat/span 0 1/2) #f)))",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }

    // Events print deterministically as a transparent struct, stable across
    // repeated renders.
    let prints: &[(&str, &str)] = &[
        (
            "print_symbol_without_whole",
            "(equal? \"(event bd (0 . 1/2) #false)\"
                     (to-string (pat/event 'bd (pat/span 0 1/2) #f)))",
        ),
        (
            "print_float_with_whole",
            "(equal? \"(event 220.0 (1/4 . 1/2) (1/4 . 1/2))\"
                     (to-string (pat/event 220.0 (pat/span 1/4 1/2) (pat/span 1/4 1/2))))",
        ),
    ];
    for render in 0..2 {
        for (case, snippet) in prints {
            assert_steel_true(&mut vm, &format!("{case}, render {render}"), snippet);
        }
    }
}

// Steel 0.8.2's `equal?` is broken for rationals nested in containers.
// Its recursive equality visitor lacks a Rational arm. The pin-*
// projection helpers exist because of this. If a steel upgrade fixes it,
// this canary flags that the projections can go.
#[test]
fn nested_rational_equal_canary() {
    let mut vm = new_pin_engine();
    assert_steel_true(
        &mut vm,
        "nested_rational_equal_canary",
        "(equal? #f (equal? (list 1/2) (list 1/2)))",
    );
}
