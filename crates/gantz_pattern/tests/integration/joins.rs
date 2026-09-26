//! Join tests discriminating the three variants' whole and active
//! derivation.

mod common;

use common::assert_pinned;

// join flattens an outer event spanning the whole query whose value is
// a two-per-cycle inner pattern.
#[test]
fn join_flattens_nested_pattern() {
    assert_pinned(
        "(((1 1) ((0 1) (1 2)) ((0 1) (1 2))) \
          ((1 1) ((1 2) (1 1)) ((1 2) (1 1))) \
          ((1 1) ((1 1) (3 2)) ((1 1) (3 2))) \
          ((1 1) ((3 2) (2 1)) ((3 2) (2 1))))",
        "(pin-events (pat/query
           (pat/join (lambda (s)
             (list (pat/event (pat/fastcat (list (pat/pure 1) (pat/pure 1))) s s))))
           (pat/span 0 2)))",
    );
}

// join chops the inner whole to the outer's, while inner-join keeps the
// inner whole untouched.
#[test]
fn join_chops_whole_inner_join_keeps_it() {
    let pp = "(pat/fast 2 (pat/pure (pat/pure 'c)))";
    assert_pinned(
        "((c ((0 1) (1 2)) ((0 1) (1 2))) \
          (c ((1 2) (1 1)) ((1 2) (1 1))))",
        &format!("(pin-events (pat/query (pat/join {pp}) (pat/span 0 1)))"),
    );
    assert_pinned(
        "((c ((0 1) (1 2)) ((0 1) (1 1))) \
          (c ((1 2) (1 1)) ((0 1) (1 1))))",
        &format!("(pin-events (pat/query (pat/inner-join {pp}) (pat/span 0 1)))"),
    );
}

// outer-join queries the inner at a zero-width instant, so a discrete
// inner yields nothing.
#[test]
fn outer_join_discrete_inner_is_silent() {
    assert_pinned(
        "()",
        "(pin-events (pat/query
           (pat/outer-join (pat/fast 2 (pat/pure (pat/pure 'c))))
           (pat/span 0 1)))",
    );
}

// A signal inner samples the outer whole's start instant, taking the
// outer's structure.
#[test]
fn outer_join_signal_inner_samples_start() {
    assert_pinned(
        "(((0 1) ((0 1) (1 2)) ((0 1) (1 2))) \
          ((1 2) ((1 2) (1 1)) ((1 2) (1 1))))",
        "(pin-events (pat/query
           (pat/outer-join (pat/fast 2 (pat/pure pat/saw)))
           (pat/span 0 1)))",
    );
}
