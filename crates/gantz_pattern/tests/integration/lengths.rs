//! Tests of the length combinators `pat/map-len` and `pat/scale-len`, and of
//! `pat/segment`.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};

/// Steel helpers for the split invariance checks. `split-same?` checks that
/// each half of a span queries to the events of the whole span, clipped to
/// the half.
const SPLIT: &str = r#"
(define (ev< a b)
  (let ((x (car (pat/event-active a)))
        (y (car (pat/event-active b)))
        (wa (pat/event-whole a))
        (wb (pat/event-whole b)))
    (if (< x y)
        #t
        (if (= x y)
            (if (< (car wa) (car wb)) #t (if (= (car wa) (car wb)) (< (cdr wa) (cdr wb)) #f))
            #f))))
(define (clip es s)
  (if (empty? es)
      '()
      (let ((a (pat/span-intersect (pat/event-active (car es)) s)))
        (if a
            (cons (pat/event (pat/event-value (car es)) a (pat/event-whole (car es)))
                  (clip (cdr es) s))
            (clip (cdr es) s)))))
(define (insert e es)
  (if (empty? es) (list e) (if (ev< (car es) e) (cons (car es) (insert e (cdr es))) (cons e es))))
(define (sort-evs es) (if (empty? es) '() (insert (car es) (sort-evs (cdr es)))))
(define (half-same? p full h)
  (equal? (pin-events (sort-evs (clip (pat/query p full) h)))
          (pin-events (sort-evs (pat/query p h)))))
(define (split-same? p s m)
  (if (half-same? p s (cons (car s) m)) (half-same? p s (cons m (cdr s))) #f))
"#;

const AB: &str = "(pat/fastcat (list (pat/pure 'a) (pat/pure 'b)))";

#[test]
fn scale_len() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, String)] = &[
        // Starts stay and lengths halve.
        (
            "shrinks",
            "((a ((0 1) (1 4)) ((0 1) (1 4))) (b ((1 2) (3 4)) ((1 2) (3 4))))",
            format!("(pin-events (pat/query (pat/scale-len 1/2 {AB}) (pat/span 0 1)))"),
        ),
        // A float factor snaps to the grid, so the spans stay exact.
        (
            "float_factor_is_exact",
            "((a ((0 1) (1 4)) ((0 1) (1 4))) (b ((1 2) (3 4)) ((1 2) (3 4))))",
            format!("(pin-events (pat/query (pat/scale-len 0.5 {AB}) (pat/span 0 1)))"),
        ),
        // A grown event reaches past the query, and the `b` of the cycle
        // before reaches into it.
        (
            "grows_into_the_query",
            "((a ((0 1) (3 4)) ((0 1) (3 4))) \
              (b ((0 1) (1 4)) ((-1 2) (1 4))) \
              (b ((1 2) (1 1)) ((1 2) (5 4))))",
            format!("(pin-events (pat/query (pat/scale-len 3/2 {AB}) (pat/span 0 1)))"),
        ),
        // A pattern factor is the factor at the start of each whole. Where it
        // is not a number, the event keeps its length.
        (
            "pattern_factor_at_the_start",
            "((a ((0 1) (1 8)) ((0 1) (1 8))) \
              (a ((1 4) (3 8)) ((1 4) (3 8))) \
              (a ((1 2) (3 4)) ((1 2) (3 4))) \
              (a ((3 4) (1 1)) ((3 4) (1 1))))",
            "(pin-events (pat/query
               (pat/scale-len (pat/fastcat (list (pat/pure 1/2) (pat/pure 'x)))
                              (pat/fast 4 (pat/pure 'a)))
               (pat/span 0 1)))"
                .into(),
        ),
        // A factor that is not positive drops every event.
        (
            "zero_factor_drops",
            "()",
            format!("(pin-events (pat/query (pat/scale-len 0 {AB}) (pat/span 0 1)))"),
        ),
        // A signal has no whole, so it passes through.
        (
            "signal_passes",
            "(((1 2) ((0 1) (1 1)) #f))",
            "(pin-events (pat/query (pat/scale-len 1/2 pat/saw) (pat/span 0 1)))".into(),
        ),
        // A factor that is neither a number nor a pattern keeps every length.
        (
            "junk_factor_keeps_lengths",
            "((a ((0 1) (1 2)) ((0 1) (1 2))) (b ((1 2) (1 1)) ((1 2) (1 1))))",
            format!("(pin-events (pat/query (pat/scale-len '() {AB}) (pat/span 0 1)))"),
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

#[test]
fn map_len() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, String)] = &[
        // A constant function gives fixed lengths.
        (
            "constant_length",
            "((a ((0 1) (1 8)) ((0 1) (1 8))) (b ((1 2) (5 8)) ((1 2) (5 8))))",
            format!("(pin-events (pat/query (pat/map-len (lambda (l) 1/8) {AB}) (pat/span 0 1)))"),
        ),
        // The function sees the length in cycles, here to cap it.
        (
            "caps_lengths",
            "((a ((0 1) (1 4)) ((0 1) (1 4))) (b ((1 2) (3 4)) ((1 2) (3 4))))",
            format!(
                "(pin-events (pat/query (pat/map-len (lambda (l) (if (< l 1/4) l 1/4)) {AB}) \
                   (pat/span 0 1)))"
            ),
        ),
        // A length that is not a positive number drops the event.
        (
            "bad_lengths_drop",
            "()",
            format!(
                "(pin-events (pat/query (pat/map-len (lambda (l) (if (< l 1) 0 'x)) {AB}) \
                   (pat/span 0 1)))"
            ),
        ),
        (
            "non_fn_is_silent",
            "()",
            format!("(pin-events (pat/query (pat/map-len 'nope {AB}) (pat/span 0 1)))"),
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

// A span and its halves give the same events, for lengths that shrink and
// for lengths that grow within the lookback.
#[test]
fn lengths_are_split_invariant() {
    let mut vm = new_pin_engine();
    vm.run(SPLIT.to_string()).expect("split helpers");
    let base = "(pat/fastcat (list (pat/pure 1) (pat/slow 2 (pat/pure 2)) (pat/pure 3)))";
    let cases = [
        format!("(pat/scale-len 1/3 {base})"),
        format!("(pat/scale-len 5/2 {base})"),
        format!("(pat/scale-len (pat/fastcat (list (pat/pure 1/2) (pat/pure 2))) {base})"),
        format!("(pat/map-len (lambda (l) (+ l 1/2)) {base})"),
    ];
    for p in &cases {
        for (s, m) in [
            ("(cons 0 1)", "1/2"),
            ("(cons 1/3 2)", "5/4"),
            ("(cons 0 3)", "7/6"),
        ] {
            assert_steel_true(
                &mut vm,
                &format!("split of {p} over {s} at {m}"),
                &format!("(split-same? {p} {s} {m})"),
            );
        }
    }
}

#[test]
fn segment() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str, &str)] = &[
        // Each quarter takes the value of the signal at its middle.
        (
            "samples_a_signal",
            "(((1 8) ((0 1) (1 4)) ((0 1) (1 4))) \
              ((3 8) ((1 4) (1 2)) ((1 4) (1 2))) \
              ((5 8) ((1 2) (3 4)) ((1 2) (3 4))) \
              ((7 8) ((3 4) (1 1)) ((3 4) (1 1))))",
            "(pin-events (pat/query (pat/segment 4 pat/saw) (pat/span 0 1)))",
        ),
        // The value holds however a query chops the event.
        (
            "holds_through_a_chop",
            "(((1 8) ((1 8) (1 4)) ((0 1) (1 4))))",
            "(pin-events (pat/query (pat/segment 4 pat/saw) (pat/span 1/8 1/4)))",
        ),
        // A float count snaps to the grid.
        (
            "float_count",
            "(((1 4) ((0 1) (1 2)) ((0 1) (1 2))) ((3 4) ((1 2) (1 1)) ((1 2) (1 1))))",
            "(pin-events (pat/query (pat/segment 2.0 pat/saw) (pat/span 0 1)))",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
    for n in ["0", "-1", "'x", "'()"] {
        assert_pinned(
            &mut vm,
            &format!("segment of {n} is silent"),
            "()",
            &format!("(pin-events (pat/query (pat/segment {n} pat/saw) (pat/span 0 1)))"),
        );
    }
}
