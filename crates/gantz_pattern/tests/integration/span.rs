//! Span-algebra and event-representation tests for the `gantz/pattern`
//! Steel module.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};

// `Span::cycles` splits at whole-cycle boundaries.
#[test]
fn span_cycles_whole() {
    let mut vm = new_pin_engine();
    assert_pinned(
        &mut vm,
        "(((0 1) (1 1)) ((1 1) (2 1)) ((2 1) (3 1)))",
        "(pin-spans (pat/span-cycles (pat/span 0 3)))",
    );
}

// Partial leading and trailing cycles keep their fractional bounds.
#[test]
fn span_cycles_partial() {
    let mut vm = new_pin_engine();
    assert_pinned(
        &mut vm,
        "(((1 4) (1 1)) ((1 1) (2 1)) ((2 1) (3 1)) ((3 1) (7 2)))",
        "(pin-spans (pat/span-cycles (pat/span 1/4 7/2)))",
    );
}

// Empty and negative spans yield no cycles.
#[test]
fn span_cycles_empty_and_negative() {
    let mut vm = new_pin_engine();
    assert_pinned(
        &mut vm,
        "()",
        "(pin-spans (pat/span-cycles (pat/span 1/2 1/2)))",
    );
    assert_pinned(
        &mut vm,
        "()",
        "(pin-spans (pat/span-cycles (pat/span 3/2 1/2)))",
    );
}

// Intersections clip to the overlap and reject disjoint spans.
#[test]
fn span_intersect() {
    let mut vm = new_pin_engine();
    assert_pinned(
        &mut vm,
        "((1 4) (3 4))",
        "(pin-span (pat/span-intersect (pat/span 0 3/4) (pat/span 1/4 1)))",
    );
    // Disjoint spans do not intersect.
    assert_pinned(
        &mut vm,
        "#f",
        "(pin-span (pat/span-intersect (pat/span 0 1/4) (pat/span 3/4 1)))",
    );
    // A zero-length span intersects nothing.
    assert_pinned(
        &mut vm,
        "#f",
        "(pin-span (pat/span-intersect (pat/span 1/2 1/2) (pat/span 0 1)))",
    );
}

#[test]
fn span_len_and_map() {
    let mut vm = new_pin_engine();
    assert_pinned(
        &mut vm,
        "(3 4)",
        "(pin-num (pat/span-len (pat/span 1/4 1)))",
    );
    assert_pinned(
        &mut vm,
        "((1 2) (1 1))",
        "(pin-span (pat/span-map (lambda (x) (* x 2)) (pat/span 1/4 1/2)))",
    );
}

// Event construction, accessors, and span/value mapping.
#[test]
fn event_representation() {
    let mut vm = new_pin_engine();
    assert_pinned(
        &mut vm,
        "(bd ((0 1) (1 2)) #f)",
        "(pin-event (pat/event 'bd (pat/span 0 1/2) #f))",
    );
    assert_pinned(
        &mut vm,
        "(bd ((0 1) (1 2)) ((0 1) (1 1)))",
        "(pin-event (pat/event 'bd (pat/span 0 1/2) (pat/span 0 1)))",
    );
    // whole-or-active falls back to active when whole is #f.
    assert_pinned(
        &mut vm,
        "((1 4) (1 2))",
        "(pin-span (pat/event-whole-or-active (pat/event 1 (pat/span 1/4 1/2) #f)))",
    );
    assert_pinned(
        &mut vm,
        "((0 1) (1 1))",
        "(pin-span (pat/event-whole-or-active \
           (pat/event 1 (pat/span 1/4 1/2) (pat/span 0 1))))",
    );
    // map-value leaves spans untouched.
    assert_pinned(
        &mut vm,
        "((11 1) ((0 1) (1 2)) #f)",
        "(pin-event (pat/event-map-value (lambda (v) (+ v 10)) \
           (pat/event 1 (pat/span 0 1/2) #f)))",
    );
    // map-spans maps active and whole, passing a #f whole through.
    assert_pinned(
        &mut vm,
        "((1 1) ((0 1) (1 4)) ((0 1) (1 2)))",
        "(pin-event (pat/event-map-spans (lambda (s) (pat/span-map (lambda (x) (/ x 2)) s)) \
           (pat/event 1 (pat/span 0 1/2) (pat/span 0 1))))",
    );
    assert_pinned(
        &mut vm,
        "((1 1) ((0 1) (1 4)) #f)",
        "(pin-event (pat/event-map-spans (lambda (s) (pat/span-map (lambda (x) (/ x 2)) s)) \
           (pat/event 1 (pat/span 0 1/2) #f)))",
    );
}

// Steel 0.8.2's `equal?` is broken for rationals nested in containers.
// Its recursive equality visitor lacks a Rational arm. The pin-*
// projection helpers exist because of this. If a steel upgrade fixes it,
// this canary flags that the projections can go.
#[test]
fn nested_rational_equal_canary() {
    let mut vm = new_pin_engine();
    assert_steel_true(&mut vm, "(equal? #f (equal? (list 1/2) (list 1/2)))");
}

// Events print deterministically as a transparent struct, stable across
// repeated renders.
#[test]
fn event_prints_deterministically() {
    let mut vm = new_pin_engine();
    for _ in 0..2 {
        assert_steel_true(
            &mut vm,
            "(equal? \"(event bd (0 . 1/2) #false)\"
                     (to-string (pat/event 'bd (pat/span 0 1/2) #f)))",
        );
        assert_steel_true(
            &mut vm,
            "(equal? \"(event 220.0 (1/4 . 1/2) (1/4 . 1/2))\"
                     (to-string (pat/event 220.0 (pat/span 1/4 1/2) (pat/span 1/4 1/2))))",
        );
    }
}
