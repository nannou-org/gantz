//! Join tests discriminating the three variants' whole and active
//! derivation.

use crate::common::{assert_pinned, new_pin_engine};

#[test]
fn join_variants_events() {
    let mut vm = new_pin_engine();
    let pp = "(pat/fast 2 (pat/pure (pat/pure 'c)))";
    let join = format!("(pin-events (pat/query (pat/join {pp}) (pat/span 0 1)))");
    let inner_join = format!("(pin-events (pat/query (pat/inner-join {pp}) (pat/span 0 1)))");
    let rows: &[(&str, &str, &str)] = &[
        // join flattens an outer event spanning the whole query whose
        // value is a two-per-cycle inner pattern.
        (
            "join_flattens_nested_pattern",
            "(((1 1) ((0 1) (1 2)) ((0 1) (1 2))) \
              ((1 1) ((1 2) (1 1)) ((1 2) (1 1))) \
              ((1 1) ((1 1) (3 2)) ((1 1) (3 2))) \
              ((1 1) ((3 2) (2 1)) ((3 2) (2 1))))",
            "(pin-events (pat/query
               (pat/join (lambda (s)
                 (list (pat/event (pat/fastcat (list (pat/pure 1) (pat/pure 1))) s s))))
               (pat/span 0 2)))",
        ),
        // join chops the inner whole to the outer's, while inner-join
        // keeps the inner whole untouched.
        (
            "join_chops_whole",
            "((c ((0 1) (1 2)) ((0 1) (1 2))) \
              (c ((1 2) (1 1)) ((1 2) (1 1))))",
            &join,
        ),
        (
            "inner_join_keeps_whole",
            "((c ((0 1) (1 2)) ((0 1) (1 1))) \
              (c ((1 2) (1 1)) ((0 1) (1 1))))",
            &inner_join,
        ),
        // outer-join queries the inner at a zero-width instant, so a
        // discrete inner yields nothing.
        (
            "outer_join_discrete_inner_is_silent",
            "()",
            "(pin-events (pat/query
               (pat/outer-join (pat/fast 2 (pat/pure (pat/pure 'c))))
               (pat/span 0 1)))",
        ),
        // A signal inner samples the outer whole's start instant, taking
        // the outer's structure.
        (
            "outer_join_signal_inner_samples_start",
            "(((0 1) ((0 1) (1 2)) ((0 1) (1 2))) \
              ((1 2) ((1 2) (1 1)) ((1 2) (1 1))))",
            "(pin-events (pat/query
               (pat/outer-join (pat/fast 2 (pat/pure pat/saw)))
               (pat/span 0 1)))",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}
