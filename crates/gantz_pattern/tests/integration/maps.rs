//! Control map construction, merging and delivery tests.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};

#[test]
fn map_patterns() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // A key wraps each value and keeps the structure.
        (
            "key_wraps_values",
            "((((1 2)) (((0 1) (1 2)))) (((3 2)) (((1 2) (1 1)))))",
            "(pin-map (lambda (e) (list (list (pin-num (hash-ref (pat/event-value e) 'n)))
                                        (list (pin-span (pat/event-active e)))))
                      (pat/query (pat/key 'n (pat/fastcat (list (pat/pure 1/2) (pat/pure 3/2))))
                                 (pat/span 0 1)))",
        ),
        // Structure comes from the left, and each left event takes the
        // right value that covers its onset. A later right value only
        // gives a continuation, which is not an onset.
        (
            "with_keeps_left_structure",
            "((1 1 #t) (1 2 #f))",
            "(pin-map (lambda (e) (list (hash-ref (pat/event-value e) 'a)
                                        (hash-ref (pat/event-value e) 'b)
                                        (pat/event-onset? e)))
                      (pat/query (pat/with (pat/key 'a (pat/pure 1))
                                           (pat/key 'b (pat/fastcat (list (pat/pure 1)
                                                                          (pat/pure 2)))))
                                 (pat/span 0 1)))",
        ),
        // A silent right side drops the event.
        (
            "with_drops_rests",
            "()",
            "(pat/query (pat/with (pat/key 'a (pat/pure 1)) pat/silence) (pat/span 0 1))",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
    // The rest of the merge rules.
    let rows: &[(&str, &str)] = &[
        (
            "with_right_key_wins",
            "(= 2 (hash-ref (pat/event-value
                              (car (pat/query (pat/with (pat/key 'a (pat/pure 1))
                                                        (pat/key 'a (pat/pure 2)))
                                              (pat/span 0 1))))
                            'a))",
        ),
        // A number gives way to a map, on either side.
        (
            "with_number_left_gives_way",
            "(= 2 (hash-ref (pat/event-value
                              (car (pat/query (pat/with (pat/pure 1) (pat/key 'a (pat/pure 2)))
                                              (pat/span 0 1))))
                            'a))",
        ),
        (
            "with_number_right_gives_way",
            "(= 1 (hash-ref (pat/event-value
                              (car (pat/query (pat/with (pat/key 'a (pat/pure 1)) (pat/pure 2))
                                              (pat/span 0 1))))
                            'a))",
        ),
        // A signal keeps the left structure, so the event stays an onset.
        // Its sample is over the whole event, so a window that chops the
        // event still gives the same value. A saw at the midpoint of the
        // whole cycle gives 1/2.
        (
            "with_signal_keeps_onsets",
            "(pat/event-onset?
               (car (pat/query (pat/with (pat/key 'a (pat/pure 1)) (pat/key 'b pat/saw))
                               (pat/span 0 1/4))))",
        ),
        (
            "with_samples_a_signal_over_the_whole",
            "(= 1/2 (hash-ref (pat/event-value
                                (car (pat/query (pat/with (pat/key 'a (pat/pure 1))
                                                          (pat/key 'b pat/saw))
                                                (pat/span 1/8 1/4))))
                              'b))",
        ),
        // Junk stays silent.
        (
            "with_junk_is_silent",
            "(empty? (pat/query (pat/with '() (pat/pure 1)) (pat/span 0 1)))",
        ),
    ];
    for (case, snippet) in rows {
        assert_steel_true(&mut vm, case, snippet);
    }
}

#[test]
fn delivery_of_maps() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str)] = &[
        // A delivered map gains the seconds of its whole, which can run past
        // the window. A quarter cycle at 2 cps takes 1/8 s.
        (
            "delta_in_seconds",
            "(let ((pairs (pat/events->secs
                            (pat/query (pat/key 'n (pat/fast 4 (pat/pure 1))) (pat/span 0 1/8))
                            (list 10.0 (list (list 0 1/8 2.0)) 2))))
               (= 0.125 (hash-ref (car (cdr (car pairs))) 'delta)))",
        ),
        // A tempo change after the onset still counts. The event takes 1/2
        // cycle at 1 cps and 1/2 cycle at 2 cps.
        (
            "delta_follows_the_tempo_past_the_window",
            "(let ((pairs (pat/events->secs
                            (pat/query (pat/key 'n (pat/pure 1)) (pat/span 0 1/4))
                            (list 0.0
                                  (list (list 0 1/4 1.0))
                                  (pat/fastcat (list (pat/pure 1) (pat/pure 2)))))))
               (= 0.75 (hash-ref (car (cdr (car pairs))) 'delta)))",
        ),
        // A gap in the tempo holds the rate at the onset.
        (
            "delta_holds_the_onset_rate_over_gaps",
            "(let ((pairs (pat/events->secs
                            (pat/query (pat/key 'n (pat/pure 1)) (pat/span 0 1/4))
                            (list 0.0 (list (list 0 1/4 4.0)) pat/silence))))
               (= 0.25 (hash-ref (car (cdr (car pairs))) 'delta)))",
        ),
        // A value that is not a map stays as it is.
        (
            "numbers_stay_plain",
            "(equal? (list (list 0.0 1.0))
                     (pat/events->secs (pat/query (pat/pure 1) (pat/span 0 1/4))
                                       (list 0.0 (list (list 0 1/4 1.0)) 1)))",
        ),
    ];
    for (case, snippet) in rows {
        assert_steel_true(&mut vm, case, snippet);
    }
}

#[test]
fn pick() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // Numbers leave as floats, for the control inputs.
        (
            "numbers_leave_as_floats",
            "((1.0 0.5) (2.0 1.5))",
            "(pat/pick (list (list 1.0 (hash 'n 1/2)) (list 2.0 (hash 'n 3/2))) 'n)",
        ),
        // A pair without the key drops, as does a value that is not a map.
        (
            "other_values_drop",
            "((2.0 3.0))",
            "(pat/pick (list (list 1.0 (hash 'm 1)) (list 1.5 7.0) (list 2.0 (hash 'n 3))) 'n)",
        ),
        // A value that is not a number passes through.
        (
            "symbols_pass",
            "((1.0 bd))",
            "(pat/pick (list (list 1.0 (hash 's 'bd))) 's)",
        ),
        ("junk_is_silent", "()", "(pat/pick '() 'n)"),
        ("junk_pairs_are_silent", "()", "(pat/pick 'junk 'n)"),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}
