;; The gantz/ui module.
;;
;; Helpers for building element trees from optional node inputs. A base
;; node's expr receives each unconnected `$?` input as `(None)`. These fns
;; drop absent values so an element carries only the attributes and
;; positionals that were wired in. See the `uirow`, `uidialer` and
;; `uilabel` graphs in base.gantz.
;;
;; Written for the prelude-free base engine. It uses primitive special
;; forms only. Names prefixed `ui//` are internal helpers and are not
;; provided.

(require-builtin steel/core/option)

(provide ui-elem ui-int ui-id)

;; The values inside the `Some`s of `opts`, in order. `None`s are dropped.
(define (ui//somes opts)
  (transduce opts (filtering Some?) (mapping Some->value) (into-list)))

;; The `(name value)` pairs of `attrs` whose value is `Some`, unwrapped.
(define (ui//attr-pairs attrs)
  (transduce attrs
             (filtering (lambda (p) (Some? (car (cdr p)))))
             (mapping (lambda (p) (list (car p) (Some->value (car (cdr p))))))
             (into-list)))

;; The child list for an optional `children` value. `None` is no children.
;; A list of elements is used as is. A single element, whose first item is
;; its tag rather than a list, is wrapped so it can be wired in directly.
(define (ui//children opt)
  (if (Some? opt)
      (let ((c (Some->value opt)))
        (if (list? c)
            (if (empty? c) c (if (list? (car c)) c (list c)))
            (list c)))
      '()))

;; An element `(tag arg ... (@ (name value) ...) child ...)`.
;;
;; `tag` is a symbol. `args` is a list of optional positional values.
;; `attrs` is a list of `(name optional-value)` pairs. `children` is an
;; optional list of elements. Absent values are dropped. The attrs block
;; is omitted when no attribute is present.
(define (ui-elem tag args attrs children)
  (let ((pairs (ui//attr-pairs attrs)))
    (append (list tag)
            (ui//somes args)
            (if (empty? pairs) '() (list (cons '@ pairs)))
            (ui//children children))))

;; An optional number as an optional exact integer, for integer attributes
;; such as `cols` and `precision`.
(define (ui-int opt)
  (if (Some? opt) (Some (exact (round (Some->value opt)))) opt))

;; The first segment of an optional node path, for `ref-gui`.
(define (ui-id opt)
  (if (Some? opt) (Some (car (Some->value opt))) opt))
