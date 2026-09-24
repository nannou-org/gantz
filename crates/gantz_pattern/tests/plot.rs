//! Plot data and plot span coercion tests.
//!
//! `pat/plot-data` returns only floats, bools and lists, so its results
//! compare with `equal?` directly.

mod common;

use common::assert_pinned;

// Each discrete event is one segment over its active span. Events that
// start within the span are onsets.
#[test]
fn discrete_events_are_segments() {
    assert_pinned(
        "(0.0 1.0 ((0.0 0.5 1.0 #t) (0.5 1.0 2.0 #t)) ())",
        "(pat/plot-data (pat/fastcat (list (pat/pure 1) (pat/pure 2))) (pat/span 0 1) 4)",
    );
}

// Segments clip to the span. An event cut at the span start is not an
// onset.
#[test]
fn segments_clip_to_span() {
    assert_pinned(
        "(0.25 0.75 ((0.25 0.5 1.0 #f) (0.5 0.75 2.0 #t)) ())",
        "(pat/plot-data (pat/fastcat (list (pat/pure 1) (pat/pure 2))) (pat/span 1/4 3/4) 4)",
    );
}

// A signal is sampled once per slice, at the midpoint of each slice.
#[test]
fn signals_are_sampled_per_slice() {
    assert_pinned(
        "(0.0 1.0 () ((0.125 0.125) (0.375 0.375) (0.625 0.625) (0.875 0.875)))",
        "(pat/plot-data pat/saw (pat/span 0 1) 4)",
    );
}

// A stack of a signal and a discrete pattern gives both.
#[test]
fn stacked_signal_and_events() {
    assert_pinned(
        "(0.0 1.0 ((0.0 1.0 3.0 #t)) ((0.25 0.25) (0.75 0.75)))",
        "(pat/plot-data (pat/stack (list pat/saw (pat/pure 3))) (pat/span 0 1) 2)",
    );
}

// Bools plot as 1 and 0. Other non-numeric values are dropped.
#[test]
fn values_map_to_floats() {
    assert_pinned(
        "(0.0 1.0 ((0.0 0.5 1.0 #t) (0.5 1.0 0.0 #t)) ())",
        "(pat/plot-data (pat/fastcat (list (pat/pure #t) (pat/pure #f))) (pat/span 0 1) 4)",
    );
    assert_pinned(
        "(0.0 1.0 ((0.5 1.0 2.0 #t)) ())",
        "(pat/plot-data (pat/fastcat (list (pat/pure \"a\") (pat/pure 2))) (pat/span 0 1) 4)",
    );
}

// A non-pattern input plots nothing over the span.
#[test]
fn non_pattern_is_empty() {
    assert_pinned("(0.0 1.0 () ())", "(pat/plot-data 5 (pat/span 0 1) 4)");
}

// A number is the span from 0. A number pair is rationalized. Anything
// else, and any span that does not move forward, falls back.
#[test]
fn as_span_coerces_or_falls_back() {
    assert_pinned("((0 1) (2 1))", "(pin-span (pat/as-span 2 'd))");
    assert_pinned("((0 1) (3 2))", "(pin-span (pat/as-span 1.5 'd))");
    assert_pinned(
        "((1 4) (3 4))",
        "(pin-span (pat/as-span (cons 1/4 0.75) 'd))",
    );
    assert_pinned("d", "(pat/as-span \"x\" 'd)");
    assert_pinned("d", "(pat/as-span (cons 1 0) 'd)");
    assert_pinned("d", "(pat/as-span 0 'd)");
    assert_pinned("d", "(pat/as-span (list 0 1) 'd)");
}
