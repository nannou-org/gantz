;; The gantz/pattern module: a tidalcycles-inspired pattern vocabulary.
;;
;; A pattern is a function from a span to the list of events occurring
;; along it. Spans are pairs of exact rational time points measured in
;; cycles. Events carry an `active` span (where the value applies within
;; the query), a `whole` span (the event's full structure, #f for
;; continuous signals) and a `value`.
;;
;; Semantics follow the cycles crate (the reference implementation whose
;; tests are ported into this crate's test suite).
;;
;; Written for the prelude-free base engine: primitive special forms only
;; (no `and`, `or`, `cond`, `when`) and no prelude list fns (`map`,
;; `filter`, `sort` and friends are hand-rolled below, tail-recursively).
;; Names prefixed `pat//` are internal helpers and are not provided.

(provide pat/span
         pat/span-start
         pat/span-end
         pat/span-len
         pat/span-map
         pat/span-cycles
         pat/span-intersect
         pat/event
         pat/event-value
         pat/event-active
         pat/event-whole
         pat/event-whole-or-active
         pat/event-map-value
         pat/event-map-spans
         pat/pure
         pat/silence
         pat/signal
         pat/steady
         pat/saw
         pat/saw2
         pat/query)

;; -- internal helpers ---------------------------------------------------------

(define (pat//max2 a b) (if (< a b) b a))
(define (pat//min2 a b) (if (< b a) b a))

;; Tail-recursive list helpers (the prelude's are unavailable on the base
;; engine).

(define (pat//rev-append xs acc)
  (if (empty? xs) acc (pat//rev-append (cdr xs) (cons (car xs) acc))))

(define (pat//map f xs)
  (pat//map-loop f xs '()))

(define (pat//map-loop f xs acc)
  (if (empty? xs)
      (reverse acc)
      (pat//map-loop f (cdr xs) (cons (f (car xs)) acc))))

(define (pat//filter keep? xs)
  (pat//filter-loop keep? xs '()))

(define (pat//filter-loop keep? xs acc)
  (if (empty? xs)
      (reverse acc)
      (pat//filter-loop keep?
                        (cdr xs)
                        (if (keep? (car xs)) (cons (car xs) acc) acc))))

;; Map `f` over `xs` and concatenate the resulting lists, preserving order.
(define (pat//flat-map f xs)
  (pat//flat-map-loop f xs '()))

(define (pat//flat-map-loop f xs acc)
  (if (empty? xs)
      (reverse acc)
      (pat//flat-map-loop f (cdr xs) (pat//rev-append (f (car xs)) acc))))

;; A stable merge sort (`sort` on the base engine rejects closures).
;; `less?` must be a strict order. Merge recursion depth is bounded by the
;; list length, fine at event-list scale.
(define (pat//sort less? xs)
  (let ((n (length xs)))
    (if (< n 2)
        xs
        (let ((mid (exact (floor (/ n 2)))))
          (pat//merge less?
                      (pat//sort less? (take xs mid))
                      (pat//sort less? (list-tail xs mid)))))))

(define (pat//merge less? a b)
  (if (empty? a)
      b
      (if (empty? b)
          a
          (if (less? (car b) (car a))
              (cons (car b) (pat//merge less? a (cdr b)))
              (cons (car a) (pat//merge less? (cdr a) b))))))

;; Order events by the start of their active spans.
(define (pat//event-earlier? a b)
  (< (car (pat/event-active a)) (car (pat/event-active b))))

;; -- spans --------------------------------------------------------------------

;; A span over `[start, end)`, in cycles.
(define (pat/span start end) (cons start end))

(define (pat/span-start s) (car s))

(define (pat/span-end s) (cdr s))

(define (pat/span-len s) (- (cdr s) (car s)))

;; Map both end points of the span with `f`.
(define (pat/span-map f s) (cons (f (car s)) (f (cdr s))))

;; Split the span into a list of sub-spans at whole-cycle boundaries.
;;
;; An empty or negative span yields the empty list.
(define (pat/span-cycles s)
  (pat//span-cycles-loop (car s) (cdr s) '()))

(define (pat//span-cycles-loop start end acc)
  (if (>= start end)
      (reverse acc)
      (if (>= start (floor end))
          (reverse (cons (cons start end) acc))
          (let ((this-end (+ (floor start) 1)))
            (pat//span-cycles-loop this-end end (cons (cons start this-end) acc))))))

;; The intersecting span between `a` and `b`, or #f when the intersection
;; is empty (including the degenerate zero-length case).
(define (pat/span-intersect a b)
  (let ((start (pat//max2 (car a) (car b)))
        (end (pat//min2 (cdr a) (cdr b))))
    (if (<= end start) #f (cons start end))))

;; -- events -------------------------------------------------------------------

;; An event: `value` over the `active` span, with `whole` carrying the
;; event's full structure (#f for continuous signals).
(define (pat/event value active whole)
  (hash 'value value 'active active 'whole whole))

(define (pat/event-value e) (hash-ref e 'value))

(define (pat/event-active e) (hash-ref e 'active))

(define (pat/event-whole e) (hash-ref e 'whole))

;; The event's whole when present, otherwise its active span.
(define (pat/event-whole-or-active e)
  (let ((w (hash-ref e 'whole)))
    (if w w (hash-ref e 'active))))

;; Map the event's value with `f`.
(define (pat/event-map-value f e)
  (pat/event (f (hash-ref e 'value)) (hash-ref e 'active) (hash-ref e 'whole)))

;; Map the event's active span and (when present) whole span with `f`.
(define (pat/event-map-spans f e)
  (pat/event (hash-ref e 'value)
             (f (hash-ref e 'active))
             (let ((w (hash-ref e 'whole)))
               (if w (f w) #f))))

;; -- constructors -------------------------------------------------------------
;;
;; A pattern is `(lambda (span) <list of events>)`. Combinators make no
;; ordering guarantee on the returned events. [`pat/query`] sorts.

;; Repeats the given value once per cycle (cycles' `atom`).
(define (pat/pure v)
  (lambda (span)
    (pat//map (lambda (cyc)
                (let ((start (floor (car cyc))))
                  (pat/event v cyc (cons start (+ start 1)))))
              (pat/span-cycles span))))

;; The pattern producing no events.
(define pat/silence (lambda (span) '()))

;; A continuous pattern sampling `sample` at the query span's midpoint.
;; Signal events carry no `whole`, and a signal yields exactly one event
;; for any query, including a zero-width one.
(define (pat/signal sample)
  (lambda (span)
    (let ((mid (+ (car span) (/ (- (cdr span) (car span)) 2))))
      (list (pat/event (sample mid) span #f)))))

;; A continuous pattern of a constant value.
(define (pat/steady v)
  (pat/signal (lambda (r) v)))

;; A signal ramping 0 to 1 over every cycle.
(define pat/saw
  (pat/signal (lambda (r) (- r (floor r)))))

;; A signal ramping -1 to 1 over every cycle (polar [`pat/saw`]).
(define pat/saw2
  (pat/signal (lambda (r) (- (* 2 (- r (floor r))) 1))))

;; Query the pattern over the span, events sorted by active-span start.
(define (pat/query p span)
  (pat//sort pat//event-earlier? (p span)))
