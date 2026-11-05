//! Tests of the `gantz/sched` module, which works on schedules of
//! `(seconds value)` pairs.

use crate::common::{assert_pinned, new_pin_engine};

#[test]
fn map() {
    let mut vm = new_pin_engine();
    assert_pinned(
        &mut vm,
        "maps_values_and_keeps_seconds",
        "((1.0 2) (1.5 3))",
        "(sched/map (lambda (v) (+ v 1)) (list (list 1.0 1) (list 1.5 2)))",
    );
}

#[test]
fn pick() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // Numbers leave as floats, for the control inputs.
        (
            "numbers_leave_as_floats",
            "((1.0 0.5) (2.0 1.5))",
            "(sched/pick (list (list 1.0 (hash 'n 1/2)) (list 2.0 (hash 'n 3/2))) 'n)",
        ),
        // A pair without the key drops, as does a value that is not a map.
        (
            "other_values_drop",
            "((2.0 3.0))",
            "(sched/pick (list (list 1.0 (hash 'm 1)) (list 1.5 7.0) (list 2.0 (hash 'n 3))) 'n)",
        ),
        // A value that is not a number passes through.
        (
            "symbols_pass",
            "((1.0 bd))",
            "(sched/pick (list (list 1.0 (hash 's 'bd))) 's)",
        ),
        ("junk_is_silent", "()", "(sched/pick '() 'n)"),
        ("junk_schedule_is_silent", "()", "(sched/pick 'junk 'n)"),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

#[test]
fn filter() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        (
            "keeps_matching_values",
            "((2.0 2))",
            "(sched/filter (lambda (v) (> v 1)) (list (list 1.0 1) (list 2.0 2)))",
        ),
        (
            "non_fn_is_silent",
            "()",
            "(sched/filter 'nope (list (list 1.0 1)))",
        ),
        (
            "junk_schedule_is_silent",
            "()",
            "(sched/filter (lambda (v) #t) 'junk)",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

#[test]
fn trig() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // Each pair opens a gate that closes `len` seconds later.
        (
            "pulses",
            "((1.0 1.0) (1.25 0.0) (2.0 1.0) (2.25 0.0))",
            "(sched/trig (list (list 2.0 'b) (list 1.0 'a)) 0.25)",
        ),
        // A close that a later open comes before is dropped.
        (
            "overlap_drops_the_earlier_close",
            "((1.0 1.0) (1.5 1.0) (2.25 0.0))",
            "(sched/trig (list (list 1.0 'a) (list 1.5 'b)) 3/4)",
        ),
        // A close and an open at the same time close first, so the open
        // starts a new gate.
        (
            "close_before_open",
            "((1.0 1.0) (1.5 0.0) (1.5 1.0) (2.0 0.0))",
            "(sched/trig (list (list 1.0 'a) (list 1.5 'b)) 0.5)",
        ),
        (
            "zero_len_is_silent",
            "()",
            "(sched/trig (list (list 1.0 'a)) 0)",
        ),
        (
            "junk_len_is_silent",
            "()",
            "(sched/trig (list (list 1.0 'a)) '())",
        ),
        ("junk_schedule_is_silent", "()", "(sched/trig 'junk 0.5)"),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

#[test]
fn gate() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // Each map with seconds under the key gives a gate, in time order.
        // Other pairs give none.
        (
            "gates_in_time_order",
            "((1.0 1.0) (1.5 0.0) (2.0 1.0) (4.0 0.0))",
            "(sched/gate (list (list 2.0 (hash 'dur 2))
                               (list 3.0 (hash 'x 1))
                               (list 3.5 7)
                               (list 1.0 (hash 'dur 1/2)))
                         'dur)",
        ),
        // An earlier event does not close the gate of a later one.
        (
            "overlap_drops_the_earlier_close",
            "((1.0 1.0) (1.5 1.0) (2.5 0.0))",
            "(sched/gate (list (list 1.0 (hash 'dur 1)) (list 1.5 (hash 'dur 1))) 'dur)",
        ),
        (
            "non_positive_seconds_give_no_gate",
            "()",
            "(sched/gate (list (list 1.0 (hash 'dur 0)) (list 2.0 (hash 'dur 'x))) 'dur)",
        ),
        ("junk_schedule_is_silent", "()", "(sched/gate 'junk 'dur)"),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

#[test]
fn split() {
    let mut vm = new_pin_engine();
    // The times of each schedule, as maps do not compare.
    let times = |expr: &str| format!("(pin-map (lambda (s) (pin-map car s)) {expr})");
    let rows: &[(&str, &str, String)] = &[
        // An integer key picks the schedule modulo the count. A float that
        // is an integer counts. Other pairs are in none.
        (
            "routes_by_the_key",
            "((1.0 2.0) (3.0) (6.0))",
            times(
                "(sched/split (list (list 1.0 (hash 'v 0))
                                    (list 2.0 (hash 'v 3))
                                    (list 3.0 (hash 'v 1.0))
                                    (list 4.0 (hash 'v 1/2))
                                    (list 5.0 (hash 'x 1))
                                    (list 6.0 (hash 'v -1)))
                              'v 3)",
            ),
        ),
        (
            "junk_schedule_gives_empty_schedules",
            "(() ())",
            times("(sched/split 'junk 'v 2)"),
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}
