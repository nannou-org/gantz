//! Windower and delivery-helper tests.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};

#[test]
fn window_spans() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // The first tick, with the Void state of a fresh expr node, yields
        // an empty span anchored at the current position.
        (
            "first_tick_empty_span",
            "(((1 2) (1 2)) (1 2))",
            "(let ((r (pat/window void 0.5 1)))
               (list (pin-span (car r)) (pin-num (car (cdr r)))))",
        ),
        // Successive ticks produce abutting spans. Each span's start is
        // exactly the previous span's end.
        (
            "spans_abut_exactly",
            "(((1 2) (1 1)) ((1 1) (3 2)))",
            "(let ((r1 (pat/window void 0.5 1)))
               (let ((r2 (pat/window (car (cdr r1)) 1.0 1)))
                 (let ((r3 (pat/window (car (cdr r2)) 1.5 1)))
                   (list (pin-span (car r2)) (pin-span (car r3))))))",
        ),
        // A tick that does not advance the position yields an empty span,
        // as does a backwards position jump from a cps drop, continuing
        // from the new position.
        (
            "stall_yields_empty_span",
            "(((1 2) (1 2)) (1 2))",
            "(let ((r (pat/window 1/2 0.5 1)))
               (list (pin-span (car r)) (pin-num (car (cdr r)))))",
        ),
        (
            "backwards_jump_yields_empty_span",
            "(((1 2) (1 2)) (1 2))",
            "(let ((r (pat/window 1 1.0 0.5)))
               (list (pin-span (car r)) (pin-num (car (cdr r)))))",
        ),
        // A forward jump beyond the window cap resets rather than covering
        // the gap, so a cps raise cannot flood downstream with a giant
        // span. A multi-cycle advance below the cap passes through.
        (
            "forward_jump_beyond_cap_resets",
            "(((200 1) (200 1)) (200 1))",
            "(let ((r (pat/window 1 100.0 2)))
               (list (pin-span (car r)) (pin-num (car (cdr r)))))",
        ),
        (
            "forward_jump_below_cap_passes",
            "(((0 1) (4 1)) (4 1))",
            "(let ((r (pat/window 0 4.0 1)))
               (list (pin-span (car r)) (pin-num (car (cdr r)))))",
        ),
        // Positions snap to the 1/1920 grid, so the float closest to 1/3
        // lands on exactly 1/3 and denominators stay bounded.
        (
            "grid_snaps_thirds_exactly",
            "(1 3)",
            "(pin-num (car (cdr (pat/window void 0.3333333333333333 1))))",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

#[test]
fn delivery() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str)] = &[
        // events->secs anchors the span start at the eval time, spaces
        // events by their exact cycle offsets over cps, keeps only onsets,
        // and emits floats only.
        (
            "events_to_secs_spaces_onsets",
            "(equal? (list (list 10.0 #t) (list 10.1875 #t) (list 10.375 #t))
                     (pat/events->secs (pat/query (pat/euclid 3 8) (pat/span 0 1))
                                       (pat/span 0 1) 10.0 2.0))",
        ),
        // Numeric values leave as floats.
        (
            "events_to_secs_numbers_leave_as_floats",
            "(equal? (list (list 5.0 0.25))
                     (pat/events->secs (pat/query (pat/pure 1/4) (pat/span 0 1))
                                       (pat/span 0 1) 5.0 1.0))",
        ),
        // Window-chopped continuations and signal events are filtered out
        // of delivery.
        (
            "onset_is_delivered",
            "(pat/event-onset? (pat/event 'x (pat/span 0 1/2) (pat/span 0 1)))",
        ),
        (
            "continuation_is_not_an_onset",
            "(equal? #f (pat/event-onset? (pat/event 'x (pat/span 1/2 1) (pat/span 0 1))))",
        ),
        (
            "signal_event_is_not_an_onset",
            "(equal? #f (pat/event-onset? (pat/event 'x (pat/span 0 1/2) #f)))",
        ),
    ];
    for (case, snippet) in rows {
        assert_steel_true(&mut vm, case, snippet);
    }
    assert_pinned(
        &mut vm,
        "non_onsets_are_not_delivered",
        "()",
        "(pat/events->secs (list (pat/event 'x (pat/span 1/2 1) (pat/span 0 1))
                                 (pat/event 'y (pat/span 0 1/2) #f))
                           (pat/span 0 1) 0.0 1.0)",
    );
}
