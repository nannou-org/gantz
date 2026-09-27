//! Plot data and plot span coercion tests.
//!
//! `pat/plot-data` returns only floats, bools and lists, so its results
//! compare with `equal?` directly.

use crate::common::{assert_pinned, new_pin_engine};

#[test]
fn plot_data() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // Each discrete event is one segment over its active span. Events
        // that start within the span are onsets.
        (
            "discrete_events_are_segments",
            "(0.0 1.0 ((0.0 0.5 1.0 #t) (0.5 1.0 2.0 #t)) ())",
            "(pat/plot-data (pat/fastcat (list (pat/pure 1) (pat/pure 2))) (pat/span 0 1) 4)",
        ),
        // Segments clip to the span. An event cut at the span start is not
        // an onset.
        (
            "segments_clip_to_span",
            "(0.25 0.75 ((0.25 0.5 1.0 #f) (0.5 0.75 2.0 #t)) ())",
            "(pat/plot-data (pat/fastcat (list (pat/pure 1) (pat/pure 2))) (pat/span 1/4 3/4) 4)",
        ),
        // A signal is sampled once per slice, at the midpoint of each
        // slice.
        (
            "signals_are_sampled_per_slice",
            "(0.0 1.0 () ((0.125 0.125) (0.375 0.375) (0.625 0.625) (0.875 0.875)))",
            "(pat/plot-data pat/saw (pat/span 0 1) 4)",
        ),
        // A stack of a signal and a discrete pattern gives both.
        (
            "stacked_signal_and_events",
            "(0.0 1.0 ((0.0 1.0 3.0 #t)) ((0.25 0.25) (0.75 0.75)))",
            "(pat/plot-data (pat/stack (list pat/saw (pat/pure 3))) (pat/span 0 1) 2)",
        ),
        // Non-numeric values pass through for the plotter to classify.
        (
            "bool_values_pass_through",
            "(0.0 1.0 ((0.0 0.5 #t #t) (0.5 1.0 #f #t)) ())",
            "(pat/plot-data (pat/fastcat (list (pat/pure #t) (pat/pure #f))) (pat/span 0 1) 4)",
        ),
        (
            "string_and_symbol_values_pass_through",
            "(0.0 1.0 ((0.0 0.5 \"a\" #t) (0.5 1.0 bd #t)) ())",
            "(pat/plot-data (pat/fastcat (list (pat/pure \"a\") (pat/pure 'bd))) (pat/span 0 1) 4)",
        ),
        // A non-pattern input plots nothing over the span.
        (
            "non_pattern_is_empty",
            "(0.0 1.0 () ())",
            "(pat/plot-data 5 (pat/span 0 1) 4)",
        ),
        // Map values pass through whole. The base engine builds them with
        // `hash`.
        (
            "map_values_pass_through",
            "(bd 2)",
            "(let ((v (car (cdr (cdr (car (car (cdr (cdr (pat/plot-data (pat/pure (hash 's 'bd 'n 2)) (pat/span 0 1) 4))))))))))
               (list (hash-ref v 's) (hash-ref v 'n)))",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

// A number is the span from 0. A number pair is rationalized. Anything
// else, and any span that does not move forward, falls back.
#[test]
fn as_span_coerces_or_falls_back() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        ("int", "((0 1) (2 1))", "(pin-span (pat/as-span 2 'd))"),
        ("float", "((0 1) (3 2))", "(pin-span (pat/as-span 1.5 'd))"),
        (
            "number_pair",
            "((1 4) (3 4))",
            "(pin-span (pat/as-span (cons 1/4 0.75) 'd))",
        ),
        ("string", "d", "(pat/as-span \"x\" 'd)"),
        ("backwards_pair", "d", "(pat/as-span (cons 1 0) 'd)"),
        ("zero", "d", "(pat/as-span 0 'd)"),
        ("list", "d", "(pat/as-span (list 0 1) 'd)"),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}
