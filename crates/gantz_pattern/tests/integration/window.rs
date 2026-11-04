//! Clock and delivery-helper tests.

use crate::common::{assert_pinned, assert_steel_true, new_pin_engine};
use gantz_core::steel::steel_vm::engine::Engine;

/// Steel helpers that drive the clock over a list of ticks. A tick is a
/// `(list time dt cps)`, with `'()` as `dt` for a partial eval.
const PLAY: &str = r#"
(require "gantz/list")

(define (pin-window r)
  (list (pin-span (car r))
        (list-ref r 1)
        (pin-map pin-seg (list-ref r 2))
        (list-ref r 3)
        (pin-state (list-ref r 4))))
(define (pin-seg s)
  (list (pin-num (car s)) (pin-num (list-ref s 1)) (list-ref s 2)))
(define (pin-state s)
  (list (pin-num (car s)) (list-ref s 1) (list-ref s 2)))

;; The `(seconds value)` onset pairs of `p` over `ticks`, in delivery order.
(define (play p ticks)
  (reverse (play-loop p ticks void '())))

(define (play-loop p ticks st acc)
  (if (empty? ticks)
      acc
      (let ((tick (car ticks)))
        (let ((r (pat/window st (car tick) (list-ref tick 1) (list-ref tick 2))))
          (play-loop p (cdr ticks) (list-ref r 4)
                     (list/fold (lambda (a pr) (cons pr a))
                                acc
                                (pat/events->secs (pat/query p (car r))
                                                  (list-ref r 1)
                                                  (list-ref r 2))))))))

;; Whether the spans of the ticks abut.
(define (abut? ticks)
  (abut-loop ticks void #f))

(define (abut-loop ticks st end)
  (if (empty? ticks)
      #t
      (let ((tick (car ticks)))
        (let ((r (pat/window st (car tick) (list-ref tick 1) (list-ref tick 2))))
          (if (if end (= end (car (car r))) #t)
              (abut-loop (cdr ticks) (list-ref r 4) (cdr (car r)))
              #f)))))

;; `n` ticks from `t` at `hz`, with the cps `(cps-at time)`. `jitter`
;; moves each tick by up to that many seconds, from a fixed sequence.
(define (ticks-from t n hz jitter cps-at)
  (ticks-loop t 0 n hz jitter cps-at '()))

(define (ticks-loop t i n hz jitter cps-at acc)
  (if (>= i n)
      (reverse acc)
      (ticks-loop (+ (+ t (/ 1.0 hz))
                     (* jitter (/ (exact->inexact (- (modulo (* i 7919) 13) 6)) 6.0)))
                  (+ i 1) n hz jitter cps-at
                  (cons (list t (/ 1.0 hz) (cps-at t)) acc))))

;; Whether each time sits within 1 ns of its expected time. Extra onsets
;; past the end of `want` are ignored, as ticks can overrun the score.
(define (times-close? got want)
  (if (empty? want)
      #t
      (if (empty? got)
          #f
          (if (< (abs (- (car (car got)) (car want))) 0.000000001)
              (times-close? (cdr got) (cdr want))
              #f))))

;; Whether the onset times never go backwards.
(define (ordered? onsets)
  (if (empty? onsets)
      #t
      (if (empty? (cdr onsets))
          #t
          (if (<= (car (car onsets)) (car (car (cdr onsets))))
              (ordered? (cdr onsets))
              #f))))

;; One onset per quarter cycle.
(define fours (pat/fast 4 (pat/pure 1)))
;; A cps of 1/2, 1 then 2, stepping per cycle.
(define stepped (pat/slowcat (list (pat/pure 1/2) (pat/pure 1) (pat/pure 2))))
"#;

fn new_play_engine() -> Engine {
    let mut vm = new_pin_engine();
    vm.run(PLAY.to_string()).expect("play preamble");
    vm
}

/// Each row pins `(span secs segs cps new-state)`.
#[test]
fn window_steps() {
    let mut vm = new_play_engine();
    let rows: &[(&str, &str, &str)] = &[
        // The first tick starts the playhead at cycle 0 on the eval time,
        // at a cps of 1. Cycle 0 therefore lands in the first window.
        (
            "first_tick_starts_at_cycle_zero",
            "(((0 1) (1 2)) 10.0 (((0 1) (1 2) 1.0)) 1.0 ((1 2) 10.5 1.0))",
            "(pin-window (pat/window void 10.0 0.5 1))",
        ),
        // A span closes the gap to one tick ahead, at the last cps.
        (
            "span_covers_up_to_one_tick_ahead",
            "(((1 1) (2 1)) 1.0 (((1 1) (2 1) 2.0)) 2.0 ((2 1) 1.5 2.0))",
            "(pin-window (pat/window (list 1 1.0 2.0 0.0) 1.0 0.5 2))",
        ),
        // The cps sets the seconds of the span, however long it is. The
        // new cps takes effect within the span and sets the next one.
        (
            "cps_sets_the_span_seconds",
            "(((0 1) (1 2)) 10.0 (((0 1) (1 2) 2.0)) 2.0 ((1 2) 10.25 2.0))",
            "(pin-window (pat/window void 10.0 0.5 2))",
        ),
        // A span that overshoots leaves the clock ahead of the eval time,
        // which then holds until it catches up.
        (
            "clock_ahead_holds",
            "(((1 1) (1 1)) 5.0 () 1.0 ((1 1) 5.0 1.0))",
            "(pin-window (pat/window (list 1 5.0 1.0 0.0) 1.0 1.0 1))",
        ),
        // A cps step inside a span splits it, so seconds stay exact.
        (
            "steps_split_the_span",
            "(((0 1) (1 1)) 0.0 (((0 1) (1 2) 1.0) ((1 2) (1 1) 2.0)) 1.0 ((1 1) 0.75 2.0))",
            "(pin-window (pat/window (list 0 0.0 1.0 0.0) 0.0 1.0
                           (pat/fastcat (list (pat/pure 1) (pat/pure 2)))))",
        ),
        // A span that crosses a cycle boundary queries each cycle.
        (
            "spans_split_at_cycle_boundaries",
            "(((1 2) (3 2)) 0.0 (((1 2) (1 1) 1.0) ((1 1) (3 2) 2.0)) 1.0 ((3 2) 0.75 2.0))",
            "(pin-window (pat/window (list 1/2 0.0 1.0 0.0) 0.0 1.0
                           (pat/slowcat (list (pat/pure 1) (pat/pure 2)))))",
        ),
        // A gap in the cps holds the last cps.
        (
            "gaps_hold_the_last_rate",
            "(((0 1) (1 1)) 0.0 (((0 1) (1 2) 4.0) ((1 2) (1 1) 2.0)) 4.0 ((1 1) 0.375 2.0))",
            "(pin-window (pat/window (list 0 0.0 4.0 0.0) 0.0 0.25
                           (pat/fastcat (list pat/silence (pat/pure 2)))))",
        ),
        // Only a positive number sets a cps. Any other value is a gap, a
        // map with a cps key included.
        (
            "map_holds",
            "(((0 1) (1 1)) 0.0 (((0 1) (1 1) 1.0)) 1.0 ((1 1) 1.0 1.0))",
            "(pin-window (pat/window (list 0 0.0 1.0 0.0) 0.0 1.0
                           (pat/pure (hash 'cps 2 'freq 440))))",
        ),
        (
            "zero_cps_holds",
            "(((0 1) (1 1)) 0.0 (((0 1) (1 1) 1.0)) 1.0 ((1 1) 1.0 1.0))",
            "(pin-window (pat/window (list 0 0.0 1.0 0.0) 0.0 1.0 (pat/pure 0)))",
        ),
        (
            "negative_cps_holds",
            "(((0 1) (1 1)) 0.0 (((0 1) (1 1) 1.0)) 1.0 ((1 1) 1.0 1.0))",
            "(pin-window (pat/window (list 0 0.0 1.0 0.0) 0.0 1.0 -2))",
        ),
        (
            "symbol_value_holds",
            "(((0 1) (1 1)) 0.0 (((0 1) (1 1) 1.0)) 1.0 ((1 1) 1.0 1.0))",
            "(pin-window (pat/window (list 0 0.0 1.0 0.0) 0.0 1.0 (pat/pure 'fast)))",
        ),
        // Where cps events overlap, a later one starts when the earlier
        // one ends.
        (
            "overlap_earliest_wins",
            "(((0 1) (1 1)) 0.0 (((0 1) (1 1) 2.0)) 2.0 ((1 1) 0.5 2.0))",
            "(pin-window (pat/window (list 0 0.0 1.0 0.0) 0.0 1.0
                           (pat/stack (list (pat/pure 2) (pat/pure 4)))))",
        ),
        // A gap within the cap closes in one span.
        (
            "gap_within_cap_passes",
            "(((0 1) (4 1)) 0.0 (((0 1) (1 1) 1.0) ((1 1) (2 1) 1.0) ((2 1) (3 1) 1.0) ((3 1) (4 1) 1.0)) 1.0 ((4 1) 4.0 1.0))",
            "(pin-window (pat/window (list 0 0.0 1.0 0.0) 3.5 0.5 1))",
        ),
        // A gap beyond the cap moves the clock rather than the playhead.
        // So does eval time that runs backwards, as restored state from an
        // older session does. Each resumes with a one-tick span.
        (
            "stall_moves_the_clock",
            "(((1 1) (3 2)) 100.0 (((1 1) (3 2) 1.0)) 1.0 ((3 2) 100.5 1.0))",
            "(pin-window (pat/window (list 1 1.0 1.0 0.0) 100.0 0.5 1))",
        ),
        (
            "restored_clock_moves",
            "(((1 1) (3 2)) 1.0 (((1 1) (3 2) 1.0)) 1.0 ((3 2) 1.5 1.0))",
            "(pin-window (pat/window (list 1 500.0 1.0 500.0) 1.0 0.5 1))",
        ),
        // A tick whose span snaps to nothing leaves the state alone, so
        // the gap keeps growing until the span reaches a grid step.
        (
            "tiny_gap_holds",
            "(((0 1) (0 1)) 0.0 () 0.001 ((0 1) 0.0 0.001))",
            "(pin-window (pat/window (list 0 0.0 0.001 0.0) 0.0 0.01 1))",
        ),
        // Window boundaries snap to the 1/1920 grid, so the float closest
        // to 1/3 lands on exactly 1/3 and denominators stay bounded.
        (
            "grid_snaps_thirds_exactly",
            "(((0 1) (1 3)) 0.0 (((0 1) (1 3) 1.0)) 1.0 ((1 3) 0.3333333333333333 1.0))",
            "(pin-window (pat/window (list 0 0.0 1.0 0.0) 0.0 0.3333333333333333 1))",
        ),
    ];
    for (case, expected, expr) in rows {
        assert_pinned(&mut vm, case, expected, expr);
    }
}

/// Onset times over many ticks stay on their exact times.
#[test]
fn clock_scenarios() {
    let mut vm = new_play_engine();
    let rows: &[(&str, &str)] = &[
        (
            "spans_abut_under_a_changing_cps",
            "(abut? (ticks-from 1.0 600 60.0 0.0 (lambda (t) stepped)))",
        ),
        // Cycles take 2, 1 and 1/2 seconds.
        (
            "cps_steps_are_exact",
            "(times-close?
               (play fours (ticks-from 1.0 240 60.0 0.0 (lambda (t) stepped)))
               (list 1.0 1.5 2.0 2.5 3.0 3.25 3.5 3.75 4.0 4.125 4.25 4.375 4.5))",
        ),
        // A cps that steps is exact at any tick rate.
        (
            "cps_steps_are_exact_at_a_slow_tick",
            "(times-close?
               (play fours (ticks-from 1.0 28 7.0 0.0 (lambda (t) stepped)))
               (list 1.0 1.5 2.0 2.5 3.0 3.25 3.5 3.75 4.0 4.125 4.25 4.375 4.5))",
        ),
        // Tick timing does not reach the event times.
        (
            "jitter_does_not_reach_onsets",
            "(times-close?
               (play fours (ticks-from 1.0 300 60.0 0.008 (lambda (t) 1/2)))
               (list 1.0 1.5 2.0 2.5 3.0 3.5 4.0 4.5))",
        ),
        // A window that lasts for many ticks does not snap its events to
        // the grid. 1/7 of a cycle has no grid point.
        (
            "long_cycles_keep_their_subdivisions",
            "(times-close?
               (play (pat/fast 7 (pat/pure 1))
                     (ticks-from 1.0 1300 60.0 0.0 (lambda (t) 1/20)))
               (list 1.0 3.857142857142857 6.714285714285714 9.571428571428571))",
        ),
        // A cps read out of map events applies across the rests between
        // them, as in `demo.pat.maps`. A cycle holds 1 cps for its first
        // half and 2 cps for its second half.
        (
            "cps_of_events_holds_across_rests",
            "(let ((p (pat/appl (pat/map (lambda (f) (hash 'freq f))
                                         (pat/fast 2 (pat/fastcat (list (pat/pure 220)
                                                                        pat/silence))))
                                (pat/map (lambda (c) (lambda (m) (hash-insert m 'cps c)))
                                         (pat/fastcat (list (pat/pure 1) (pat/pure 2)))))))
               (let ((cps (pat/map (lambda (m) (hash-ref m 'cps)) p)))
                 (times-close?
                   (play p (ticks-from 1.0 150 60.0 0.0 (lambda (t) cps)))
                   (list 1.0 1.5 1.75 2.25 2.5 3.0))))",
        ),
        // A cps change neither jumps nor replays. Every onset stays in
        // order, and the 9 s at 1/2 cps and 11 s at 3/5 cps give 45 onsets.
        (
            "dial_up_keeps_order",
            "(let ((cps-at (lambda (t) (if (< t 10.0) 1/2 3/5))))
               (let ((onsets (play fours (ticks-from 1.0 1200 60.0 0.0 cps-at))))
                 (if (ordered? onsets) (= 45 (length onsets)) #f)))",
        ),
        (
            "dial_down_keeps_order",
            "(let ((cps-at (lambda (t) (if (< t 10.0) 3/5 1/2))))
               (let ((onsets (play fours (ticks-from 1.0 1200 60.0 0.0 cps-at))))
                 (ordered? onsets)))",
        ),
        // The dial's push evaluates the clock with no tick. The clock
        // holds, so the onsets stay the same.
        (
            "partial_evals_lose_nothing",
            "(let ((ticks (ticks-from 1.0 120 60.0 0.0 (lambda (t) 1/2))))
               (times-close?
                 (play fours (list/flat-map
                               (lambda (tick) (list (list (car tick) '() 1/2) tick))
                               ticks))
                 (list 1.0 1.5 2.0 2.5)))",
        ),
        // A stall skips the time without losing a cycle.
        (
            "stall_keeps_order_and_position",
            "(let ((ticks (append (ticks-from 1.0 120 60.0 0.0 (lambda (t) 1/2))
                                  (ticks-from 40.0 120 60.0 0.0 (lambda (t) 1/2)))))
               (if (abut? ticks)
                   (let ((onsets (play fours ticks)))
                     (if (ordered? onsets) (= 8 (length onsets)) #f))
                   #f))",
        ),
    ];
    for (case, snippet) in rows {
        assert_steel_true(&mut vm, case, snippet);
    }
}

#[test]
fn delivery() {
    let mut vm = new_pin_engine();
    let rows: &[(&str, &str)] = &[
        // events->secs spaces events by their exact cycle offsets from
        // the start of the window, keeps only onsets, and emits floats only.
        (
            "events_to_secs_spaces_onsets",
            "(equal? (list (list 10.0 #t) (list 10.1875 #t) (list 10.375 #t))
                     (pat/events->secs (pat/query (pat/euclid 3 8) (pat/span 0 1))
                                       10.0
                                       (list (list 0 1 2.0))))",
        ),
        // An onset past a cps step takes the seconds of each part.
        (
            "events_to_secs_walks_the_rates",
            "(equal? (list (list 1.0 #t) (list 1.5 #t) (list 1.625 #t) (list 1.75 #t))
                     (pat/events->secs (pat/query (pat/euclid 4 4) (pat/span 0 1))
                                       1.0
                                       (list (list 0 1/4 1/2) (list 1/4 1 2.0))))",
        ),
        // Numeric values leave as floats.
        (
            "events_to_secs_numbers_leave_as_floats",
            "(equal? (list (list 5.0 0.25))
                     (pat/events->secs (pat/query (pat/pure 1/4) (pat/span 0 1))
                                       5.0
                                       (list (list 0 1 1.0))))",
        ),
        // Window-chopped continuations and signal events are filtered out
        // of delivery.
        (
            "onset_is_delivered",
            "(pat/event-onset? (pat/event 'x (pat/span 0 1/2) (pat/span 0 1)))",
        ),
        (
            "continuation_is_not_an_onset",
            "(equal? #f (pat/event-onset? (pat/event 'x (pat/span 1/2 1) (pat/span 0 1))))",
        ),
        (
            "signal_event_is_not_an_onset",
            "(equal? #f (pat/event-onset? (pat/event 'x (pat/span 0 1/2) #f)))",
        ),
    ];
    for (case, snippet) in rows {
        assert_steel_true(&mut vm, case, snippet);
    }
    assert_pinned(
        &mut vm,
        "non_onsets_are_not_delivered",
        "()",
        "(pat/events->secs (list (pat/event 'x (pat/span 1/2 1) (pat/span 0 1))
                                 (pat/event 'y (pat/span 0 1/2) #f))
                           0.0
                           (list (list 0 1 1.0)))",
    );
}
