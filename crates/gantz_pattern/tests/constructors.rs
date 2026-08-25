//! Constructor and query tests.

mod common;

use common::{assert_pinned, assert_steel_true};

// pure yields one event per cycle with whole equal to the cycle.
#[test]
fn pure_values_per_cycle() {
    assert_pinned(
        "((hello ((0 1) (1 1)) ((0 1) (1 1))) \
          (hello ((1 1) (2 1)) ((1 1) (2 1))) \
          (hello ((2 1) (3 1)) ((2 1) (3 1))))",
        "(pin-events (pat/query (pat/pure 'hello) (pat/span 0 3)))",
    );
}

// A partial trailing cycle keeps the full-cycle whole while the active
// is clipped to the query.
#[test]
fn pure_partial_cycle_whole() {
    assert_pinned(
        "((x ((0 1) (1 1)) ((0 1) (1 1))) \
          (x ((1 1) (2 1)) ((1 1) (2 1))) \
          (x ((2 1) (3 1)) ((2 1) (3 1))) \
          (x ((3 1) (7 2)) ((3 1) (4 1))))",
        "(pin-events (pat/query (pat/pure 'x) (pat/span 0 7/2)))",
    );
}

// A zero-width query yields nothing from a discrete pattern.
#[test]
fn pure_empty_span() {
    assert_pinned(
        "()",
        "(pin-events (pat/query (pat/pure 'x) (pat/span 1/2 1/2)))",
    );
}

// A signal yields exactly one whole-less event for any query, sampling
// the midpoint, including for a zero-width (instant) query.
#[test]
fn saw_samples_midpoint() {
    assert_pinned(
        "(((1 2) ((1 2) (1 2)) #f))",
        "(pin-events (pat/query pat/saw (pat/span 1/2 1/2)))",
    );
    // Midpoint of a wide query.
    assert_pinned(
        "(((1 2) ((0 1) (1 1)) #f))",
        "(pin-events (pat/query pat/saw (pat/span 0 1)))",
    );
}

// Negative saw phases wrap.
#[test]
fn saw_negative_phases_wrap() {
    let saw_value = |span: &str| {
        format!("(pin-value (pat/event-value (car (pat/query pat/saw (pat/span {span})))))")
    };
    assert_steel_true(&format!(
        "(equal? {} {})",
        saw_value("-1/2 -1/2"),
        saw_value("1/2 1/2"),
    ));
    assert_steel_true(&format!(
        "(equal? {} {})",
        saw_value("-3/4 -3/4"),
        saw_value("1/4 1/4"),
    ));
}

// saw2 is the polar saw: 0 at phase 1/2.
#[test]
fn saw2_polar() {
    assert_pinned(
        "(((0 1) ((1 2) (1 2)) #f))",
        "(pin-events (pat/query pat/saw2 (pat/span 1/2 1/2)))",
    );
    assert_pinned(
        "(((-1 2) ((1 4) (1 4)) #f))",
        "(pin-events (pat/query pat/saw2 (pat/span 1/4 1/4)))",
    );
}

// steady always yields its value, silence always yields nothing.
#[test]
fn steady_and_silence() {
    assert_steel_true(
        "(define (all-sevens n)
           (if (< n 0)
               #t
               (let ((es (pat/query (pat/steady 7) (pat/span (/ n 10) (/ n 10)))))
                 (if (= (length es) 1)
                     (if (= (pat/event-value (car es)) 7)
                         (all-sevens (- n 1))
                         #f)
                     #f))))
         (all-sevens 10)",
    );
    assert_pinned("()", "(pin-events (pat/query pat/silence (pat/span 0 10)))");
}

// query sorts events by active start (a deliberately reversed pattern).
#[test]
fn query_sorts_by_active_start() {
    assert_pinned(
        "((a ((0 1) (1 2)) #f) (b ((1 2) (1 1)) #f))",
        "(pin-events (pat/query
           (lambda (span)
             (list (pat/event 'b (pat/span 1/2 1) #f)
                   (pat/event 'a (pat/span 0 1/2) #f)))
           (pat/span 0 1)))",
    );
}
