//! Apply-family tests: cycles' `test_apply` span table for `app`, plus
//! derived whole tables for `appl`/`appr` and the continuous-side
//! degradation to a #f whole.

mod common;

use common::assert_pinned;

const A: &str = "(pat/fast 2 (pat/pure 1))";
const B: &str = "(pat/fast 3 (pat/pure (lambda (v) (+ v 2))))";

// Ported `test_apply`: fast 2 values applied with fast 3 functions over
// one cycle, structure from the intersections.
#[test]
fn app_intersection_structure() {
    assert_pinned(
        "(((3 1) ((0 1) (1 3)) ((0 1) (1 3))) \
          ((3 1) ((1 3) (1 2)) ((1 3) (1 2))) \
          ((3 1) ((1 2) (2 3)) ((1 2) (2 3))) \
          ((3 1) ((2 3) (1 1)) ((2 3) (1 1))))",
        &format!("(pin-events (pat/query (pat/app {A} {B}) (pat/span 0 1)))"),
    );
}

// Same actives, wholes carried from the left (the value pattern).
#[test]
fn appl_left_structure() {
    assert_pinned(
        "(((3 1) ((0 1) (1 3)) ((0 1) (1 2))) \
          ((3 1) ((1 3) (1 2)) ((0 1) (1 2))) \
          ((3 1) ((1 2) (2 3)) ((1 2) (1 1))) \
          ((3 1) ((2 3) (1 1)) ((1 2) (1 1))))",
        &format!("(pin-events (pat/query (pat/appl {A} {B}) (pat/span 0 1)))"),
    );
}

// Same actives, wholes carried from the right (the function pattern).
#[test]
fn appr_right_structure() {
    assert_pinned(
        "(((3 1) ((0 1) (1 3)) ((0 1) (1 3))) \
          ((3 1) ((1 3) (1 2)) ((1 3) (2 3))) \
          ((3 1) ((1 2) (2 3)) ((1 3) (2 3))) \
          ((3 1) ((2 3) (1 1)) ((2 3) (1 1))))",
        &format!("(pin-events (pat/query (pat/appr {A} {B}) (pat/span 0 1)))"),
    );
}

// The structure fn only applies when BOTH wholes are present: a
// continuous function pattern degrades even appl's whole to #f.
#[test]
fn appl_against_signal_degrades_whole() {
    assert_pinned(
        "(((2 1) ((0 1) (1 1)) #f))",
        "(pin-events (pat/query
           (pat/appl (pat/pure 1) (pat/steady (lambda (v) (+ v 1))))
           (pat/span 0 1)))",
    );
}

// merge-with combines values at intersections with app structure.
#[test]
fn merge_with_sums() {
    assert_pinned(
        "(((11 1) ((0 1) (1 3)) ((0 1) (1 3))) \
          ((11 1) ((1 3) (1 2)) ((1 3) (1 2))) \
          ((11 1) ((1 2) (2 3)) ((1 2) (2 3))) \
          ((11 1) ((2 3) (1 1)) ((2 3) (1 1))))",
        "(pin-events (pat/query
           (pat/merge-with + (pat/fast 2 (pat/pure 1)) (pat/fast 3 (pat/pure 10)))
           (pat/span 0 1)))",
    );
}

// pat/map transforms values leaving spans untouched, pat/filter keeps
// matching values, pat/filter-events sees whole events.
#[test]
fn map_and_filters() {
    assert_pinned(
        "(((10 1) ((0 1) (1 2)) ((0 1) (1 2))) \
          ((10 1) ((1 2) (1 1)) ((1 2) (1 1))))",
        "(pin-events (pat/query (pat/map (lambda (v) (* v 10)) (pat/fast 2 (pat/pure 1))) \
           (pat/span 0 1)))",
    );
    assert_pinned(
        "((a ((0 1) (1 2)) ((0 1) (1 2))))",
        "(pin-events (pat/query
           (pat/filter (lambda (v) (equal? v 'a))
                       (pat/fastcat (list (pat/pure 'a) (pat/pure 'b))))
           (pat/span 0 1)))",
    );
    assert_pinned(
        "((b ((1 2) (1 1)) ((1 2) (1 1))))",
        "(pin-events (pat/query
           (pat/filter-events (lambda (e) (<= 1/2 (car (pat/event-active e))))
                              (pat/fastcat (list (pat/pure 'a) (pat/pure 'b))))
           (pat/span 0 1)))",
    );
}
