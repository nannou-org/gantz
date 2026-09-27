;; The gantz/list module.
;;
;; Higher-order, generator and slicing fns over plain lists. Steel's own
;; `map`, `filter`, `foldl` and `drop` live in its prelude, which the
;; base engine does not load. The native `sort` rejects closures.
;;
;; Every fn is total. A partial graph eval can hand any argument an
;; unfired input's `'()` or a void-flavored binding, so:
;;
;; - A non-list in place of a list counts as the empty list.
;; - A non-fn in place of a fn gives the empty result. That is `'()` for
;;   a list result, `init` for `list/fold` and `#f` for `list/any`,
;;   `list/all` and `list/find`.
;; - A count or bound is rounded to an exact integer. A non-number counts
;;   as 0.
;;
;; `list/map`, `list/filter`, `list/fold` and `list/zip` use Steel's
;; native transducers. These iterate in Rust and enter the VM only to
;; call `f`, which makes them several times faster than a Scheme loop.
;;
;; Written for the base engine: primitive special forms only (no `and`,
;; `or`, `cond`). Names prefixed `list//` are internal helpers and are
;; not provided.

(provide list/map
         list/filter
         list/fold
         list/flat-map
         list/concat
         list/zip
         list/any
         list/all
         list/find
         list/sort
         list/range
         list/take
         list/drop
         list/last)

;; `xs` when it is a list, otherwise the empty list.
(define (list//of xs) (if (list? xs) xs '()))

;; `n` rounded to an exact integer. A non-number, NaN or infinity is 0.
(define (list//int n)
  (if (real? n) (if (finite? n) (exact (round n)) 0) 0))

;; `n` as an exact integer clamped to `[0, (length xs)]`.
(define (list//count xs n)
  (let ((i (list//int n))
        (len (length xs)))
    (if (< i 0) 0 (if (< len i) len i))))

(define (list//rev-append xs acc)
  (if (empty? xs) acc (list//rev-append (cdr xs) (cons (car xs) acc))))

;; Push `x` onto the reversed `acc`. A list pushes each of its items and
;; any other value pushes itself.
(define (list//splice x acc)
  (if (list? x) (list//rev-append x acc) (cons x acc)))

;; Apply `f` to each item of `xs`.
(define (list/map f xs)
  (if (function? f) (transduce (list//of xs) (mapping f) (into-list)) '()))

;; The items of `xs` for which `keep?` is truthy, in order.
(define (list/filter keep? xs)
  (if (function? keep?)
      (transduce (list//of xs) (filtering keep?) (into-list))
      '()))

;; Left fold. Calls `(f acc x)` for each item `x`, first to last, and
;; returns the final `acc`.
(define (list/fold f init xs)
  (if (function? f) (transduce (list//of xs) (into-reducer f init)) init))

;; Apply `f` to each item of `xs` and join the results in order. A result
;; that is not a list counts as one item.
(define (list/flat-map f xs)
  (if (function? f) (list//flat-map-loop f (list//of xs) '()) '()))

(define (list//flat-map-loop f xs acc)
  (if (empty? xs)
      (reverse acc)
      (list//flat-map-loop f (cdr xs) (list//splice (f (car xs)) acc))))

;; Join the lists in `xss` in order. An item that is not a list counts as
;; one item.
(define (list/concat xss)
  (list/flat-map (lambda (xs) xs) xss))

;; Pair the items of `xs` and `ys` by position as 2-lists. Stops at the
;; end of the shorter list.
(define (list/zip xs ys)
  (transduce (list//of xs) (zipping (list//of ys)) (into-list)))

;; Whether `pred` is truthy for any item of `xs`.
(define (list/any pred xs)
  (if (function? pred) (list//any-loop pred (list//of xs)) #f))

(define (list//any-loop pred xs)
  (if (empty? xs) #f (if (pred (car xs)) #t (list//any-loop pred (cdr xs)))))

;; Whether `pred` is truthy for every item of `xs`. True for the empty
;; list.
(define (list/all pred xs)
  (if (function? pred) (list//all-loop pred (list//of xs)) #f))

(define (list//all-loop pred xs)
  (if (empty? xs) #t (if (pred (car xs)) (list//all-loop pred (cdr xs)) #f)))

;; The first item of `xs` for which `pred` is truthy, or `#f`.
(define (list/find pred xs)
  (if (function? pred) (list//find-loop pred (list//of xs)) #f))

(define (list//find-loop pred xs)
  (if (empty? xs)
      #f
      (if (pred (car xs)) (car xs) (list//find-loop pred (cdr xs)))))

;; `xs` in the order of `less?`, a strict order. A stable merge sort.
(define (list/sort less? xs)
  (if (function? less?) (list//sort less? (list//of xs)) '()))

(define (list//sort less? xs)
  (let ((n (length xs)))
    (if (< n 2)
        xs
        (let ((mid (quotient n 2)))
          (list//merge less?
                       (list//sort less? (take xs mid))
                       (list//sort less? (list-tail xs mid))
                       '())))))

;; Merge the sorted `a` and `b` behind the reversed `acc`. A tie takes
;; from `a` first, which keeps the sort stable.
(define (list//merge less? a b acc)
  (if (empty? a)
      (list//rev-append acc b)
      (if (empty? b)
          (list//rev-append acc a)
          (if (less? (car b) (car a))
              (list//merge less? a (cdr b) (cons (car b) acc))
              (list//merge less? (cdr a) b (cons (car a) acc))))))

;; The integers from `start` up to but not including `end`.
(define (list/range start end)
  (list//range-loop (list//int start) (- (list//int end) 1) '()))

(define (list//range-loop start i acc)
  (if (< i start) acc (list//range-loop start (- i 1) (cons i acc))))

;; The first `n` items of `xs`. `n` is clamped to the length of `xs`.
(define (list/take xs n)
  (let ((xs (list//of xs)))
    (take xs (list//count xs n))))

;; All but the first `n` items of `xs`. `n` is clamped to the length of
;; `xs`.
(define (list/drop xs n)
  (let ((xs (list//of xs)))
    (list-tail xs (list//count xs n))))

;; The last item of `xs`, or `'()` when `xs` is empty.
(define (list/last xs)
  (let ((xs (list//of xs)))
    (if (empty? xs) '() (last xs))))
