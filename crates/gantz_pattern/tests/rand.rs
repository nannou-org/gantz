//! Seeded randomness tests for `pat/rand` and `pat/degrade-by`.

mod common;

use common::{assert_pinned, assert_steel_true};

// The value of `(pat/rand 7)` at the instant `t`.
fn rand_at(seed: &str, t: &str) -> String {
    format!("(pat/event-value (car (pat/query (pat/rand {seed}) (pat/span {t} {t}))))")
}

// rand is a whole-less signal whose value at `t` is the uniform draw from
// the seed with `t` folded in.
#[test]
fn rand_is_a_seeded_signal() {
    assert_steel_true(&format!(
        "(require \"gantz/rng\")
         (equal? {} (rng/uniform (rng/fold-in 7 1/4)))",
        rand_at("7", "1/4"),
    ));
    assert_pinned(
        "#f",
        "(pat/event-whole (car (pat/query (pat/rand 7) (pat/span 0 1))))",
    );
}

// Values stay in [0, 1), repeat for the same seed and time, and differ
// between seeds.
#[test]
fn rand_values() {
    assert_steel_true(
        "(define (in-range n)
           (if (>= n 64)
               #t
               (let ((v (pat/event-value
                         (car (pat/query (pat/rand 7) (pat/span (/ n 16) (/ n 16)))))))
                 (if (>= v 0) (if (< v 1) (in-range (+ n 1)) #f) #f))))
         (in-range 0)",
    );
    assert_steel_true(&format!(
        "(equal? {} {})",
        rand_at("7", "3/8"),
        rand_at("7", "3/8")
    ));
    assert_steel_true(&format!(
        "(not (equal? {} {}))",
        rand_at("7", "3/8"),
        rand_at("8", "3/8")
    ));
}

// A probability of 0 keeps every event, and 1 drops every event. That
// includes whole-less signal events.
#[test]
fn degrade_by_extremes() {
    let p = "(pat/fast 8 (pat/pure 'x))";
    assert_steel_true(&format!(
        "(= 8 (length (pat/query (pat/degrade-by 7 0 {p}) (pat/span 0 1))))"
    ));
    assert_pinned(
        "()",
        &format!("(pin-events (pat/query (pat/degrade-by 7 1 {p}) (pat/span 0 1)))"),
    );
    assert_steel_true("(= 1 (length (pat/query (pat/degrade-by 7 0 pat/saw) (pat/span 0 1))))");
    assert_pinned(
        "()",
        "(pin-events (pat/query (pat/degrade-by 7 1 pat/saw) (pat/span 0 1)))",
    );
}

// Over 64 events, some are kept and some dropped. Each half of an event
// agrees with the whole event, and an event is kept exactly when
// `pat/rand` at its whole's midpoint is at least the probability.
#[test]
fn degrade_by_is_consistent() {
    assert_steel_true(
        "(define X (pat/degrade-by 7 1/2 (pat/fast 4 (pat/pure 'x))))
         (define (n a b) (length (pat/query X (pat/span a b))))
         (define (rand-at t)
           (pat/event-value (car (pat/query (pat/rand 7) (pat/span t t)))))
         (define (agree k)
           (if (>= k 64)
               #t
               (let ((a (/ k 4)) (m (+ (/ k 4) 1/8)) (b (/ (+ k 1) 4)))
                 (let ((kept (n a b)))
                   (if (= kept (n a m))
                       (if (= kept (n m b))
                           (if (= kept (if (>= (rand-at m) 1/2) 1 0))
                               (agree (+ k 1))
                               #f)
                           #f)
                       #f)))))
         (let ((kept (n 0 16)))
           (if (> kept 0) (if (< kept 64) (agree 0) #f) #f))",
    );
}
