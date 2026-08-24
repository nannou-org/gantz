;; The gantz/pattern module.
;;
;; A pattern is a function from a span to the list of events occurring
;; along it. Spans are pairs of exact rational time points measured in
;; cycles. Events carry a `value`, an `active` span where the value
;; applies within the query, and a `whole` span carrying the event's
;; full structure. Whole is #f for continuous signals.
;;
;; Written for the prelude-free base engine. Primitive special forms
;; only, with the missing prelude list fns hand-rolled below. Names
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
         pat/events->secs)

;; -- internal helpers ---------------------------------------------------------

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
;; is empty or zero-length.
(define (pat/span-intersect a b)
  (let ((start (pat//max2 (car a) (car b)))
        (end (pat//min2 (cdr a) (cdr b))))
    (if (<= end start) #f (cons start end))))

;; -- events -------------------------------------------------------------------

;; An event holds a `value`, its `active` span and its `whole` span.
;; Whole is #f for continuous signals.
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

;; Repeats the given value once per cycle.
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

;; A signal ramping -1 to 1 over every cycle.
(define pat/saw2
  (pat/signal (lambda (r) (- (* 2 (- r (floor r))) 1))))

;; Query the pattern over the span, events sorted by active-span start.
(define (pat/query p span)
  (pat//sort pat//event-earlier? (p span)))

;; -- rates, cats, shift -------------------------------------------------------

;; The grid pattern time snaps to when converting from floats: fine enough
;; for musical subdivisions (2^7 * 3 * 5 per cycle), coarse enough to keep
;; denominators bounded.
(define pat//grid 1920)

;; Convert a number to an exact rational, snapping floats to the nearest
;; 1/1920 of a cycle. Exact numbers pass through untouched. Graph number
;; nodes produce floats, so node exprs pass numeric pattern parameters
;; (rates, shifts, weights) through this to keep pattern time exact.
(define (pat/rationalize x)
  (if (exact? x)
      x
      (/ (exact (round (* x pat//grid))) pat//grid)))

;; Speed the pattern up by the factor `r`. A zero rate yields silence.
(define (pat/fast r p)
  (if (zero? r)
      pat/silence
      (lambda (span)
        (pat//map (lambda (e)
                    (pat/event-map-spans
                     (lambda (s) (pat/span-map (lambda (t) (/ t r)) s))
                     e))
                  (p (pat/span-map (lambda (t) (* t r)) span))))))

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
              (p (pat/span-map (lambda (t) (- t amount)) span)))))

;; Concatenate the patterns, one pattern per cycle.
(define (pat/slowcat ps)
  (let ((n (length ps)))
    (if (zero? n)
        pat/silence
        (lambda (span)
          (pat//flat-map
           (lambda (cyc)
             (let ((ix (modulo (floor (car cyc)) n)))
               ((list-ref ps ix) cyc)))
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
                                      ((car (cdr sp)) sect))
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

;; Layer the patterns: a query concatenates every pattern's events.
(define (pat/stack ps)
  (lambda (span)
    (pat//flat-map (lambda (p) (p span)) ps)))

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

;; -- higher-order combinators ---------------------------------------------------

;; Map event values with `f`.
(define (pat/map f p)
  (lambda (span)
    (pat//map (lambda (e) (pat/event-map-value f e)) (p span))))

;; Keep events whose value satisfies `keep?`.
(define (pat/filter keep? p)
  (lambda (span)
    (pat//filter (lambda (e) (keep? (pat/event-value e))) (p span))))

;; Keep events satisfying `keep?`.
(define (pat/filter-events keep? p)
  (lambda (span)
    (pat//filter keep? (p span))))

;; The whole common to both events: the intersection of their wholes when
;; both are present, otherwise #f (including non-intersecting wholes).
(define (pat//whole-intersect ow iw)
  (if ow (if iw (pat/span-intersect ow iw) #f) #f))

;; Join a pattern of patterns: inner patterns queried with the outer
;; event's active span, event spans intersected (whole and active alike).
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
        ((pat/event-value oe) (pat/event-active oe))))
     (pp span))))

;; Like [`pat/join`], but structure comes from the inner pattern alone:
;; wholes untouched, actives clipped to the original query span.
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
        ((pat/event-value oe) (pat/event-active oe))))
     (pp q-span))))

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
          ((pat/event-value oe) (cons start start)))))
     (pp q-span))))

;; Apply a pattern of functions `pf` to a pattern of values `pv`: an event
;; per intersection of active spans (both sides queried with the original
;; query span), whole = `(structure left-whole right-whole)` only when
;; both wholes are present, else #f.
(define (pat//apply pv pf structure)
  (lambda (span)
    (pat//flat-map
     (lambda (ev)
       (pat//flat-map
        (lambda (ef)
          (let ((active (pat/span-intersect (pat/event-active ev)
                                            (pat/event-active ef))))
            (if active
                (let ((lw (pat/event-whole ev))
                      (rw (pat/event-whole ef)))
                  (list (pat/event ((pat/event-value ef) (pat/event-value ev))
                                   active
                                   (if lw (if rw (structure lw rw) #f) #f))))
                '())))
        (pf span)))
     (pv span))))

;; Apply with structure from the intersection of both wholes.
(define (pat/app pv pf)
  (pat//apply pv pf pat/span-intersect))

;; Apply with structure from the left (the value pattern).
(define (pat/appl pv pf)
  (pat//apply pv pf (lambda (l r) l)))

;; Apply with structure from the right (the function pattern).
(define (pat/appr pv pf)
  (pat//apply pv pf (lambda (l r) r)))

;; Merge two patterns by calling `(f a-value b-value)` at every
;; intersection of active spans (intersection structure).
(define (pat/merge-with f pa pb)
  (pat/app pa (pat/map (lambda (bv) (lambda (av) (f av bv))) pb)))

;; -- euclidean rhythms ----------------------------------------------------------

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
;; differently rotated pattern, diverging at e.g. (5, 8).
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

;; Rotate the list left by `off` (modulo its length).
(define (pat//rotate xs off)
  (let ((len (length xs)))
    (if (< len 1)
        xs
        (let ((o (modulo off len)))
          (append (list-tail xs o) (take xs o))))))

;; The bjorklund onset pattern distributing `k` onsets as evenly as
;; possible over `n` slots (`k` clamped to `0..=n`), rotated left by
;; `off` slots. Returns a list of `n` booleans.
(define (pat/euclid-bools k n off)
  (if (< n 1)
      '()
      (let ((kk (pat//min2 (pat//max2 k 0) n)))
        (pat//rotate
         (pat//flat-map (lambda (g) g)
                        (pat//bjorklund-loop (pat//repeat (list #t) kk '())
                                             (pat//repeat (list #f) (- n kk) '())))
         off))))

;; Cyclic distance from each slot to the next onset (inclusive of the
;; current slot), or the empty list when there are no onsets at all.
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

;; `k` onsets distributed over `n` equal slots per cycle, silent slots
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

;; -- windowing and delivery -----------------------------------------------------

;; Whether the event begins at its whole's start, i.e. is a true onset
;; rather than the continuation of an event chopped by a window
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
;; sane configurations while bounding the events a single tick can
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
;; grid, so successive spans abut exactly, quantisation error never
;; accumulates, and denominators stay bounded. The span is empty on the
;; first tick and whenever the position has not advanced. Any position
;; jump beyond `pat//max-window`, in either direction, also yields an
;; empty span and continues from the new position. A cps change
;; rescales the timeline, so its gap is dropped rather than replayed or
;; fast-forwarded.
(define (pat/window st t cps)
  (let ((pos (/ (exact (round (* t cps pat//grid))) pat//grid)))
    (let ((prev (if (number? st) st pos)))
      (list (if (< pos prev)
                (cons pos pos)
                (if (> (- pos prev) pat//max-window)
                    (cons pos pos)
                    (cons prev pos)))
            pos))))

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
  (pat//map
   (lambda (e)
     (list (+ t (exact->inexact (/ (- (car (pat/event-active e)) (car span)) cps)))
           (let ((v (pat/event-value e)))
             (if (number? v) (exact->inexact v) v))))
   (pat//filter pat/event-onset? events)))
