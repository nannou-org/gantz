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
        // A delivered map keeps its keys and gains none.
        (
            "maps_stay_as_they_are",
            "(let ((m (car (cdr (car (pat/events->secs
                                       (pat/query (pat/key 'n (pat/pure 1)) (pat/span 0 1/4))
                                       0.0
                                       (list (list 0 1/4 1.0))))))))
               (if (= 1 (hash-ref m 'n)) (not (hash-contains? m 'delta)) #f))",
        ),
        // A number leaves as a float.
        (
            "numbers_stay_plain",
            "(equal? (list (list 0.0 1.0))
                     (pat/events->secs (pat/query (pat/pure 1) (pat/span 0 1/4))
                                       0.0
                                       (list (list 0 1/4 1.0))))",
        ),
    ];
    for (case, snippet) in rows {
        assert_steel_true(&mut vm, case, snippet);
    }
}
