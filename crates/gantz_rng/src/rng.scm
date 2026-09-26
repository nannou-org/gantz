;; The gantz/rng module. Seeded, stateless random numbers.
;;
;; A seed is an integer. Every value these fns produce is a pure function
;; of their arguments. A seed gives the same values on every run, on every
;; platform and in every version.
;;
;; Seeds are 64 bits. An integer-valued number in [-2^63, 2^64) is a seed
;; modulo 2^64, so -1 and 2^64 - 1 are the same seed. Any other value is
;; first folded into seed 0 as a key. Output seeds are signed integers.
;;
;; Every fn uses up its seed. Two uses of one seed give correlated
;; results. Fold a different key into the seed for each use.
;;
;; (rng/fold-in seed key)    A new seed from `seed` and the value `key`.
;;                           Numbers fold in by exact value, so 3, 3.0 and
;;                           6/2 give the same seed.
;; (rng/split seed n)        A list of `n` seeds. Element `i` is
;;                           (rng/fold-in seed i).
;; (rng/uniform seed)        A float in [0, 1).
;; (rng/integer seed lo hi)  An integer in [lo, hi), or `lo` when
;;                           `hi <= lo`.
;; (rng/bernoulli seed p)    #t with the probability `p`.
;; (rng/choose seed xs)      A uniformly chosen element of the list or
;;                           vector `xs`. An empty `xs` is an error.
;; (rng/shuffle seed xs)     The elements of `xs` as a list, in a uniformly
;;                           random order.

(require-builtin #%gantz/rng)

(provide rng/fold-in
         rng/split
         rng/uniform
         rng/integer
         rng/bernoulli
         rng/choose
         rng/shuffle)
