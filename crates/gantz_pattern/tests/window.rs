//! Windower and delivery-helper tests.

mod common;

use common::{assert_pinned, assert_steel_true};

// The first tick, with the Void state of a fresh expr node, yields an
// empty span anchored at the current position.
#[test]
fn first_tick_empty_span() {
    assert_pinned(
        "(((1 2) (1 2)) (1 2))",
        "(let ((r (pat/window void 0.5 1)))
           (list (pin-span (car r)) (pin-num (car (cdr r)))))",
    );
}

// Successive ticks produce abutting spans. Each span's start is exactly
// the previous span's end.
#[test]
fn spans_abut_exactly() {
    assert_pinned(
        "(((1 2) (1 1)) ((1 1) (3 2)))",
        "(let ((r1 (pat/window void 0.5 1)))
           (let ((r2 (pat/window (car (cdr r1)) 1.0 1)))
             (let ((r3 (pat/window (car (cdr r2)) 1.5 1)))
               (list (pin-span (car r2)) (pin-span (car r3))))))",
    );
}

// A tick that does not advance the position yields an empty span, as
// does a backwards position jump from a cps drop, continuing from the
// new position.
#[test]
fn stalls_and_jumps_yield_empty_spans() {
    assert_pinned(
        "(((1 2) (1 2)) (1 2))",
        "(let ((r (pat/window 1/2 0.5 1)))
           (list (pin-span (car r)) (pin-num (car (cdr r)))))",
    );
    assert_pinned(
        "(((1 2) (1 2)) (1 2))",
        "(let ((r (pat/window 1 1.0 0.5)))
           (list (pin-span (car r)) (pin-num (car (cdr r)))))",
    );
}

// Positions snap to the 1/1920 grid, so the float closest to 1/3 lands
// on exactly 1/3 and denominators stay bounded.
#[test]
fn grid_snaps_thirds_exactly() {
    assert_pinned(
        "(1 3)",
        "(pin-num (car (cdr (pat/window void 0.3333333333333333 1))))",
    );
}

// events->secs anchors the span start at the eval time, spaces events by
// their exact cycle offsets over cps, keeps only onsets, and emits
// floats only.
#[test]
fn events_to_secs() {
    assert_steel_true(
        "(equal? (list (list 10.0 #t) (list 10.1875 #t) (list 10.375 #t))
                 (pat/events->secs (pat/query (pat/euclid 3 8) (pat/span 0 1))
                                   (pat/span 0 1) 10.0 2.0))",
    );
    // Numeric values leave as floats.
    assert_steel_true(
        "(equal? (list (list 5.0 0.25))
                 (pat/events->secs (pat/query (pat/pure 1/4) (pat/span 0 1))
                                   (pat/span 0 1) 5.0 1.0))",
    );
}

// Window-chopped continuations and signal events are filtered out of
// delivery.
#[test]
fn only_onsets_delivered() {
    assert_steel_true("(pat/event-onset? (pat/event 'x (pat/span 0 1/2) (pat/span 0 1)))");
    assert_steel_true(
        "(equal? #f (pat/event-onset? (pat/event 'x (pat/span 1/2 1) (pat/span 0 1))))",
    );
    assert_steel_true("(equal? #f (pat/event-onset? (pat/event 'x (pat/span 0 1/2) #f)))");
    assert_pinned(
        "()",
        "(pat/events->secs (list (pat/event 'x (pat/span 1/2 1) (pat/span 0 1))
                                 (pat/event 'y (pat/span 0 1/2) #f))
                           (pat/span 0 1) 0.0 1.0)",
    );
}
