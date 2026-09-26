;; The gantz/pattern module.
;;
;; A pattern is a function from a span to the list of events occurring
;; along it. Spans are pairs of exact rational time points measured in
;; cycles. Events carry a `value`, an `active` span where the value
;; applies within the query, and a `whole` span carrying the event's
;; full structure. Whole is #f for continuous signals.
;;
;; Written for the prelude-free base engine. It uses primitive special
;; forms only and defines the missing prelude list fns below. Names
;; prefixed `pat//` are internal helpers and are not provided.

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
         pat/map
         pat/filter
         pat/filter-events
         pat/join
         pat/inner-join
         pat/outer-join
         pat/app
         pat/appl
         pat/appr
         pat/merge-with
         pat/euclid-bools
         pat/euclid
         pat/euclid-off
         pat/euclid-full
         pat/event-onset?
         pat/window
         pat/events->secs
         pat/euclid-with
         pat/as-span
         pat/plot-data)

(define (pat//max2 a b) (if (< a b) b a))
(define (pat//min2 a b) (if (< b a) b a))

;; Tail-recursive list helpers, standing in for the unavailable prelude
;; fns.

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

(define (pat//fold f init xs)
  (if (empty? xs)
      init
      (pat//fold f (f init (car xs)) (cdr xs))))

;; A stable merge sort. The base engine's `sort` rejects closures.
;; `less?` must be a strict order. Merge recursion depth is bounded by the
;; list length, which is fine at event-list scale.
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
    (pat//map (lambda (cyc)
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
        (let ((mid (+ (car span) (/ (- (cdr span) (car span)) 2))))
          (list (pat/event (sample mid) span #f)))
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

;; Query the pattern over the span. Events are sorted by active-span start.
(define (pat/query p span)
  (pat//sort pat//event-earlier? (pat//events p span)))

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
        (pat//map (lambda (e)
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
    (pat//map (lambda (e)
                (pat/event-map-spans
                 (lambda (s) (pat/span-map (lambda (t) (+ t amount)) s))
                 e))
              (pat//events p (pat/span-map (lambda (t) (- t amount)) span)))))

;; Concatenate the patterns, one pattern per cycle.
(define (pat/slowcat ps)
  (let ((n (length ps)))
    (if (zero? n)
        pat/silence
        (lambda (span)
          (pat//flat-map
           (lambda (cyc)
             (let ((ix (modulo (floor (car cyc)) n)))
               (pat//events (list-ref ps ix) cyc)))
           (pat/span-cycles span))))))

;; Concatenate the patterns so they all fit within a single cycle.
(define (pat/fastcat ps)
  (let ((n (length ps)))
    (if (zero? n)
        pat/silence
        (pat/fast n (pat/slowcat ps)))))

;; Like [`pat/fastcat`], but each element is a `(list weight pattern)`
;; pair giving the pattern's proportion of the cycle. Every resulting
;; event's whole becomes its pattern's sub-span.
(define (pat/timecat pairs)
  (let ((total (pat//fold (lambda (acc pr) (+ acc (car pr))) 0 pairs)))
    (if (zero? total)
        pat/silence
        (let ((sub-spans (pat//timecat-spans pairs total 0 '())))
          (lambda (span)
            (pat//flat-map
             (lambda (cyc)
               (let ((sam (floor (car cyc))))
                 (pat//flat-map
                  (lambda (sp)
                    (let ((p-span (pat/span-map (lambda (t) (+ t sam)) (car sp))))
                      (let ((sect (pat/span-intersect cyc p-span)))
                        (if sect
                            (pat//map (lambda (e)
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
    (pat//flat-map (lambda (p) (pat//events p span)) ps)))

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

;; Map event values with `f`. A non-fn `f` yields silence.
(define (pat/map f p)
  (lambda (span)
    (if (function? f)
        (pat//map (lambda (e) (pat/event-map-value f e)) (pat//events p span))
        '())))

;; Keep events whose value satisfies `keep?`. A non-fn `keep?` yields
;; silence.
(define (pat/filter keep? p)
  (lambda (span)
    (if (function? keep?)
        (pat//filter (lambda (e) (keep? (pat/event-value e))) (pat//events p span))
        '())))

;; Keep events satisfying `keep?`. A non-fn `keep?` yields silence.
(define (pat/filter-events keep? p)
  (lambda (span)
    (if (function? keep?)
        (pat//filter keep? (pat//events p span))
        '())))

;; The whole common to both events. It is the intersection of their wholes
;; when both are present. Otherwise, and when the wholes do not intersect,
;; it is #f.
(define (pat//whole-intersect ow iw)
  (if ow (if iw (pat/span-intersect ow iw) #f) #f))

;; Join a pattern of patterns. Inner patterns are queried with the outer
;; event's active span. Both whole and active spans are intersected.
(define (pat/join pp)
  (lambda (span)
    (pat//flat-map
     (lambda (oe)
       (pat//flat-map
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
    (pat//flat-map
     (lambda (oe)
       (pat//flat-map
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
    (pat//flat-map
     (lambda (oe)
       (let ((start (car (pat/event-whole-or-active oe))))
         (pat//flat-map
          (lambda (ie)
            (let ((active (pat/span-intersect q-span (pat/event-active oe))))
              (if active
                  (list (pat/event (pat/event-value ie) active (pat/event-whole oe)))
                  '())))
          (pat//events (pat/event-value oe) (cons start start)))))
     (pat//events pp q-span))))

;; Apply a pattern of functions `pf` to a pattern of values `pv`. Both
;; sides are queried with the original query span. Each intersection of
;; active spans yields an event. Its whole is `(structure left-whole
;; right-whole)` when both wholes are present, else #f.
(define (pat//apply pv pf structure)
  (lambda (span)
    (pat//flat-map
     (lambda (ev)
       (pat//flat-map
        (lambda (ef)
          (let ((active (pat/span-intersect (pat/event-active ev)
                                            (pat/event-active ef)))
                (f (pat/event-value ef)))
            (if active
                (if (function? f)
                    (let ((lw (pat/event-whole ev))
                          (rw (pat/event-whole ef)))
                      (list (pat/event (f (pat/event-value ev))
                                       active
                                       (if lw (if rw (structure lw rw) #f) #f))))
                    '())
                '())))
        (pat//events pf span)))
     (pat//events pv span))))

;; Apply with structure from the intersection of both wholes.
(define (pat/app pv pf)
  (pat//apply pv pf pat/span-intersect))

;; Apply with structure from the value pattern `pv`.
(define (pat/appl pv pf)
  (pat//apply pv pf (lambda (l r) l)))

;; Apply with structure from the function pattern `pf`.
(define (pat/appr pv pf)
  (pat//apply pv pf (lambda (l r) r)))

;; Merge two patterns by calling `(f a-value b-value)` at every
;; intersection of active spans. Structure is the intersection of both
;; wholes.
(define (pat/merge-with f pa pb)
  (pat/app pa (pat/map (lambda (bv) (lambda (av) (f av bv))) pb)))

(define (pat//repeat v n acc)
  (if (<= n 0) acc (pat//repeat v (- n 1) (cons v acc))))

(define (pat//zip2 xs ys acc)
  (if (empty? xs)
      (reverse acc)
      (pat//zip2 (cdr xs) (cdr ys) (cons (list (car xs) (car ys)) acc))))

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
         (pat//flat-map (lambda (g) g)
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
            (pat//map (lambda (i) (pat//onset-distance i bs len)) (range 0 len))
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
              (pat/fastcat (pat//map pat/pure (pat/euclid-bools k n off)))))

;; [`pat/euclid`] with each onset elongated to fill the silence before
;; the next onset. Event values are #t.
(define (pat/euclid-full k n)
  (let ((bs (pat/euclid-bools k n 0)))
    (let ((ds (pat//onset-distances bs)))
      (if (empty? ds)
          pat/silence
          (let ((p (pat/fastcat (pat//map pat/pure (pat//zip2 bs ds '())))))
            (lambda (span)
              (pat//flat-map
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

;; The longest span a single window may cover, in cycles. Steady-state
;; windows span tick-duration times cps cycles, so this sits far above
;; sane configurations. It also bounds the events a single tick can
;; produce. A jump beyond it, such as a cps change rescaling the
;; timeline, resets rather than querying the whole gap.
(define pat//max-window 8)

;; Advance a tick-driven query window.
;;
;; `st` is the previous cycle position, where any non-number means the
;; first tick. `t` is the eval time in seconds and `cps` the tempo in
;; cycles per second. Returns `(list span new-position)` where `span`
;; runs from the previous position to the current one.
;;
;; The position derives from absolute time snapped to the 1/1920-cycle
;; grid. Successive spans therefore abut exactly, quantisation error never
;; accumulates, and denominators stay bounded. The span is empty on the
;; first tick and whenever the position has not advanced. Any position
;; jump beyond `pat//max-window`, in either direction, also yields an
;; empty span and continues from the new position. A cps change
;; rescales the timeline, so its gap is dropped rather than replayed or
;; fast-forwarded.
(define (pat/window st t cps)
  (if (if (number? t) (number? cps) #f)
      (let ((pos (/ (exact (round (* t cps pat//grid))) pat//grid)))
        (let ((prev (if (number? st) st pos)))
          (list (if (< pos prev)
                    (cons pos pos)
                    (if (> (- pos prev) pat//max-window)
                        (cons pos pos)
                        (cons prev pos)))
                pos)))
      ;; A partial eval without a numeric time or cps holds position, an
      ;; empty span and untouched state.
      (list (cons 0 0) st)))

;; Convert queried window events to a list of `(list seconds value)`
;; pairs for timestamped delivery paths.
;;
;; `span` is the queried window, `t` the eval time in seconds anchoring
;; the span's start, and `cps` the tempo converting cycle offsets to
;; seconds. Only onset events are kept, as window-chopped continuations
;; of a sustained event would otherwise retrigger every tick. Times and
;; numeric values leave as floats, since the delivery paths drop
;; rationals.
(define (pat/events->secs events span t cps)
  (if (if (list? events)
          (if (pair? span) (if (number? t) (number? cps) #f) #f)
          #f)
      (pat//map
       (lambda (e)
         (list (+ t (exact->inexact (/ (- (car (pat/event-active e)) (car span)) cps)))
               (let ((v (pat/event-value e)))
                 (if (number? v) (exact->inexact v) v))))
       (pat//filter pat/event-onset? events))
      '()))

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

;; An event value for plotting. A number becomes inexact. Any other value
;; passes through for the plotter to classify.
(define (pat//plot-value v)
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
        (pat//fold
         (lambda (pts e)
           (let ((a (pat/event-active e)))
             (if (pat/event-whole e)
                 pts
                 (cons (list (exact->inexact (/ (+ (car a) (cdr a)) 2))
                             (pat//plot-value (pat/event-value e)))
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
          (pat//fold
           (lambda (segs e)
             (let ((a (pat/event-active e)))
               (if (pat/event-whole e)
                   (cons (list (exact->inexact (car a))
                               (exact->inexact (cdr a))
                               (pat//plot-value (pat/event-value e))
                               (pat/event-onset? e))
                         segs)
                   segs)))
           '()
           (reverse events))
          (if (pat//fold (lambda (any e) (if any any (not (pat/event-whole e)))) #f events)
              (pat//signal-points p span res)
              '()))))
