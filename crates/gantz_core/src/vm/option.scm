;; Ergonomic helpers over steel's builtin Option type.
;;
;; Steel's own `option.scm` library is unavailable on the prelude-free
;; base engine (it depends on the contract modules), so gantz provides
;; the small subset that makes `$?` optional inputs pleasant to use.
;;
;; Written for the base engine: primitive special forms only (no `and`,
;; `or`, `cond`).
(require-builtin steel/core/option)

(provide option? unwrap-or map-option)

;; Whether the value is an Option (`Some` or `None`).
(define (option? v)
  (if (Some? v) #t (None? v)))

;; The value inside `Some`, or `default` when `None`.
(define (unwrap-or opt default)
  (if (Some? opt) (Some->value opt) default))

;; Apply `f` to the value inside `Some`, passing `None` through.
(define (map-option f opt)
  (if (Some? opt) (Some (f (Some->value opt))) opt))
