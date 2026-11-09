;; The gantz/pattern module.
;;
;; A pattern is a function from a span to the list of events occurring
;; along it. Spans are pairs of exact rational time points measured in
;; cycles. Events carry a `value`, an `active` span where the value
;; applies within the query, and a `whole` span carrying the event's
;; full structure. Whole is #f for continuous signals.
;;
;; Written for the prelude-free base engine. It uses primitive special
;; forms only and takes its list fns from `gantz/list`. Names
;; prefixed `pat//` are internal helpers and are not provided.

(require "gantz/list")
(require "gantz/rng")

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
         pat/indices
         pat/silence
         pat/signal
         pat/steady
         pat/saw
         pat/saw2
         pat/rand
         pat/query
         pat/rationalize
         pat/fast
         pat/slow
         pat/shift
         pat/slowcat
         pat/fastcat
         pat/timecat
         pat/stack
         pat/fit-span
         pat/fit-cycle
         pat/cache
         pat/map
         pat/map-events
         pat/filter
         pat/filter-events
         pat/map-len
         pat/scale-len
         pat/segment
         pat/degrade-by
         pat/join
         pat/inner-join
         pat/outer-join
         pat/app
         pat/appl
         pat/appr
         pat/merge-with
         pat/merge-withl
         pat/merge-withr
         pat/key
         pat/get
         pat/with
         pat/euclid-bools
         pat/euclid
         pat/euclid-off
         pat/euclid-full
         pat/event-onset?
         pat/event-phase
         pat/window
         pat/events->secs
         pat/at
         pat/nudge
         pat/secs
         pat/len-secs
         pat/euclid-with
         pat/as-span
         pat/plot-data)

(define (pat//max2 a b) (if (< a b) b a))
(define (pat//min2 a b) (if (< b a) b a))

(define (pat//rev-append xs acc)
  (if (empty? xs) acc (pat//rev-append (cdr xs) (cons (car xs) acc))))

;; Order events by the start of their active spans.
(define (pat//event-earlier? a b)
  (< (car (pat/event-active a)) (car (pat/event-active b))))

;; Query `p` when it is a pattern. Otherwise return no events. Partial
;; graph evals can hand a combinator a non-pattern in place of an unfired
;; pattern input. That input must be silent rather than an application
;; error.
(define (pat//events p span)
  (if (function? p) (p span) '()))

;; A span over `[start, end)`, in cycles.
(define (pat/span start end) (cons start end))

(define (pat/span-start s) (car s))

(define (pat/span-end s) (cdr s))

(define (pat/span-len s) (- (cdr s) (car s)))

;; The midpoint of the span.
(define (pat//span-mid s) (+ (car s) (/ (- (cdr s) (car s)) 2)))

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
;; is empty or zero-length.
(define (pat/span-intersect a b)
  (let ((start (pat//max2 (car a) (car b)))
        (end (pat//min2 (cdr a) (cdr b))))
    (if (<= end start) #f (cons start end))))

;; The struct printer references `display`, which is prelude-only. Only
;; the scheme display path invokes it. The Rust fmt that gantz renders
;; values with never does, so a shim satisfies the struct expansion.
(define (display . args) void)

;; An event holds a `value`, its `active` span and its `whole` span.
;; Whole is #f for continuous signals. Transparent, so events print
;; deterministically as `(event value active whole)`.
(struct event (value active whole) #:transparent)

(define (pat/event value active whole) (event value active whole))

(define (pat/event-value e) (event-value e))

(define (pat/event-active e) (event-active e))

(define (pat/event-whole e) (event-whole e))

;; The event's whole when present, otherwise its active span.
(define (pat/event-whole-or-active e)
  (let ((w (event-whole e)))
    (if w w (event-active e))))

;; Map the event's value with `f`.
(define (pat/event-map-value f e)
  (event (f (event-value e)) (event-active e) (event-whole e)))

;; Map the event's active span with `f`. Map its whole span too when present.
(define (pat/event-map-spans f e)
  (event (event-value e)
         (f (event-active e))
         (let ((w (event-whole e)))
           (if w (f w) #f))))

;; A pattern is `(lambda (span) <list of events>)`. Combinators make no
;; ordering guarantee on the returned events. [`pat/query`] sorts.

;; One event per cycle of the span. Its value is `(f index)`, where
;; `index` is the cycle's integer index.
(define (pat//per-cycle f)
  (lambda (span)
    (list/map (lambda (cyc)
                (let ((start (floor (car cyc))))
                  (pat/event (f start) cyc (cons start (+ start 1)))))
              (pat/span-cycles span))))

;; Repeats the given value once per cycle.
(define (pat/pure v)
  (pat//per-cycle (lambda (i) v)))

;; The index of each cycle, once per cycle. Indices are exact integers
;; and are negative before cycle 0.
(define pat/indices
  (pat//per-cycle (lambda (i) i)))

;; The pattern producing no events.
(define pat/silence (lambda (span) '()))

;; A continuous pattern sampling `sample` at the query span's midpoint.
;; Signal events carry no `whole`, and a signal yields exactly one event
;; for any query, including a zero-width one.
(define (pat/signal sample)
  (lambda (span)
    (if (function? sample)
        (list (pat/event (sample (pat//span-mid span)) span #f))
        '())))

;; A continuous pattern of a constant value.
(define (pat/steady v)
  (pat/signal (lambda (r) v)))

;; A signal ramping 0 to 1 over every cycle.
(define pat/saw
  (pat/signal (lambda (r) (- r (floor r)))))

;; A signal ramping -1 to 1 over every cycle.
(define pat/saw2
  (pat/signal (lambda (r) (- (* 2 (- r (floor r))) 1))))

;; A signal of seeded random floats in [0, 1). The value at time `t` is
;; `(rng/uniform (rng/fold-in seed t))`, so it depends only on the seed and
;; the exact time.
(define (pat/rand seed)
  (pat/signal (lambda (t) (rng/uniform (rng/fold-in seed t)))))

;; Query the pattern over the span. Events are sorted by active-span start.
(define (pat/query p span)
  (list/sort pat//event-earlier? (pat//events p span)))

;; The grid that pattern time snaps to when converting from floats. It is
;; fine enough for musical subdivisions, with 2^7 * 3 * 5 slots per cycle.
;; It is coarse enough to keep denominators bounded.
(define pat//grid 1920)

;; Convert a number to an exact rational, snapping floats to the nearest
;; 1/1920 of a cycle. Exact numbers pass through untouched. Graph number
;; nodes produce floats, so node exprs pass numeric pattern parameters
;; through this to keep pattern time exact.
(define (pat/rationalize x)
  (if (number? x)
      (if (exact? x)
          x
          (/ (exact (round (* x pat//grid))) pat//grid))
      x))

;; Speed the pattern up by the factor `r`. A zero rate yields silence.
(define (pat/fast r p)
  (if (zero? r)
      pat/silence
      (lambda (span)
        (list/map (lambda (e)
                    (pat/event-map-spans
                     (lambda (s) (pat/span-map (lambda (t) (/ t r)) s))
                     e))
                  (pat//events p (pat/span-map (lambda (t) (* t r)) span))))))

;; Slow the pattern down by the factor `r`. A zero factor yields silence.
(define (pat/slow r p)
  (if (zero? r)
      pat/silence
      (pat/fast (/ 1 r) p)))

;; Shift the pattern later in time by `amount` cycles.
(define (pat/shift amount p)
  (lambda (span)
    (list/map (lambda (e)
                (pat/event-map-spans
                 (lambda (s) (pat/span-map (lambda (t) (+ t amount)) s))
                 e))
              (pat//events p (pat/span-map (lambda (t) (- t amount)) span)))))

;; Concatenate the patterns, one pattern per cycle. As in Tidal's `cat`,
;; each pattern plays its own cycles in turn: cycle `c` plays cycle
;; `(c - ix) / n` of the pattern at `ix = c mod n`.
(define (pat/slowcat ps)
  (let ((n (length ps)))
    (if (zero? n)
        pat/silence
        (lambda (span)
          (list/flat-map
           (lambda (cyc)
             (let ((c (floor (car cyc))))
               (let ((ix (modulo c n)))
                 (pat//events
                  (pat/shift (- c (quotient (- c ix) n)) (list-ref ps ix))
                  cyc))))
           (pat/span-cycles span))))))

;; Concatenate the patterns so they all fit within a single cycle.
(define (pat/fastcat ps)
  (let ((n (length ps)))
    (if (zero? n)
        pat/silence
        (pat/fast n (pat/slowcat ps)))))

;; Like [`pat/fastcat`], but each element is a `(list weight pattern)`
;; pair giving the pattern's proportion of the cycle. Every resulting
;; event's whole becomes its pattern's sub-span. An element that is not
;; such a pair with a number weight is ignored.
(define (pat/timecat pairs)
  (pat//timecat (list/filter pat//weighted? pairs)))

;; Whether `pr` is a `(list weight pattern)` pair with a number weight.
(define (pat//weighted? pr)
  (if (pair? pr)
      (if (number? (car pr)) (pair? (cdr pr)) #f)
      #f))

(define (pat//timecat pairs)
  (let ((total (list/fold (lambda (acc pr) (+ acc (car pr))) 0 pairs)))
    (if (zero? total)
        pat/silence
        (let ((sub-spans (pat//timecat-spans pairs total 0 '())))
          (lambda (span)
            (list/flat-map
             (lambda (cyc)
               (let ((sam (floor (car cyc))))
                 (list/flat-map
                  (lambda (sp)
                    (let ((p-span (pat/span-map (lambda (t) (+ t sam)) (car sp))))
                      (let ((sect (pat/span-intersect cyc p-span)))
                        (if sect
                            (list/map (lambda (e)
                                        (pat/event (pat/event-value e)
                                                   (pat/event-active e)
                                                   p-span))
                                      (pat//events (car (cdr sp)) sect))
                            '()))))
                  sub-spans)))
             (pat/span-cycles span)))))))

;; Normalize timecat weights into abutting sub-spans of one cycle.
(define (pat//timecat-spans pairs total start acc)
  (if (empty? pairs)
      (reverse acc)
      (let ((w (car (car pairs)))
            (p (car (cdr (car pairs)))))
        (let ((end (+ start (/ w total))))
          (pat//timecat-spans (cdr pairs)
                              total
                              end
                              (cons (list (cons start end) p) acc))))))

;; Layer the patterns. A query concatenates every pattern's events.
(define (pat/stack ps)
  (lambda (span)
    (list/flat-map (lambda (p) (pat//events p span)) ps)))

;; Fit the pattern's `src` span to the `dst` span by adjusting the rate
;; and shifting. Degenerate spans yield silence.
(define (pat/fit-span src dst p)
  (if (zero? (pat/span-len dst))
      pat/silence
      (if (zero? (pat/span-len src))
          pat/silence
          (let ((r (/ (pat/span-len src) (pat/span-len dst))))
            (pat/shift (- (car dst) (* (car src) r)) (pat/fast r p))))))

;; [`pat/fit-span`] with a single-cycle `src`.
(define (pat/fit-cycle dst p)
  (pat/fit-span (cons 0 1) dst p))

;; The events of `p`, kept per cycle. The first query of a cycle queries `p`
;; over the whole cycle once. Later queries of the cycle read the kept
;; events and clip them to the query. The kept events go in `n` slices of
;; 1/n cycle each, and a query reads only the slices that it overlaps. The
;; latest `pat//cache-cycles` cycles are kept.
;;
;; The events are those of `p` when a query of part of a cycle gives the
;; events of the query of the whole cycle, clipped to the part. Patterns
;; from the combinators of this module do. A cycle with an event that has
;; no whole, as from a signal, is not kept. Its queries go to `p`, as do
;; zero-width queries. A query across a cycle boundary gives the events of
;; each cycle apart, and an event across the boundary comes in two parts.
(define (pat/cache n p)
  (let ((n (if (if (integer? n) (> n 0) #f) (exact n) 1))
        (kept (hash))
        (order '()))
    (lambda (span)
      (if (< (car span) (cdr span))
          (list/flat-map
           (lambda (cyc)
             (let ((c (floor (car cyc))))
               (let ((slices (if (hash-contains? kept c)
                                 (hash-ref kept c)
                                 (let ((slices (pat//cache-cycle n p c)))
                                   (set! kept (hash-insert kept c slices))
                                   (set! order (cons c order))
                                   (if (> (length order) pat//cache-cycles)
                                       (begin
                                         (set! kept (hash-remove kept (list-ref order pat//cache-cycles)))
                                         (set! order (list/take order pat//cache-cycles)))
                                       #f)
                                   slices))))
                 (if slices
                     (pat//cache-read n c slices cyc)
                     (pat//events p cyc)))))
           (pat/span-cycles span))
          (pat//events p span)))))

;; How many cycles a [`pat/cache`] keeps.
(define pat//cache-cycles 4)

;; The events of `p` over cycle `c` in `n` slices, or #f when an event has no
;; whole. An event goes in each slice that its active span overlaps.
(define (pat//cache-cycle n p c)
  (let ((evs (pat//events p (cons c (+ c 1)))))
    (if (list/all pat/event-whole evs)
        (pat//cache-fill n c (make-vector n '()) (reverse evs))
        #f)))

(define (pat//cache-fill n c slices evs)
  (if (empty? evs)
      slices
      (let ((a (pat/event-active (car evs))))
        (let ((first (pat//cache-slice n c (car a))))
          (pat//cache-add! slices (car evs) first (pat//max2 first (pat//cache-last n c (cdr a))))
          (pat//cache-fill n c slices (cdr evs))))))

(define (pat//cache-add! slices e i last)
  (if (> i last)
      slices
      (begin
        (vector-set! slices i (cons e (vector-ref slices i)))
        (pat//cache-add! slices e (+ i 1) last))))

;; The slice of cycle `c` that holds the time `t`.
(define (pat//cache-slice n c t)
  (pat//min2 (- n 1) (pat//max2 0 (exact (floor (* (- t c) n))))))

;; The last slice of cycle `c` that a span ending at `t` overlaps.
(define (pat//cache-last n c t)
  (pat//min2 (- n 1) (pat//max2 0 (- (exact (ceiling (* (- t c) n))) 1))))

;; The kept events of cycle `c` that overlap `span`, clipped to it. A slice
;; after the first gives only the events that start in it, so each event
;; comes once.
(define (pat//cache-read n c slices span)
  (let ((first (pat//cache-slice n c (car span))))
    (pat//cache-gather n c slices span first first
                       (pat//max2 first (pat//cache-last n c (cdr span)))
                       '())))

(define (pat//cache-gather n c slices span first i last acc)
  (if (> i last)
      (reverse acc)
      (let ((start (+ c (/ i n))))
        (pat//cache-gather
         n c slices span first (+ i 1) last
         (pat//cache-clip (vector-ref slices i) span (if (= i first) #f start) acc)))))

;; Push each event of `evs` that starts at or after `from` onto `acc`, with
;; its active span clipped to `span`. A #f `from` takes every event.
(define (pat//cache-clip evs span from acc)
  (if (empty? evs)
      acc
      (let ((e (car evs)))
        (let ((a (pat/event-active e)))
          (pat//cache-clip
           (cdr evs) span from
           (if (if from (< (car a) from) #f)
               acc
               (if (if (>= (car a) (car span)) (<= (cdr a) (cdr span)) #f)
                   (cons e acc)
                   (let ((clipped (pat/span-intersect a span)))
                     (if clipped
                         (cons (pat/event (pat/event-value e) clipped (pat/event-whole e)) acc)
                         acc)))))))))

;; Map event values with `f`. A non-fn `f` yields silence.
(define (pat/map f p)
  (lambda (span)
    (if (function? f)
        (list/map (lambda (e) (pat/event-map-value f e)) (pat//events p span))
        '())))

;; Map events with `f`, which takes an event and returns an event. Unlike
;; [`pat/map`], `f` sees the event's spans. Results that are not events
;; are dropped, so a partial eval of `f` stays silent. A non-fn `f`
;; yields silence.
(define (pat/map-events f p)
  (lambda (span)
    (list/filter event? (list/map f (pat//events p span)))))

;; Keep events whose value satisfies `keep?`. A non-fn `keep?` yields
;; silence.
(define (pat/filter keep? p)
  (lambda (span)
    (if (function? keep?)
        (list/filter (lambda (e) (keep? (pat/event-value e))) (pat//events p span))
        '())))

;; Keep events satisfying `keep?`. A non-fn `keep?` yields silence.
(define (pat/filter-events keep? p)
  (lambda (span)
    (list/filter keep? (pat//events p span))))

;; The cycles before a query in which [`pat//map-len-by`] looks for the last
;; part of an event that grows into the query.
(define pat//len-lookback 1)

;; A new length of an event, in cycles. A positive number snaps to the
;; grid, as [`pat/rationalize`] does. Anything else, and a number that snaps
;; to 0, is #f.
(define (pat//len x)
  (if (number? x)
      (let ((r (pat/rationalize x)))
        (if (> r 0) r #f))
      #f))

;; Give each event of `p` the length `(g e)` in cycles, with its start kept.
;; A length that is not a positive number drops the event. An event with no
;; whole, a signal, passes through.
;;
;; Each part of an event that a query chops keeps its active span, clipped
;; to the new whole. The last part, the one that ends with the whole,
;; moves its end with the whole.
;;
;; When `grows?` is true, an event that grows can reach into a query from
;; before it. The query then looks back `pat//len-lookback` cycles for such
;; last parts. An event that grows by that or more can miss a query that
;; starts after its old end by that much. Its output then depends on the
;; query span.
;;
;; When `grows?` is false, a new length longer than the old one is cut to
;; the old one. No event can then reach past its old whole, and the query
;; skips the lookback.
(define (pat//map-len-by g grows? p)
  (lambda (span)
    (let ((now (list/flat-map
                (lambda (e) (if (pat/event-whole e) (pat//relen g grows? e span) (list e)))
                (pat//events p span))))
      (if grows?
          (append
           now
           (list/flat-map
            (lambda (e) (if (pat/event-whole e) (pat//relen g grows? e span) '()))
            (pat//events p (cons (- (car span) pat//len-lookback) (car span)))))
          now))))

;; The part of event `e` within `span` once its whole has the length that
;; `g` gives it, as a list of no events or one. When `grows?` is false, the
;; new whole ends no later than the old one.
(define (pat//relen g grows? e span)
  (let ((len (pat//len (g e))))
    (if len
        (let ((w (pat/event-whole e))
              (a (pat/event-active e)))
          (let ((end (+ (car w) len)))
            (let ((nw (cons (car w) (if grows? end (pat//min2 end (cdr w))))))
              (let ((na (if (= (cdr a) (cdr w)) (cons (car a) (cdr nw)) a)))
                (let ((clip (pat/span-intersect na nw)))
                  (let ((in (if clip (pat/span-intersect clip span) #f)))
                    (if in (list (pat/event (pat/event-value e) in nw)) '())))))))
        '())))

;; The length of the whole of `e`, in cycles.
(define (pat//whole-len e)
  (pat/span-len (pat/event-whole e)))

;; Give each event of `p` the length `(f len)`, where `len` is the length
;; of its whole in cycles. See [`pat//map-len-by`]. A non-fn `f` yields
;; silence.
(define (pat/map-len f p)
  (if (function? f)
      (pat//map-len-by (lambda (e) (f (pat//whole-len e))) #t p)
      pat/silence))

;; Multiply the length of each event of `p` by `k`, a number or a pattern
;; of numbers. A pattern factor is the first number of `k` at the start of
;; each event's whole, so the structure comes from `p`. An event with no
;; factor at its start keeps its length, as does every event when `k` is
;; neither a number nor a pattern. See [`pat//map-len-by`].
;;
;; A number factor of 1 or less only shortens events. A query then skips
;; the lookback. A pattern factor can grow events, and a query of it looks
;; back.
(define (pat/scale-len k p)
  (if (number? k)
      (pat//map-len-by (lambda (e) (* k (pat//whole-len e))) (> k 1) p)
      (if (function? k)
          (pat//map-len-by
           (lambda (e)
             (let ((len (pat//whole-len e))
                   (f (list/find number?
                                 (list/map pat/event-value
                                           (pat/at k (car (pat/event-whole e)))))))
               (if f (* f len) len)))
           #t
           p)
          p)))

;; Drop each event of `p` with the probability `prob`, drawn from `seed`.
;; The draw keys on the midpoint of the event's whole, or of its active
;; span when it has no whole. So all fragments of an event agree, and an
;; event is kept exactly when `(pat/rand seed)` at that midpoint is at
;; least `prob`. A non-number `prob` yields silence.
(define (pat/degrade-by seed prob p)
  (if (number? prob)
      (pat/filter-events
       (lambda (e)
         (let ((mid (pat//span-mid (pat/event-whole-or-active e))))
           (not (rng/bernoulli (rng/fold-in seed mid) prob))))
       p)
      pat/silence))

;; The whole common to both events. It is the intersection of their wholes
;; when both are present. Otherwise, and when the wholes do not intersect,
;; it is #f.
(define (pat//whole-intersect ow iw)
  (if ow (if iw (pat/span-intersect ow iw) #f) #f))

;; Join a pattern of patterns. Inner patterns are queried with the outer
;; event's active span. Both whole and active spans are intersected.
(define (pat/join pp)
  (lambda (span)
    (list/flat-map
     (lambda (oe)
       (list/flat-map
        (lambda (ie)
          (let ((active (pat/span-intersect (pat/event-active oe)
                                            (pat/event-active ie))))
            (if active
                (list (pat/event (pat/event-value ie)
                                 active
                                 (pat//whole-intersect (pat/event-whole oe)
                                                       (pat/event-whole ie))))
                '())))
        (pat//events (pat/event-value oe) (pat/event-active oe))))
     (pat//events pp span))))

;; Like [`pat/join`], but structure comes from the inner pattern alone.
;; Wholes are untouched and actives are clipped to the original query span.
(define (pat/inner-join pp)
  (lambda (q-span)
    (list/flat-map
     (lambda (oe)
       (list/flat-map
        (lambda (ie)
          (let ((active (pat/span-intersect q-span (pat/event-active ie))))
            (if active
                (list (pat/event (pat/event-value ie) active (pat/event-whole ie)))
                '())))
        (pat//events (pat/event-value oe) (pat/event-active oe))))
     (pat//events pp q-span))))

;; Like [`pat/join`], but structure comes from the outer pattern alone.
;; The inner is queried at the instant of the outer's whole start, so a
;; discrete inner yields nothing and only signal inners are productive.
(define (pat/outer-join pp)
  (lambda (q-span)
    (list/flat-map
     (lambda (oe)
       (let ((start (car (pat/event-whole-or-active oe))))
         (list/flat-map
          (lambda (ie)
            (let ((active (pat/span-intersect q-span (pat/event-active oe))))
              (if active
                  (list (pat/event (pat/event-value ie) active (pat/event-whole oe)))
                  '())))
          (pat//events (pat/event-value oe) (cons start start)))))
     (pat//events pp q-span))))

;; `(list (f v))`, or no values when `f` is not a function.
(define (pat//applied f v)
  (if (function? f) (list (f v)) '()))

;; Apply a pattern of functions `pf` to a pattern of values `pv`. Both
;; sides are queried with the query span. Each intersection of active
;; spans yields an event. Its whole is the intersection of both wholes
;; when both are present, else #f. A value of `pf` that is not a function
;; yields no event.
(define (pat/app pv pf)
  (lambda (span)
    (list/flat-map
     (lambda (ev)
       (list/flat-map
        (lambda (ef)
          (let ((active (pat/span-intersect (pat/event-active ev)
                                            (pat/event-active ef))))
            (if active
                (list/map (lambda (v)
                            (pat/event v
                                       active
                                       (pat//whole-intersect (pat/event-whole ev)
                                                             (pat/event-whole ef))))
                          (pat//applied (pat/event-value ef) (pat/event-value ev)))
                '())))
        (pat//events pf span)))
     (pat//events pv span))))

;; Join the events of `ps`, the side with the structure, with the events
;; of `po` that meet them, as Tidal's `<*` and `*>` do. Each event of `ps`
;; queries `po` over its whole, or over its active span when it has none,
;; so a signal gives one value however a query chops the event. Each
;; intersection of active spans yields an event with the whole of the
;; `ps` event for each value in `(combine s-value o-value)`.
(define (pat//app-side ps po combine)
  (lambda (span)
    (list/flat-map
     (lambda (es)
       (list/flat-map
        (lambda (eo)
          (let ((active (pat/span-intersect (pat/event-active es)
                                            (pat/event-active eo))))
            (if active
                (list/map (lambda (v) (pat/event v active (pat/event-whole es)))
                          (combine (pat/event-value es) (pat/event-value eo)))
                '())))
        (pat//events po (pat/event-whole-or-active es))))
     (pat//events ps span))))

;; Apply with structure from the value pattern `pv`.
(define (pat/appl pv pf)
  (pat//app-side pv pf (lambda (v f) (pat//applied f v))))

;; Apply with structure from the function pattern `pf`.
(define (pat/appr pv pf)
  (pat//app-side pf pv pat//applied))

;; Merge two patterns by calling `(f a-value b-value)` at every
;; intersection of active spans. Structure is the intersection of both
;; wholes. A non-fn `f` yields silence.
(define (pat/merge-with f pa pb)
  (if (function? f)
      (pat/app pa (pat/map (lambda (bv) (lambda (av) (f av bv))) pb))
      pat/silence))

;; [`pat/merge-with`] with structure from `pa`, as [`pat/appl`] has.
(define (pat/merge-withl f pa pb)
  (if (function? f)
      (pat/appl pa (pat/map (lambda (bv) (lambda (av) (f av bv))) pb))
      pat/silence))

;; [`pat/merge-with`] with structure from `pb`, as [`pat/appr`] has.
(define (pat/merge-withr f pa pb)
  (if (function? f)
      (pat/appr pa (pat/map (lambda (bv) (lambda (av) (f av bv))) pb))
      pat/silence))

;; Sample `p` into `n` events per cycle, as Tidal's `segment` does. Each
;; event takes the values of `p` where they meet it, so a signal gives each
;; event one value. An `n` that is not a positive number yields silence.
(define (pat/segment n p)
  (if (if (number? n) (> n 0) #f)
      (pat/appr p (pat/fast (pat/rationalize n) (pat/pure (lambda (v) v))))
      pat/silence))

;; A pattern of one-key maps, with each value of `p` under `key`.
(define (pat/key key p)
  (pat/map (lambda (v) (hash key v)) p))

;; The value under `key` of each map of `p`, the inverse of [`pat/key`]. An
;; event whose value is not a map, or has no `key`, is dropped.
(define (pat/get key p)
  (lambda (span)
    (list/flat-map
     (lambda (e)
       (let ((m (pat/event-value e)))
         (if (if (hash? m) (hash-contains? m key) #f)
             (list (pat/event-map-value (lambda (m) (hash-ref m key)) e))
             '())))
     (pat//events p span))))

;; Union two event values. A key in both takes its value from `b`. A
;; value that is not a map gives way to one that is.
(define (pat//union a b)
  (if (hash? a)
      (if (hash? b) (hash-union b a) a)
      b))

;; Add the values of `pb` to the maps of `pa`, as Tidal's `#` does.
;; Structure comes from `pa`, as [`pat/appl`] has. An event with nothing
;; from `pb` is dropped.
(define (pat/with pa pb)
  (pat//app-side pa pb (lambda (a b) (list (pat//union a b)))))

(define (pat//repeat v n acc)
  (if (<= n 0) acc (pat//repeat v (- n 1) (cons v acc))))

;; Pairwise-append two equal-length lists of lists.
(define (pat//zip-append xs ys acc)
  (if (empty? xs)
      (reverse acc)
      (pat//zip-append (cdr xs) (cdr ys) (cons (append (car xs) (car ys)) acc))))

;; The bjorklund left/right merge over two lists of onset groups. The
;; true merge is required here. Bresenham-style closed forms produce a
;; differently rotated pattern. For example, they diverge at (5, 8).
(define (pat//bjorklund-loop xs ys)
  (if (<= (pat//min2 (length xs) (length ys)) 1)
      (append xs ys)
      (if (> (length xs) (length ys))
          (let ((ly (length ys)))
            (pat//bjorklund-loop (pat//zip-append (take xs ly) ys '())
                                 (list-tail xs ly)))
          (let ((lx (length xs)))
            (pat//bjorklund-loop (pat//zip-append xs (take ys lx) '())
                                 (list-tail ys lx))))))

;; Rotate the list left by `off` modulo its length.
(define (pat//rotate xs off)
  (let ((len (length xs)))
    (if (< len 1)
        xs
        (let ((o (modulo off len)))
          (append (list-tail xs o) (take xs o))))))

;; The bjorklund onset pattern distributing `k` onsets as evenly as
;; possible over `n` slots, rotated left by `off` slots. `k` is clamped
;; to `0..=n`. Returns a list of `n` booleans.
(define (pat/euclid-bools k n off)
  (if (< n 1)
      '()
      (let ((kk (pat//min2 (pat//max2 k 0) n)))
        (pat//rotate
         (list/concat
          (pat//bjorklund-loop (pat//repeat (list #t) kk '())
                               (pat//repeat (list #f) (- n kk) '())))
         off))))

;; Cyclic distance from each slot to the next onset, inclusive of the
;; current slot. Returns the empty list when there are no onsets at all.
(define (pat//onset-distances bs)
  (let ((len (length bs)))
    (if (< len 1)
        '()
        (if (pat//onset-distance 0 bs len)
            (list/map (lambda (i) (pat//onset-distance i bs len)) (range 0 len))
            '()))))

(define (pat//onset-distance ix bs len)
  (pat//onset-distance-loop ix 1 bs len))

(define (pat//onset-distance-loop ix dist bs len)
  (if (> dist len)
      #f
      (if (list-ref bs (modulo (+ ix dist) len))
          dist
          (pat//onset-distance-loop ix (+ dist 1) bs len))))

;; Map the span's length with `f`, adjusting the end.
(define (pat//span-map-len f s)
  (cons (car s) (+ (car s) (f (- (cdr s) (car s))))))

;; `k` onsets distributed over `n` equal slots per cycle. Silent slots are
;; filtered out. Event values are #t.
(define (pat/euclid k n)
  (pat/euclid-off k n 0))

;; [`pat/euclid`] rotated left by `off` slots.
(define (pat/euclid-off k n off)
  (pat/filter (lambda (v) v)
              (pat/fastcat (list/map pat/pure (pat/euclid-bools k n off)))))

;; [`pat/euclid`] with each onset elongated to fill the silence before
;; the next onset. Event values are #t.
(define (pat/euclid-full k n)
  (let ((bs (pat/euclid-bools k n 0)))
    (let ((ds (pat//onset-distances bs)))
      (if (empty? ds)
          pat/silence
          (let ((p (pat/fastcat (list/map pat/pure (list/zip bs ds)))))
            (lambda (span)
              (list/flat-map
               (lambda (e)
                 (let ((v (pat/event-value e)))
                   (if (car v)
                       (list (pat/event-map-value
                              (lambda (bv) #t)
                              (pat/event-map-spans
                               (lambda (s)
                                 (pat//span-map-len
                                  (lambda (l) (* l (car (cdr v))))
                                  s))
                               e)))
                       '())))
               (p span))))))))

;; Whether the event begins at its whole's start. Such an event is a true
;; onset rather than the continuation of an event chopped by a window
;; boundary. Signal events are never onsets.
(define (pat/event-onset? e)
  (let ((w (pat/event-whole e)))
    (if w
        (let ((astart (car (pat/event-active e)))
              (wstart (car w)))
          (if (< astart wstart) #f (not (< wstart astart))))
        #f)))

;; How far the active part of `e` starts through its whole, from 0 to 1.
;; For an event from `pat/at`, this is how far its cycle point is through
;; it. A signal has no whole, so it has no phase and gives #f.
(define (pat/event-phase e)
  (let ((w (pat/event-whole e)))
    (if w
        (exact->inexact
         (/ (- (car (pat/event-active e)) (car w)) (- (cdr w) (car w))))
        #f)))

;; The largest gap in cycles that a tick may close in one window. A
;; steady window spans tick-duration times cps cycles, so this sits far
;; above sane configurations. A wider gap moves the clock to the eval
;; time rather than the playhead, so a stall cannot flood downstream
;; with a giant span.
(define pat//max-window 8)

;; The cps that a value sets. A positive number sets itself. Anything else
;; sets nothing. A value that is not positive sets nothing, since a
;; stopped clock can never reach the event that restarts it.
(define (pat//cps v)
  (if (number? v) (if (> v 0) (exact->inexact v) #f) #f))

;; A cps input as a pattern. A number is a steady cps and a function
;; passes through. Anything else gives #f.
(define (pat//cps-pattern x)
  (if (number? x) (pat/steady x) (if (function? x) x #f)))

;; Push the abutting `(start end cps)` segments of `[start, end)` onto
;; the reversed `segs`. `events` are the cps events sorted by active
;; start and `r` is the cps on entry. A gap holds the cps before it and
;; an event that sets no cps is a gap. An event that overlaps an earlier
;; one takes effect from where the earlier one ends.
(define (pat//cycle-segs events start end r segs)
  (if (empty? events)
      (if (< start end) (cons (list start end r) segs) segs)
      (let ((e (car events)))
        (let ((a (pat/span-intersect (cons start end) (pat/event-active e)))
              (v (pat//cps (pat/event-value e))))
          (if (if a v #f)
              (pat//cycle-segs (cdr events) (cdr a) end v
                               (cons (list (car a) (cdr a) v)
                                     (if (< start (car a))
                                         (cons (list start (car a) r) segs)
                                         segs)))
              (pat//cycle-segs (cdr events) start end r segs))))))

;; The cps at the end of the reversed `segs`, or `r` when there are none.
(define (pat//segs-cps segs r)
  (if (empty? segs) r (list-ref (car segs) 2)))

;; The reversed `(start end cps)` segments of `cps` over `span`, where
;; `r` is the cps before the span. Each cycle has its own query, so that
;; the sample of a signal never crosses a reset at a cycle boundary.
(define (pat//cps-segs cps span r)
  (list/fold
   (lambda (segs cyc)
     (pat//cycle-segs (pat/query cps cyc) (car cyc) (cdr cyc)
                      (pat//segs-cps segs r) segs))
   '()
   (pat/span-cycles span)))

;; The seconds from cycle point `start` to `x` at `r` cps.
(define (pat//seg-secs start x r)
  (/ (exact->inexact (- x start)) r))

;; The seconds that `segs` take to play.
(define (pat//segs-secs segs)
  (list/fold
   (lambda (secs s)
     (+ secs (pat//seg-secs (car s) (list-ref s 1) (list-ref s 2))))
   0.0
   segs))

;; Advance a tick-driven clock and return its next query window.
;;
;; `st` is the clock state `(list c te r last-t)`, where any non-pair
;; means the first tick. `c` is the exact playhead in cycles, `te` the
;; seconds at which the playhead plays, `r` the last cps and `last-t` the
;; eval time of the last tick.
;; `t` is the eval time in seconds and `dt` the tick duration. `cps` is a
;; number or a pattern of numbers.
;;
;; Returns `(list span secs segs start-cps new-st)`. `span` is the window
;; in cycles and `secs` the seconds at which its start plays. `segs` are
;; the abutting `(start end cps)` segments of the span, in order.
;; `start-cps` is the cps at the start of the span.
;;
;; Each tick moves the playhead toward where it must be one tick from
;; now at the last cps, then adds the seconds that `cps` gives the new
;; span to `te`. A cps change makes that move too long or too short and
;; the next tick corrects it. Event times do not depend on this, as
;; `te` integrates the exact span. Spans abut, the playhead never jumps
;; and the 1/1920 grid only picks the window boundaries.
;;
;; A gap in `cps` holds the last cps, which starts at 1. The first
;; tick starts the playhead at cycle 0. `te` moves to the eval time when
;; it trails by more than `pat//max-window` cycles, or when eval time
;; runs backwards as state restored from an older session does. The
;; playhead stays put, so the pattern resumes where it was. A partial
;; eval without a numeric time or tick, or without `cps`, holds the
;; clock. Its span is then empty at the playhead.
(define (pat/window st t dt cps)
  (let ((p (pat//cps-pattern cps)))
    (if (if (number? t) (if (number? dt) p #f) #f)
        (let ((s (if (pair? st) st (list 0 t 1.0 t))))
          (pat//advance (car s) (list-ref s 1) (list-ref s 2) (list-ref s 3)
                        t dt p))
        (pat//held st t))))

(define (pat//advance c te r last-t t dt cps)
  (let ((target (+ t dt)))
    (let ((from (if (if (< t last-t)
                        #t
                        (> (* (- target te) r) pat//max-window))
                    t
                    te)))
      (let ((gap (* (- target from) r)))
        (let ((end (if (> gap 0) (+ c (pat/rationalize gap)) c)))
          (let ((segs (pat//cps-segs cps (cons c end) r)))
            (let ((ordered (reverse segs)))
              (list (cons c end)
                    from
                    ordered
                    (if (empty? ordered) r (list-ref (car ordered) 2))
                    (list end
                          (+ from (pat//segs-secs segs))
                          (pat//segs-cps segs r)
                          t)))))))))

;; The window of a clock that does not advance. Its span is empty at the
;; playhead of `st`. A clock with no state yet sits at cycle 0 at time `t`,
;; at the start cps of 1.
(define (pat//held st t)
  (if (pair? st)
      (let ((c (car st)))
        (list (cons c c) (list-ref st 1) '() (list-ref st 2) st))
      (list (cons 0 0) (if (number? t) t 0.0) '() 1.0 st)))

;; The seconds at which cycle point `x` plays. `segs` are the abutting
;; segments of a window and `te` the seconds of their start.
(define (pat//point-secs te segs x)
  (if (empty? segs)
      te
      (let ((s (car segs)))
        (if (< x (list-ref s 1))
            (+ te (pat//seg-secs (car s) x (list-ref s 2)))
            (pat//point-secs
             (+ te (pat//seg-secs (car s) (list-ref s 1) (list-ref s 2)))
             (cdr segs)
             x)))))

;; The onsets among queried window events, as a schedule of
;; `(list seconds value)` pairs for timestamped delivery paths.
;;
;; `secs` and `segs` are the seconds and the segments that `pat/window`
;; gave with the queried span. Only onset events are kept, as
;; window-chopped continuations of a sustained event would otherwise
;; retrigger every tick. Times and numeric values leave as floats, since
;; the delivery paths drop rationals.
(define (pat/events->secs events secs segs)
  (if (if (list? events) (if (number? secs) (list? segs) #f) #f)
      (list/map
       (lambda (e)
         (list (exact->inexact
                (pat//point-secs secs segs (car (pat/event-active e))))
               (pat//inexact (pat/event-value e))))
       (list/filter pat/event-onset? events))
      '()))

;; The events of `p` that are alive at cycle point `x`. The query covers
;; one step of the grid at `x`, and a part of an event that starts later in
;; that step does not count, so each event counts once. A partial eval
;; with no cycle point gives no events.
(define (pat/at p x)
  (if (number? x)
      (let ((x (pat/rationalize x)))
        (list/filter
         (lambda (e) (not (< x (car (pat/event-active e)))))
         (pat/query p (cons x (+ x (/ 1 pat//grid))))))
      '()))

;; The cycle point `secs` after cycle point `x` at `cps`, or before it
;; when `secs` is negative. A target whose output leads another reads
;; behind by the difference, and a target whose output lags reads ahead by
;; its latency. A cps change between the two points counts, so the same
;; seconds cover a different number of cycles on each side of it. `held`
;; is the cps at `x`, which a gap in `cps` holds. A partial eval with a
;; missing input gives `x` back.
(define (pat/nudge x secs cps held)
  (let ((p (pat//cps-pattern cps))
        (r (pat//cps held)))
    (if (if (number? x) (if (number? secs) (if p r #f) #f) #f)
        (let ((x (pat/rationalize x)))
          (if (< secs 0)
              (pat//seek-back p x (- 0 secs) r)
              (pat//seek-ahead p x secs r)))
        x)))

;; The seconds from cycle point `from` to cycle point `to` at `cps`, the
;; inverse of [`pat/nudge`]. They are negative when `to` is before `from`.
;; A cps change between the two points counts. `held` is the cps at `from`,
;; which a gap in `cps` holds. A partial eval with a missing input gives
;; #f.
(define (pat/secs from to cps held)
  (let ((p (pat//cps-pattern cps))
        (r (pat//cps held)))
    (if (if (number? from) (if (number? to) (if p r #f) #f) #f)
        (let ((a (pat/rationalize from))
              (b (pat/rationalize to)))
          (if (< b a)
              (- 0.0 (pat//span-secs p (cons b a) r))
              (pat//span-secs p (cons a b) r)))
        #f)))

;; The seconds that `span` takes to play at `cps`, where `r` is the cps
;; that a gap holds.
(define (pat//span-secs cps span r)
  (pat//segs-secs (pat//cps-segs cps span r)))

;; The maps of `p`, each with the seconds of its event's whole at `cps`
;; under `key`. Tidal sends this value to SuperDirt as `delta`. A cps
;; change inside an event counts. `held` is the cps that a gap in `cps`
;; holds. A value that is not a map becomes a map of the one key, as
;; [`pat/with`] does. A signal has no whole, so it passes through as it
;; is, as does each event of a partial eval with a missing cps.
(define (pat/len-secs key p cps held)
  (let ((c (pat//cps-pattern cps))
        (r (pat//cps held)))
    (if (if c r #f)
        (pat/map-events
         (lambda (e)
           (let ((w (pat/event-whole e)))
             (if w
                 (pat/event-map-value
                  (lambda (v)
                    (hash-insert (if (hash? v) v (hash)) key (pat//span-secs c w r)))
                  e)
                 e)))
         p)
        p)))

;; Each cycle has its own query, as in `pat//cps-segs`.
(define (pat//seek-ahead cps x secs r)
  (let ((end (+ (floor x) 1)))
    (pat//seek-ahead-segs cps end
                          (reverse (pat//cps-segs cps (cons x end) r))
                          secs r)))

(define (pat//seek-ahead-segs cps end segs secs r)
  (if (empty? segs)
      (pat//seek-ahead cps end secs r)
      (let ((s (car segs)))
        (let ((dur (pat//seg-secs (car s) (list-ref s 1) (list-ref s 2))))
          (if (< secs dur)
              (+ (car s) (pat/rationalize (* secs (list-ref s 2))))
              (pat//seek-ahead-segs cps end (cdr segs) (- secs dur)
                                    (list-ref s 2)))))))

(define (pat//seek-back cps x secs r)
  (let ((start (- (ceiling x) 1)))
    (pat//seek-back-segs cps start
                         (pat//cps-segs cps (cons start x) r)
                         secs r)))

(define (pat//seek-back-segs cps start segs secs r)
  (if (empty? segs)
      (pat//seek-back cps start secs r)
      (let ((s (car segs)))
        (let ((dur (pat//seg-secs (car s) (list-ref s 1) (list-ref s 2))))
          (if (< secs dur)
              (- (list-ref s 1) (pat/rationalize (* secs (list-ref s 2))))
              (pat//seek-back-segs cps start (cdr segs) (- secs dur)
                                   (list-ref s 2)))))))

;; Apply the euclidean mask to the pattern. Structure comes from the
;; mask's onsets, values from `p`.
(define (pat/euclid-with p k n off)
  (pat/appr p (pat/map (lambda (b) (lambda (v) v)) (pat/euclid-off k n off))))

;; Coerce `x` to a query span for plotting. A pair of numbers passes
;; through rationalized. A number `n` is the span from 0 to `n`. Anything
;; else, or a span that does not move forward, gives `default`.
(define (pat/as-span x default)
  (let ((s (if (pair? x)
               (if (number? (car x))
                   (if (number? (cdr x))
                       (cons (pat/rationalize (car x)) (pat/rationalize (cdr x)))
                       #f)
                   #f)
               (if (number? x) (cons 0 (pat/rationalize x)) #f))))
    (if s (if (< (car s) (cdr s)) s default) default)))

;; A value on its way out of pattern time. A number becomes inexact. Any
;; other value passes through.
(define (pat//inexact v)
  (if (number? v) (exact->inexact v) v))

;; The `i`th of `n` equal slices of `span`.
(define (pat//span-slice span i n)
  (let ((start (pat/span-start span))
        (len (pat/span-len span)))
    (pat/span (+ start (/ (* len i) n)) (+ start (/ (* len (+ i 1)) n)))))

;; The `(x value)` samples of the signal events in `p` over `n` slices of
;; `span`. `x` is the midpoint of each sampled event's active span.
(define (pat//signal-points p span n)
  (pat//signal-points-loop p span n (- n 1) '()))

(define (pat//signal-points-loop p span n i acc)
  (if (< i 0)
      acc
      (pat//signal-points-loop
       p span n (- i 1)
       (pat//rev-append
        (list/fold
         (lambda (pts e)
           (let ((a (pat/event-active e)))
             (if (pat/event-whole e)
                 pts
                 (cons (list (exact->inexact (/ (+ (car a) (cdr a)) 2))
                             (pat//inexact (pat/event-value e)))
                       pts))))
         '()
         (pat//events p (pat//span-slice span i n)))
        acc))))

;; Query `p` over `span` for plotting. Returns
;; `(list start end segments points)`. Times and top-level numeric values
;; are inexact. Other values pass through.
;;
;; `segments` holds one `(start end value onset?)` per discrete event,
;; spanning its active part. `points` holds `(x value)` samples of any
;; continuous signal, taken over `res` equal slices of the span. Signals
;; are only sampled when the full query holds a signal event, so a
;; discrete pattern costs one query. Events are not sorted, as the plotter
;; needs no order. A non-pattern `p` gives no segments and no points.
(define (pat/plot-data p span res)
  (let ((events (pat//events p span)))
    (list (exact->inexact (pat/span-start span))
          (exact->inexact (pat/span-end span))
          (list/fold
           (lambda (segs e)
             (let ((a (pat/event-active e)))
               (if (pat/event-whole e)
                   (cons (list (exact->inexact (car a))
                               (exact->inexact (cdr a))
                               (pat//inexact (pat/event-value e))
                               (pat/event-onset? e))
                         segs)
                   segs)))
           '()
           (reverse events))
          (if (list/any (lambda (e) (not (pat/event-whole e))) events)
              (pat//signal-points p span res)
              '()))))
