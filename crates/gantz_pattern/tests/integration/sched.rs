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
