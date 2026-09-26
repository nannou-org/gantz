//! Shared test harness. An engine with the `gantz/pattern` module
//! plus the pin-projection helpers.
//!
//! Steel 0.8.2's `equal?` cannot compare rationals nested in containers.
//! See the canary test in `span.rs`. So expected values are compared via
//! pinned projections. Every exact number becomes a numerator and
//! denominator list, spans become 2-lists and events become value,
//! active and whole lists. Pinned forms contain only ints, floats, bools
//! and symbols, which `equal?` handles, and they also pin exactness. An
//! accidental float shows up as `0.5` instead of `(1 2)`.

use gantz_core::steel::SteelVal;
use gantz_core::steel::steel_vm::engine::Engine;

/// Steel source prepended to every test snippet. The module require plus
/// the pin helpers. Kept here rather than in the module so runtime code
/// never depends on the test-only projection format.
pub const PIN: &str = r#"
(require "gantz/pattern")
(define (pin-map f xs)
  (if (empty? xs) '() (cons (f (car xs)) (pin-map f (cdr xs)))))
(define (pin-num x)
  (if (exact? x) (list (numerator x) (denominator x)) x))
(define (pin-span s)
  (if s (list (pin-num (car s)) (pin-num (cdr s))) #f))
(define (pin-spans ss) (pin-map pin-span ss))
(define (pin-event e)
  (list (pin-value (pat/event-value e))
        (pin-span (pat/event-active e))
        (pin-span (pat/event-whole e))))
(define (pin-events es) (pin-map pin-event es))
(define (pin-value v) (if (number? v) (pin-num v) v))
"#;

/// An engine with the pattern module registered and the pin preamble
/// evaluated. Building one compiles the whole pattern module, so each test
/// builds one and passes it to every assertion.
pub fn new_pin_engine() -> Engine {
    let mut vm = gantz_core::vm::new_engine(gantz_pattern::modules());
    vm.run(PIN.to_string()).expect("pin preamble");
    vm
}

/// Evaluate a snippet on an engine prepared by [`new_pin_engine`],
/// returning the final value.
pub fn eval(vm: &mut Engine, snippet: &str) -> SteelVal {
    let vals = vm
        .run(snippet.to_string())
        .unwrap_or_else(|e| panic!("steel error: {e}\nin snippet:\n{snippet}"));
    vals.last()
        .unwrap_or_else(|| panic!("snippet evaluated to no value:\n{snippet}"))
        .clone()
}

/// Assert the snippet evaluates to `#t`.
pub fn assert_steel_true(vm: &mut Engine, snippet: &str) {
    match eval(vm, snippet) {
        SteelVal::BoolV(true) => (),
        other => panic!("expected #t, got {other:?} for:\n{snippet}"),
    }
}

/// Assert `expr` evaluates to the quoted `expected` pinned literal,
/// re-evaluating `expr` for a readable actual value on failure.
pub fn assert_pinned(vm: &mut Engine, expected: &str, expr: &str) {
    let check = format!("(equal? '{expected} {expr})");
    match eval(vm, &check) {
        SteelVal::BoolV(true) => (),
        _ => {
            let actual = eval(vm, expr);
            panic!("expected {expected}\n     got {actual:?}\n     for {expr}");
        }
    }
}
