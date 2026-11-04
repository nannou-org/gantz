;; The gantz/sched module.
;;
;; A schedule is a list of `(seconds value)` pairs, each value with the time
;; in seconds at which it takes effect. `pat/events->secs` makes one from
;; the onsets of a pattern, and a control input such as a synth parameter
;; sets each value at its time.
;;
;; Written for the prelude-free base engine, as `gantz/pattern` is. Names
;; prefixed `sched//` are internal helpers and are not provided.

(require "gantz/list")

(provide sched/pick)

;; A value on its way to a control input. A number becomes inexact. Any
;; other value passes through.
(define (sched//inexact v)
  (if (number? v) (exact->inexact v) v))

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
