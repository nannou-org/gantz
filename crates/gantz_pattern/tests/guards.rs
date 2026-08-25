//! Partial-eval guard tests. A partial graph eval can hand any
//! combinator a non-pattern (an unfired input's `'()` or a void-flavored
//! binding) in place of a pattern, function, span or number. Every such
//! case must be silent rather than an application error.

mod common;

use common::assert_pinned;

// The junk values a partial eval can produce in place of a pattern.
const JUNK: &[&str] = &["'()", "void", "7", "'sym", "\"str\""];

// Querying junk directly yields no events.
#[test]
fn query_of_junk_is_silent() {
    for junk in JUNK {
        assert_pinned(
            "()",
            &format!("(pin-events (pat/query {junk} (pat/span 0 1)))"),
        );
    }
}

// Every combinator wrapping junk still queries silently.
#[test]
fn combinators_wrapping_junk_are_silent() {
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
    ];
    for junk in ["'()", "void"] {
        for wrap in wraps {
            let p = wrap.replace('J', junk);
            let src = format!("(length (pat/query {p} (pat/span 0 2)))");
            // Only assert it evaluates without error and stays a list
            // length (silent legs may still leave the non-junk legs
            // producing events, e.g. stack).
            common::assert_steel_true(&format!("(>= {src} 0)"));
        }
    }
}

// Joins with junk INNER values (a pattern of non-patterns) are silent.
#[test]
fn joins_with_junk_inner_values_are_silent() {
    for join in ["pat/join", "pat/inner-join", "pat/outer-join"] {
        assert_pinned(
            "()",
            &format!("(pin-events (pat/query ({join} (pat/pure 'not-a-pattern)) (pat/span 0 1)))"),
        );
    }
}

// The apply family drops events whose "function" is not applicable.
#[test]
fn apply_drops_non_fn_values() {
    assert_pinned(
        "()",
        "(pin-events (pat/query (pat/app (pat/pure 1) (pat/pure 'not-a-fn)) (pat/span 0 1)))",
    );
}

// Non-fn mapping and filtering fns yield silence.
#[test]
fn non_fn_map_and_filter_are_silent() {
    assert_pinned(
        "()",
        "(pin-events (pat/query (pat/map 'nope (pat/pure 1)) (pat/span 0 1)))",
    );
    assert_pinned(
        "()",
        "(pin-events (pat/query (pat/filter 'nope (pat/pure 1)) (pat/span 0 1)))",
    );
    assert_pinned(
        "()",
        "(pin-events (pat/query (pat/signal 'nope) (pat/span 0 1)))",
    );
}

// The windower holds position on junk time or cps, leaving state alone.
#[test]
fn window_with_junk_inputs_holds() {
    assert_pinned(
        "(((0 1) (0 1)) (1 2))",
        "(let ((r (pat/window 1/2 0.5 '())))
           (list (pin-span (car r)) (pin-num (car (cdr r)))))",
    );
    assert_pinned(
        "(((0 1) (0 1)) (1 2))",
        "(let ((r (pat/window 1/2 '() 1)))
           (list (pin-span (car r)) (pin-num (car (cdr r)))))",
    );
}

// Delivery with junk inputs emits nothing.
#[test]
fn events_to_secs_with_junk_is_silent() {
    for src in [
        "(pat/events->secs '() (pat/span 0 1) 0.0 1.0)",
        "(pat/events->secs 'junk (pat/span 0 1) 0.0 1.0)",
        "(pat/events->secs (pat/query (pat/pure 1) (pat/span 0 1)) '() 0.0 1.0)",
        "(pat/events->secs (pat/query (pat/pure 1) (pat/span 0 1)) (pat/span 0 1) '() 1.0)",
        "(pat/events->secs (pat/query (pat/pure 1) (pat/span 0 1)) (pat/span 0 1) 0.0 '())",
    ] {
        assert_pinned("()", src);
    }
}

// rationalize passes non-numbers through for downstream guards.
#[test]
fn rationalize_passes_junk_through() {
    common::assert_steel_true("(equal? '() (pat/rationalize '()))");
}
