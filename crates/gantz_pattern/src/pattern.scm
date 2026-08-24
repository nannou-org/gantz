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
         pat/event-map-spans)

;; -- internal helpers ---------------------------------------------------------

(define (pat//max2 a b) (if (< a b) b a))
(define (pat//min2 a b) (if (< b a) b a))

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
