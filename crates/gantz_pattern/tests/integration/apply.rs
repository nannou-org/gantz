//! Apply-family tests covering each variant's whole derivation and the
//! continuous-side degradation to a #f whole.

use crate::common::{assert_pinned, new_pin_engine};

const A: &str = "(pat/fast 2 (pat/pure 1))";
const B: &str = "(pat/fast 3 (pat/pure (lambda (v) (+ v 2))))";

#[test]
fn apply_family_events() {
    let mut vm = new_pin_engine();
    let app = format!("(pin-events (pat/query (pat/app {A} {B}) (pat/span 0 1)))");
    let appl = format!("(pin-events (pat/query (pat/appl {A} {B}) (pat/span 0 1)))");
    let appr = format!("(pin-events (pat/query (pat/appr {A} {B}) (pat/span 0 1)))");
    let rows: &[(&str, &str, &str)] = &[
        // fast 2 values applied with fast 3 functions over one cycle,
        // structure from the intersections.
        (
            "app_intersection_structure",
            "(((3 1) ((0 1) (1 3)) ((0 1) (1 3))) \
              ((3 1) ((1 3) (1 2)) ((1 3) (1 2))) \
              ((3 1) ((1 2) (2 3)) ((1 2) (2 3))) \
              ((3 1) ((2 3) (1 1)) ((2 3) (1 1))))",
            &app,
        ),
        // Same actives, wholes carried from the left, the value pattern.
        (
            "appl_left_structure",
            "(((3 1) ((0 1) (1 3)) ((0 1) (1 2))) \
              ((3 1) ((1 3) (1 2)) ((0 1) (1 2))) \
              ((3 1) ((1 2) (2 3)) ((1 2) (1 1))) \
              ((3 1) ((2 3) (1 1)) ((1 2) (1 1))))",
            &appl,
        ),
        // Same actives, wholes carried from the right, the function
        // pattern.
        (
            "appr_right_structure",
            "(((3 1) ((0 1) (1 3)) ((0 1) (1 3))) \
              ((3 1) ((1 3) (1 2)) ((1 3) (2 3))) \
              ((3 1) ((1 2) (2 3)) ((1 3) (2 3))) \
              ((3 1) ((2 3) (1 1)) ((2 3) (1 1))))",
            &appr,
        ),
        // The structure fn only applies when both wholes are present. A
        // continuous function pattern degrades even appl's whole to #f.
        (
            "appl_against_signal_degrades_whole",
            "(((2 1) ((0 1) (1 1)) #f))",
            "(pin-events (pat/query
               (pat/appl (pat/pure 1) (pat/steady (lambda (v) (+ v 1))))
               (pat/span 0 1)))",
        ),
        // merge-with combines values at intersections with app structure.
        (
            "merge_with_sums",
            "(((11 1) ((0 1) (1 3)) ((0 1) (1 3))) \
              ((11 1) ((1 3) (1 2)) ((1 3) (1 2))) \
              ((11 1) ((1 2) (2 3)) ((1 2) (2 3))) \
              ((11 1) ((2 3) (1 1)) ((2 3) (1 1))))",
            "(pin-events (pat/query
               (pat/merge-with + (pat/fast 2 (pat/pure 1)) (pat/fast 3 (pat/pure 10)))
               (pat/span 0 1)))",
        ),
        // pat/map transforms values leaving spans untouched, pat/filter
        // keeps matching values, pat/filter-events sees whole events.
        (
            "map",
            "(((10 1) ((0 1) (1 2)) ((0 1) (1 2))) \
              ((10 1) ((1 2) (1 1)) ((1 2) (1 1))))",
            "(pin-events (pat/query (pat/map (lambda (v) (* v 10)) (pat/fast 2 (pat/pure 1))) \
               (pat/span 0 1)))",
        ),
        (
            "filter",
            "((a ((0 1) (1 2)) ((0 1) (1 2))))",
            "(pin-events (pat/query
               (pat/filter (lambda (v) (equal? v 'a))
                           (pat/fastcat (list (pat/pure 'a) (pat/pure 'b))))
               (pat/span 0 1)))",
        ),
        (
            "filter_events",
            "((b ((1 2) (1 1)) ((1 2) (1 1))))",
            "(pin-events (pat/query
               (pat/filter-events (lambda (e) (<= 1/2 (car (pat/event-active e))))
                                  (pat/fastcat (list (pat/pure 'a) (pat/pure 'b))))
               (pat/span 0 1)))",
        ),
        // pat/map-events sees each event's spans. Here each value becomes
        // the start of its event's whole.
        (
            "map_events_sees_spans",
            "(((0 1) ((0 1) (1 2)) ((0 1) (1 2))) \
              ((1 2) ((1 2) (1 1)) ((1 2) (1 1))))",
            "(pin-events (pat/query
               (pat/map-events
                (lambda (e) (pat/event-map-value (lambda (v) (car (pat/event-whole e))) e))
                (pat/fast 2 (pat/pure 'x)))
               (pat/span 0 1)))",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}
