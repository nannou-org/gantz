;; The gantz/sched module.
;;
;; A schedule is a list of `(seconds value)` pairs, each value with the time
;; in seconds at which it takes effect. `pat/events->secs` makes one from
;; the onsets of a pattern, and a control input such as a synth parameter
;; sets each value at its time.
;;
;; A gate is a schedule of 1.0 to open and 0.0 to close, for a control such
;; as the gate of an envelope. A control input accepts times later than the
;; tick that gives them, so a gate can close in a later tick.
;;
;; Written for the prelude-free base engine, as `gantz/pattern` is. Names
;; prefixed `sched//` are internal helpers and are not provided.

(require "gantz/list")

(provide sched/map
         sched/pick
         sched/filter
         sched/trig
         sched/gate
         sched/split)

;; A value on its way to a control input. A number becomes inexact. Any
;; other value passes through.
(define (sched//inexact v)
  (if (number? v) (exact->inexact v) v))

;; Apply `f` to the value of each pair of `schedule`. The times stay.
(define (sched/map f schedule)
  (if (if (function? f) (list? schedule) #f)
      (list/map (lambda (pr) (list (car pr) (f (car (cdr pr))))) schedule)
      '()))

;; Project the maps of `schedule` onto their `key`, for a control input. A
;; pair whose value holds no such key is dropped. Numbers leave as floats.
(define (sched/pick schedule key)
  (if (list? schedule)
      (list/flat-map
       (lambda (pr)
         (let ((m (car (cdr pr))))
           (if (if (hash? m) (hash-contains? m key) #f)
               (list (list (car pr) (sched//inexact (hash-ref m key))))
               '())))
       schedule)
      '()))

;; The pairs of `schedule` whose values satisfy `keep?`. The times stay.
(define (sched/filter keep? schedule)
  (if (if (function? keep?) (list? schedule) #f)
      (list/filter (lambda (pr) (keep? (car (cdr pr)))) schedule)
      '()))

;; The value under `key` of map `m` when it is a positive number, else #f.
(define (sched//secs-at m key)
  (if (if (hash? m) (hash-contains? m key) #f)
      (let ((s (hash-ref m key)))
        (if (number? s) (if (> s 0) (exact->inexact s) #f) #f))
      #f))

;; Whether gate pair `a` comes before gate pair `b`. A close comes before
;; an open at the same time, so the open starts a new gate.
(define (sched//gate-earlier? a b)
  (if (< (car a) (car b))
      #t
      (if (= (car a) (car b))
          (if (= (car (cdr a)) 0.0) (= (car (cdr b)) 1.0) #f)
          #f)))

;; The gate of each `(open close)` pair of seconds, in time order. A close
;; is dropped when an open that starts later comes before it, so an event
;; does not close the gate of a later event that overlaps it.
(define (sched//gates spans)
  (list/sort
   sched//gate-earlier?
   (list/flat-map
    (lambda (s)
      (let ((open (car s))
            (close (car (cdr s))))
        (if (list/any (lambda (o)
                        (let ((t (car o)))
                          (if (< open t) (< t close) #f)))
                      spans)
            (list (list open 1.0))
            (list (list open 1.0) (list close 0.0)))))
    spans)))

;; A gate pulse of `len` seconds at the time of each pair of `schedule`. It
;; opens at the time of the pair and closes `len` seconds later. A `len`
;; that is not a positive number gives no pulses.
(define (sched/trig schedule len)
  (if (if (list? schedule) (if (number? len) (> len 0) #f) #f)
      (let ((len (exact->inexact len)))
        (sched//gates
         (list/map (lambda (pr) (list (car pr) (+ (car pr) len))) schedule)))
      '()))

;; A gate for each map of `schedule` that holds a positive number of
;; seconds under `key`. It opens at the time of the pair and closes that
;; many seconds later. A pair without such a number gives no gate.
;;
;; A close is dropped when a later open of the same schedule comes before
;; it. A gate of a later tick is in another schedule, so a close of this
;; one can still close it.
(define (sched/gate schedule key)
  (if (list? schedule)
      (sched//gates
       (list/flat-map
        (lambda (pr)
          (let ((s (sched//secs-at (car (cdr pr)) key)))
            (if s (list (list (car pr) (+ (car pr) s))) '())))
        schedule))
      '()))

;; The value under `key` of map `m` when it is an integer, as an exact
;; integer, else #f.
(define (sched//index m key)
  (if (if (hash? m) (hash-contains? m key) #f)
      (let ((k (hash-ref m key)))
        (if (integer? k) (exact k) #f))
      #f))

;; `schedule` split into `n` schedules. A pair is in schedule `i` when the
;; integer under `key` of its map is `i` modulo `n`. A pair without such an
;; integer is in none.
(define (sched/split schedule key n)
  (list/map
   (lambda (i)
     (if (list? schedule)
         (list/filter
          (lambda (pr)
            (let ((k (sched//index (car (cdr pr)) key)))
              (if k (= (modulo k n) i) #f)))
          schedule)
         '()))
   (list/range 0 n)))
