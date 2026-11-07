//! Tests for `pat/cache`.

use crate::common::{assert_steel_true, eval, new_pin_engine};
use gantz_core::steel::SteelVal;

/// Patterns with events that cross slices, cycles and query edges.
const PATTERNS: &[&str] = &[
    "(pat/fastcat (list (pat/pure 1) (pat/pure 2) (pat/pure 3)))",
    "(pat/timecat (list (list 1 (pat/pure 'a)) (list 3 (pat/pure 'b)) (list 2 pat/silence)))",
    "(pat/slow 3/2 (pat/fastcat (list (pat/pure 'x) (pat/pure 'y))))",
    "(pat/shift 1/7 (pat/fast 5 pat/indices))",
    "(pat/stack (list (pat/pure 0) (pat/fast 3 (pat/pure 1))))",
    "(pat/slowcat (list (pat/fast 2 (pat/pure 'p)) (pat/pure 'q)))",
];

/// Query spans: whole cycles, parts of a slice, slice edges and spans
/// across cycles.
const SPANS: &[&str] = &[
    "0 1",
    "0 3",
    "1/3 2/3",
    "1/5 1/4",
    "1/4 1/2",
    "7/8 9/8",
    "5/2 41/10",
    "-1/2 1/2",
    "1/1200 2/1200",
];

/// A cached pattern gives the events of the pattern for each span and
/// slice count, with the query split at cycle boundaries.
#[test]
fn cache_gives_the_events_of_its_pattern() {
    let mut vm = new_pin_engine();
    eval(
        &mut vm,
        "by-cycle",
        "(require \"gantz/list\")
         (define (by-cycle p)
           (lambda (s) (list/flat-map p (pat/span-cycles s))))",
    );
    for p in PATTERNS {
        for n in [1, 4, 7] {
            eval(
                &mut vm,
                "define",
                &format!("(define c (pat/cache {n} {p}))"),
            );
            for span in SPANS {
                let case = format!("{p} n {n} span {span}");
                let check = format!(
                    "(equal? (pin-events (pat/query c (pat/span {span})))
                             (pin-events (pat/query (by-cycle {p}) (pat/span {span}))))"
                );
                assert_steel_true(&mut vm, &case, &check);
            }
        }
    }
}

/// Signals and zero-width queries go to the pattern.
#[test]
fn cache_passes_signals_and_zero_width_queries_through() {
    let mut vm = new_pin_engine();
    let rows = [
        ("pat/saw", "1/3 1/2"),
        ("(pat/stack (list (pat/pure 1) pat/saw))", "0 1/2"),
        ("(pat/pure 1)", "1/2 1/2"),
    ];
    for (p, span) in rows {
        let check = format!(
            "(equal? (pin-events (pat/query (pat/cache 4 {p}) (pat/span {span})))
                     (pin-events (pat/query {p} (pat/span {span}))))"
        );
        assert_steel_true(&mut vm, p, &check);
    }
}

/// The pattern is queried once per kept cycle. The latest four cycles are
/// kept.
#[test]
fn cache_queries_each_cycle_once() {
    let mut vm = new_pin_engine();
    eval(
        &mut vm,
        "define",
        "(define calls 0)
         (define c (pat/cache 8 (lambda (span) (set! calls (+ calls 1)) ((pat/pure 1) span))))
         (define (q s e) (pat/query c (pat/span s e)))",
    );
    let calls = |vm: &mut _, case: &str, snippet: &str| match eval(vm, case, snippet) {
        SteelVal::IntV(n) => n,
        v => panic!("{case}: {v:?}"),
    };
    assert_eq!(
        calls(&mut vm, "parts", "(q 0 1/3) (q 1/3 1/2) (q 1/2 1) calls"),
        1
    );
    assert_eq!(calls(&mut vm, "across", "(q 1/2 3/2) calls"), 2);
    assert_eq!(calls(&mut vm, "more", "(q 2 5) calls"), 5);
    assert_eq!(calls(&mut vm, "kept", "(q 1 5) calls"), 5);
    assert_eq!(calls(&mut vm, "dropped", "(q 0 1) calls"), 6);
}

/// A slice count that is not a positive integer counts as 1, and a value
/// that is not a pattern is silent.
#[test]
fn cache_of_junk() {
    let mut vm = new_pin_engine();
    for n in ["'x", "0", "-2", "1/2"] {
        let check = format!(
            "(equal? (pin-events (pat/query (pat/cache {n} (pat/fast 2 (pat/pure 1))) (pat/span 0 1)))
                     (pin-events (pat/query (pat/fast 2 (pat/pure 1)) (pat/span 0 1))))"
        );
        assert_steel_true(&mut vm, n, &check);
    }
    assert_steel_true(
        &mut vm,
        "not a pattern",
        "(empty? (pat/query (pat/cache 4 '()) (pat/span 0 1)))",
    );
}
