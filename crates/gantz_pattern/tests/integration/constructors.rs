//! Constructor and query tests.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};

#[test]
fn constructor_and_query_events() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // pure yields one event per cycle with whole equal to the cycle.
        (
            "pure_values_per_cycle",
            "((hello ((0 1) (1 1)) ((0 1) (1 1))) \
              (hello ((1 1) (2 1)) ((1 1) (2 1))) \
              (hello ((2 1) (3 1)) ((2 1) (3 1))))",
            "(pin-events (pat/query (pat/pure 'hello) (pat/span 0 3)))",
        ),
        // A partial trailing cycle keeps the full-cycle whole while the
        // active is clipped to the query.
        (
            "pure_partial_cycle_whole",
            "((x ((0 1) (1 1)) ((0 1) (1 1))) \
              (x ((1 1) (2 1)) ((1 1) (2 1))) \
              (x ((2 1) (3 1)) ((2 1) (3 1))) \
              (x ((3 1) (7 2)) ((3 1) (4 1))))",
            "(pin-events (pat/query (pat/pure 'x) (pat/span 0 7/2)))",
        ),
        // A zero-width query yields nothing from a discrete pattern.
        (
            "pure_empty_span",
            "()",
            "(pin-events (pat/query (pat/pure 'x) (pat/span 1/2 1/2)))",
        ),
        // indices yields each cycle's index, negative before cycle 0, with
        // the same structure as pure.
        (
            "indices_values_per_cycle",
            "(((-1 1) ((-1 1) (0 1)) ((-1 1) (0 1))) \
              ((0 1) ((0 1) (1 1)) ((0 1) (1 1))) \
              ((1 1) ((1 1) (2 1)) ((1 1) (2 1))) \
              ((2 1) ((2 1) (5 2)) ((2 1) (3 1))))",
            "(pin-events (pat/query pat/indices (pat/span -1 5/2)))",
        ),
        // Indices count the cycles of the pattern's own time, so a faster
        // pattern yields more of them per cycle.
        (
            "indices_follow_pattern_time",
            "(((0 1) ((0 1) (1 2)) ((0 1) (1 2))) \
              ((1 1) ((1 2) (1 1)) ((1 2) (1 1))))",
            "(pin-events (pat/query (pat/fast 2 pat/indices) (pat/span 0 1)))",
        ),
        // A signal yields exactly one whole-less event for any query,
        // sampling the midpoint. That includes a zero-width instant query.
        (
            "saw_samples_instant",
            "(((1 2) ((1 2) (1 2)) #f))",
            "(pin-events (pat/query pat/saw (pat/span 1/2 1/2)))",
        ),
        (
            "saw_samples_midpoint_of_wide_query",
            "(((1 2) ((0 1) (1 1)) #f))",
            "(pin-events (pat/query pat/saw (pat/span 0 1)))",
        ),
        // saw2 is the polar saw, 0 at phase 1/2.
        (
            "saw2_zero_at_half",
            "(((0 1) ((1 2) (1 2)) #f))",
            "(pin-events (pat/query pat/saw2 (pat/span 1/2 1/2)))",
        ),
        (
            "saw2_negative_at_quarter",
            "(((-1 2) ((1 4) (1 4)) #f))",
            "(pin-events (pat/query pat/saw2 (pat/span 1/4 1/4)))",
        ),
        // silence always yields nothing.
        (
            "silence",
            "()",
            "(pin-events (pat/query pat/silence (pat/span 0 10)))",
        ),
        // query sorts events by active start. The pattern is deliberately
        // reversed.
        (
            "query_sorts_by_active_start",
            "((a ((0 1) (1 2)) #f) (b ((1 2) (1 1)) #f))",
            "(pin-events (pat/query
               (lambda (span)
                 (list (pat/event 'b (pat/span 1/2 1) #f)
                       (pat/event 'a (pat/span 0 1/2) #f)))
               (pat/span 0 1)))",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

#[test]
fn signal_sampling_equivalences() {
    let mut vm = new_pin_engine();
    // Negative saw phases wrap.
    let saw_value = |span: &str| {
        format!("(pin-value (pat/event-value (car (pat/query pat/saw (pat/span {span})))))")
    };
    let rows = [
        (
            "saw_wraps_minus_half",
            format!(
                "(equal? {} {})",
                saw_value("-1/2 -1/2"),
                saw_value("1/2 1/2"),
            ),
        ),
        (
            "saw_wraps_minus_three_quarters",
            format!(
                "(equal? {} {})",
                saw_value("-3/4 -3/4"),
                saw_value("1/4 1/4"),
            ),
        ),
        // steady always yields its value.
        (
            "steady",
            "(define (all-sevens n)
               (if (< n 0)
                   #t
                   (let ((es (pat/query (pat/steady 7) (pat/span (/ n 10) (/ n 10)))))
                     (if (= (length es) 1)
                         (if (= (pat/event-value (car es)) 7)
                             (all-sevens (- n 1))
                             #f)
                         #f))))
             (all-sevens 10)"
                .to_string(),
        ),
    ];
    for (case, snippet) in &rows {
        assert_steel_true(&mut vm, case, snippet);
    }
}
