//! Tests of the readers that work at a cycle point. They cover the events
//! alive there, how far the point is through each of them, and the move of
//! a point by seconds.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};
use gantz_core::steel::steel_vm::engine::Engine;

/// Steel helpers that read a pattern at the playhead of a clock.
const PLAYHEAD: &str = r#"
;; The count of 1/60 s ticks from 0 s that have no event alive at the
;; playhead.
(define (blank-ticks p cps)
  (blank-loop p cps void 0.0 0 0))

(define (blank-loop p cps st t i blanks)
  (if (>= i 240)
      blanks
      (let ((r (pat/window st t (/ 1.0 60.0) cps)))
        (blank-loop p cps (list-ref r 4) (+ t (/ 1.0 60.0)) (+ i 1)
                    (if (empty? (pat/at p (pat/span-start (car r))))
                        (+ blanks 1)
                        blanks)))))
"#;

fn new_playhead_engine() -> Engine {
    let mut vm = new_pin_engine();
    vm.run(PLAYHEAD.to_string()).expect("playhead helpers");
    vm
}

#[test]
fn at() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // The event keeps its whole, and its active part is one grid step at
        // the point.
        (
            "keeps_the_whole",
            "(((7 1) ((1 4) (481 1920)) ((0 1) (1 1))))",
            "(pin-events (pat/at (pat/pure 7) 1/4))",
        ),
        (
            "a_float_point_snaps_to_the_grid",
            "(((7 1) ((1 4) (481 1920)) ((0 1) (1 1))))",
            "(pin-events (pat/at (pat/pure 7) 0.25))",
        ),
        ("a_rest_gives_nothing", "()", "(pat/at pat/silence 1/4)"),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
    let rows: &[(&str, &str)] = &[
        (
            "a_part_that_starts_inside_the_step_waits",
            "(= 1 (length (pat/at (pat/fast 3840 (pat/pure 1)) 1/4)))",
        ),
        (
            "a_stack_gives_each_layer",
            "(= 2 (length (pat/at (pat/stack (list (pat/pure 1) (pat/pure 2))) 1/4)))",
        ),
    ];
    for (case, expr) in rows {
        assert_steel_true(&mut vm, case, expr);
    }
}

#[test]
fn event_phase() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str)] = &[
        (
            "runs_over_the_whole",
            "(= 0.25 (pat/event-phase (car (pat/at (pat/pure 7) 1/4))))",
        ),
        (
            "starts_at_zero",
            "(= 0.0 (pat/event-phase (car (pat/at (pat/pure 7) 1))))",
        ),
        (
            "a_signal_has_no_phase",
            "(not (pat/event-phase (car (pat/at (pat/steady 1) 1/4))))",
        ),
    ];
    for (case, expr) in rows {
        assert_steel_true(&mut vm, case, expr);
    }
}

/// The tick at a cps drop overruns, so the next tick has an empty span. An
/// event stays alive through it, and the playhead still finds that event.
#[test]
fn alive_through_a_cps_drop() {
    let mut vm = new_playhead_engine();
    assert_steel_true(
        &mut vm,
        "no_tick_goes_blank",
        "(= 0 (blank-ticks (pat/pure 1) (pat/fastcat (list (pat/pure 1) (pat/pure 1/2)))))",
    );
}

/// Seconds cover the cycles that the cps gives them, on each side of a change.
#[test]
fn nudge() {
    let mut vm = new_pin_engine();
    let drop = "(pat/fastcat (list (pat/pure 1) (pat/pure 1/2)))";
    let gap = "(pat/fastcat (list (pat/pure 1) pat/silence))";
    let rows: &[(&str, &str, String)] = &[
        ("steady", "(5 8)", "(pat/nudge 1/2 1/4 1/2 1/2)".to_string()),
        ("zero", "(1 2)", "(pat/nudge 1/2 0.0 1/2 1/2)".to_string()),
        ("back", "(3 8)", "(pat/nudge 1/2 -1/4 1/2 1/2)".to_string()),
        (
            "across_a_cycle",
            "(9 8)",
            "(pat/nudge 7/8 1/2 1/2 1/2)".to_string(),
        ),
        (
            "across_a_drop",
            "(5 8)",
            format!("(pat/nudge 1/4 1/2 {drop} 1)"),
        ),
        (
            "back_across_a_drop",
            "(1 4)",
            format!("(pat/nudge 5/8 -1/2 {drop} 1)"),
        ),
        (
            "a_gap_holds_the_cps",
            "(3 4)",
            format!("(pat/nudge 1/2 1/2 {gap} 1/2)"),
        ),
        (
            "a_float_point_snaps_to_the_grid",
            "(5 8)",
            "(pat/nudge 0.5 1/4 1/2 1/2)".to_string(),
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, &format!("(pin-num {expr})"));
    }
    assert_steel_true(
        &mut vm,
        "a_reader_sees_the_later_phase",
        "(= 0.375 (pat/event-phase (car (pat/at (pat/pure 7) (pat/nudge 1/4 0.25 1/2 1/2)))))",
    );
}
