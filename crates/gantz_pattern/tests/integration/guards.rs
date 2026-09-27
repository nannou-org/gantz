//! Partial-eval guard tests. A partial graph eval can hand any
//! combinator a non-pattern in place of a pattern, function, span or
//! number. That is an unfired input's `'()` or a void-flavored binding.
//! Every such case must be silent rather than an application error.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};

// The junk values a partial eval can produce in place of a pattern.
const JUNK: &[&str] = &["'()", "void", "7", "'sym", "\"str\""];

// Junk patterns, junk inner patterns and junk fns all query silently.
#[test]
fn junk_patterns_and_fns_are_silent() {
    let mut vm = new_pin_engine();
    // Querying junk directly yields no events.
    for junk in JUNK {
        assert_pinned(
            &mut vm,
            &format!("query of {junk}"),
            "()",
            &format!("(pin-events (pat/query {junk} (pat/span 0 1)))"),
        );
    }
    // Joins with junk inner values, a pattern of non-patterns, are silent.
    for join in ["pat/join", "pat/inner-join", "pat/outer-join"] {
        assert_pinned(
            &mut vm,
            &format!("{join} of a junk inner"),
            "()",
            &format!("(pin-events (pat/query ({join} (pat/pure 'not-a-pattern)) (pat/span 0 1)))"),
        );
    }
    // map-events drops results that are not events, such as the junk of a
    // partial eval of the mapping fn. Events kept as they are stay.
    for junk in JUNK {
        assert_pinned(
            &mut vm,
            &format!("map-events to {junk}"),
            "((a ((0 1) (1 2)) ((0 1) (1 2))))",
            &format!(
                "(pin-events (pat/query
                   (pat/map-events
                    (lambda (e) (if (equal? (pat/event-value e) 'a) e {junk}))
                    (pat/fastcat (list (pat/pure 'a) (pat/pure 'b))))
                   (pat/span 0 1)))"
            ),
        );
    }
    let rows: &[(&str, &str)] = &[
        // The apply family drops events whose "function" is not applicable.
        (
            "app_drops_non_fn_values",
            "(pin-events (pat/query (pat/app (pat/pure 1) (pat/pure 'not-a-fn)) (pat/span 0 1)))",
        ),
        // Non-fn mapping and filtering fns yield silence.
        (
            "map_with_non_fn",
            "(pin-events (pat/query (pat/map 'nope (pat/pure 1)) (pat/span 0 1)))",
        ),
        (
            "map_events_with_non_fn",
            "(pin-events (pat/query (pat/map-events 'nope (pat/pure 1)) (pat/span 0 1)))",
        ),
        (
            "filter_with_non_fn",
            "(pin-events (pat/query (pat/filter 'nope (pat/pure 1)) (pat/span 0 1)))",
        ),
        (
            "filter_events_with_non_fn",
            "(pin-events (pat/query (pat/filter-events 'nope (pat/pure 1)) (pat/span 0 1)))",
        ),
        (
            "degrade_by_with_non_fn",
            "(pin-events (pat/query (pat/degrade-by 7 'nope (pat/pure 1)) (pat/span 0 1)))",
        ),
        (
            "signal_with_non_fn",
            "(pin-events (pat/query (pat/signal 'nope) (pat/span 0 1)))",
        ),
    ];
    for (case, expr) in rows {
        assert_pinned(&mut vm, case, "()", expr);
    }
}

// Every combinator wrapping junk still queries silently.
#[test]
fn combinators_wrapping_junk_are_silent() {
    let mut vm = new_pin_engine();
    let wraps = [
        "(pat/fast 2 J)",
        "(pat/slow 2 J)",
        "(pat/shift 1/4 J)",
        "(pat/slowcat (list J (pat/pure 'a)))",
        "(pat/fastcat (list J (pat/pure 'a) J))",
        "(pat/timecat (list (list 1 J) (list 2 (pat/pure 'a))))",
        "(pat/stack (list J (pat/pure 'a)))",
        "(pat/fit-span (pat/span 0 1) (pat/span 0 1/2) J)",
        "(pat/map (lambda (v) v) J)",
        "(pat/map-events (lambda (e) e) J)",
        "(pat/filter (lambda (v) #t) J)",
        "(pat/filter-events (lambda (e) #t) J)",
        "(pat/app J (pat/pure (lambda (v) v)))",
        "(pat/app (pat/pure 1) J)",
        "(pat/appl (pat/pure 1) J)",
        "(pat/appr (pat/pure 1) J)",
        "(pat/merge-with + (pat/pure 1) J)",
        "(pat/join J)",
        "(pat/inner-join J)",
        "(pat/outer-join J)",
        "(pat/euclid-with J 3 8 0)",
        "(pat/degrade-by 7 1/2 J)",
        "(pat/degrade-by J 1/2 (pat/pure 'a))",
        // Any value is a seed, so a junk seed still gives values.
        "(pat/rand J)",
    ];
    for junk in ["'()", "void"] {
        for wrap in wraps {
            let p = wrap.replace('J', junk);
            let src = format!("(length (pat/query {p} (pat/span 0 2)))");
            // Only assert it evaluates without error and stays a list
            // length. Silent legs may still leave the non-junk legs
            // producing events, as in stack.
            assert_steel_true(&mut vm, &p, &format!("(>= {src} 0)"));
        }
    }
}

// The windower and delivery helpers hold or stay silent on junk inputs.
#[test]
fn junk_window_and_delivery_inputs() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // The windower holds position on junk time or cps, leaving state
        // alone.
        (
            "window_with_junk_cps_holds",
            "(((0 1) (0 1)) (1 2))",
            "(let ((r (pat/window 1/2 0.5 '())))
               (list (pin-span (car r)) (pin-num (car (cdr r)))))",
        ),
        (
            "window_with_junk_time_holds",
            "(((0 1) (0 1)) (1 2))",
            "(let ((r (pat/window 1/2 '() 1)))
               (list (pin-span (car r)) (pin-num (car (cdr r)))))",
        ),
        // Delivery with junk inputs emits nothing.
        (
            "events_to_secs_with_empty_events",
            "()",
            "(pat/events->secs '() (pat/span 0 1) 0.0 1.0)",
        ),
        (
            "events_to_secs_with_junk_events",
            "()",
            "(pat/events->secs 'junk (pat/span 0 1) 0.0 1.0)",
        ),
        (
            "events_to_secs_with_junk_span",
            "()",
            "(pat/events->secs (pat/query (pat/pure 1) (pat/span 0 1)) '() 0.0 1.0)",
        ),
        (
            "events_to_secs_with_junk_time",
            "()",
            "(pat/events->secs (pat/query (pat/pure 1) (pat/span 0 1)) (pat/span 0 1) '() 1.0)",
        ),
        (
            "events_to_secs_with_junk_cps",
            "()",
            "(pat/events->secs (pat/query (pat/pure 1) (pat/span 0 1)) (pat/span 0 1) 0.0 '())",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
    // rationalize passes non-numbers through for downstream guards.
    assert_steel_true(
        &mut vm,
        "rationalize_passes_junk_through",
        "(equal? '() (pat/rationalize '()))",
    );
}
