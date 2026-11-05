//! Tests of `pat/secs`, the seconds between two cycle points, and
//! `pat/len-secs`, which adds the seconds of each event to its map.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};

/// A cps of 1 for the first half of each cycle, then 2.
const STEP: &str = "(pat/fastcat (list (pat/pure 1) (pat/pure 2)))";

#[test]
fn secs() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, String)] = &[
        ("steady", "0.5", "(pat/secs 0 1 2 2)".into()),
        ("backward_is_negative", "-0.5", "(pat/secs 1 0 2 2)".into()),
        // A cps change between the points counts.
        ("patterned", "0.75", format!("(pat/secs 0 1 {STEP} 1)")),
        ("across_cycles", "1.5", format!("(pat/secs 0 2 {STEP} 1)")),
        // A gap at the start holds `held`, and a later gap holds the cps
        // before it.
        (
            "gap_at_the_start_holds_held",
            "0.625",
            "(pat/secs 0 1 (pat/fastcat (list (pat/pure 'x) (pat/pure 1))) 4)".into(),
        ),
        (
            "later_gap_holds_the_cps_before",
            "1.0",
            "(pat/secs 0 1 (pat/fastcat (list (pat/pure 1) (pat/pure 'x))) 4)".into(),
        ),
        ("float_points", "0.25", "(pat/secs 0.5 1.0 2 2)".into()),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
    for (case, expr) in [
        ("junk_point", "(pat/secs '() 1 1 1)"),
        ("junk_cps", "(pat/secs 0 1 'x 1)"),
        ("junk_held", "(pat/secs 0 1 1 '())"),
        ("zero_held", "(pat/secs 0 1 1 0)"),
    ] {
        assert_pinned(&mut vm, case, "#f", expr);
    }
}

// `pat/secs` undoes `pat/nudge`, within the grid that the nudged point
// snaps to.
#[test]
fn secs_inverts_nudge() {
    let mut vm = new_pin_engine();
    for (x, s) in [
        ("1/3", "0.4"),
        ("1/3", "-0.4"),
        ("5/4", "1.3"),
        ("0", "-2.2"),
    ] {
        let expr =
            format!("(< (abs (- (pat/secs {x} (pat/nudge {x} {s} {STEP} 1) {STEP} 1) {s})) 0.001)");
        assert_steel_true(&mut vm, &format!("nudge {x} by {s}"), &expr);
    }
}

#[test]
fn len_secs() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, String)] = &[
        // Each half-cycle event plays for 1/4 s at 2 cps.
        (
            "steady",
            "((0.25 ((0 1) (1 2)) ((0 1) (1 2))) (0.25 ((1 2) (1 1)) ((1 2) (1 1))))",
            "(pin-events (pat/query
               (pat/get 'dur (pat/len-secs 'dur (pat/fast 2 (pat/pure (hash 'n 1))) 2 2))
               (pat/span 0 1)))"
                .into(),
        ),
        // A cps change inside an event counts.
        (
            "patterned",
            "((0.75 ((0 1) (1 1)) ((0 1) (1 1))))",
            format!(
                "(pin-events (pat/query
                   (pat/get 'dur (pat/len-secs 'dur (pat/pure (hash 'n 1)) {STEP} 1))
                   (pat/span 0 1)))"
            ),
        ),
        // The other keys of the map stay.
        (
            "keeps_keys",
            "(((1 1) ((0 1) (1 1)) ((0 1) (1 1))))",
            "(pin-events (pat/query
               (pat/get 'n (pat/len-secs 'dur (pat/pure (hash 'n 1)) 1 1))
               (pat/span 0 1)))"
                .into(),
        ),
        // A value that is not a map becomes a map of the one key.
        (
            "non_map_becomes_a_map",
            "((1.0 ((0 1) (1 1)) ((0 1) (1 1))))",
            "(pin-events (pat/query
               (pat/get 'dur (pat/len-secs 'dur (pat/pure 'x) 1 1))
               (pat/span 0 1)))"
                .into(),
        ),
        // A chopped event has the seconds of its whole.
        (
            "chopped_event_has_its_whole",
            "((1.0 ((1 4) (1 2)) ((0 1) (1 1))))",
            "(pin-events (pat/query
               (pat/get 'dur (pat/len-secs 'dur (pat/pure (hash)) 1 1))
               (pat/span 1/4 1/2)))"
                .into(),
        ),
        // A signal has no whole, so it passes through.
        (
            "signal_passes",
            "(((1 2) ((0 1) (1 1)) #f))",
            "(pin-events (pat/query (pat/len-secs 'dur pat/saw 1 1) (pat/span 0 1)))".into(),
        ),
        // A missing cps leaves the pattern as it is.
        (
            "junk_cps_passes",
            "((x ((0 1) (1 1)) ((0 1) (1 1))))",
            "(pin-events (pat/query (pat/len-secs 'dur (pat/pure 'x) '() 1) (pat/span 0 1)))"
                .into(),
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}
