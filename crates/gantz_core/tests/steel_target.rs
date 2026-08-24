//! Pins the Steel semantics the IR emitter relies on (see the compiler
//! redesign plan). The emitter lowers branch reconvergence to local
//! "join point" fns and feedback loops to self-recursive local fns, so it
//! depends on:
//!
//! 1. Local `define`s in a fn body behaving like `letrec*` (forward refs from
//!    later-called bodies, self-recursion).
//! 2. Tail-call optimization for self- and mutual recursion between local
//!    defines (loop iterations must not grow the stack).
//! 3. `define-values` within a body.
//! 4. `define`s interleaved with expression statements within a body.
//!
//! All on `Engine::new_base()` - the prelude-free engine the VM runs - using
//! only primitive forms (`if`, `begin`, `define`, `define-values`, `let`,
//! `lambda`, `set!`).

use steel::SteelVal;
use steel::steel_vm::engine::Engine;

/// Iterations deep enough that a non-TCO implementation would exhaust memory
/// or stack rather than complete.
const DEEP: usize = 200_000;

fn run_int(src: &str) -> isize {
    let mut vm = Engine::new_base();
    let vals = vm.run(src.to_string()).unwrap();
    match vals.last() {
        Some(SteelVal::IntV(i)) => *i,
        other => panic!("expected IntV, got {other:?}"),
    }
}

/// A self-recursive local define in tail position runs in constant stack.
/// This is the shape of a lowered iterate-until-branch loop (`rec` join).
#[test]
fn tco_self_recursive_local_define() {
    let src = format!(
        "(define (top)
           (define (loopfn acc n)
             (if (= n 0) acc (loopfn (+ acc 1) (- n 1))))
           (loopfn 0 {DEEP}))
         (top)"
    );
    assert_eq!(run_int(&src), DEEP as isize);
}

/// Mutually tail-recursive sibling defines run in constant stack. This is the
/// shape of a loop body whose inner join tail-calls back to the loop join.
#[test]
fn tco_mutual_tail_calls_between_local_defines() {
    let src = format!(
        "(define (top)
           (define (a acc n) (if (= n 0) acc (b (+ acc 1) (- n 1))))
           (define (b acc n) (if (= n 0) acc (a (+ acc 1) (- n 1))))
           (a 0 {DEEP}))
         (top)"
    );
    assert_eq!(run_int(&src), DEEP as isize);
}

/// A local define may call a sibling defined *after* it (letrec* semantics):
/// the reference resolves at call time. Gives the emitter freedom in join
/// emission order.
#[test]
fn forward_reference_between_sibling_defines() {
    let src = "(define (top)
                 (define (a x) (b (+ x 1)))
                 (define (b x) (* x 10))
                 (a 1))
               (top)";
    assert_eq!(run_int(src), 20);
}

/// `define-values` destructures a list within a body (multi-output node
/// results bind this way).
#[test]
fn define_values_in_body() {
    let src = "(define (top)
                 (define-values (x y) (list 3 4))
                 (+ x y))
               (top)";
    assert_eq!(run_int(src), 7);
}

/// `define`s may be interleaved with expression statements within a fn body
/// (a lowered body mixes node-call defines with branch `if` expressions).
#[test]
fn define_after_expression_in_body() {
    let src = "(define (top)
                 (define x 1)
                 (if (= x 1) '() '())
                 (define y (+ x 1))
                 (+ x y))
               (top)";
    assert_eq!(run_int(src), 3);
}

// ----------------------------------------------------------------------------
// Pins for the registered-steel-module semantics `vm::new_engine` relies on:
// registration is lazy (no compilation until the first `require`), compiled
// modules are cached for the engine's lifetime, and provided names bind
// unmangled in the requiring program.

fn run_int_vm(vm: &mut Engine, src: &str) -> isize {
    let vals = vm.run(src.to_string()).unwrap();
    match vals.last() {
        Some(SteelVal::IntV(i)) => *i,
        other => panic!("expected IntV, got {other:?}"),
    }
}

/// A module registered via `new_engine` resolves through `require`, and its
/// provided names bind unmangled in the requiring program.
#[test]
fn required_module_provides_bind_unmangled() {
    let mut vm = gantz_core::vm::new_engine(&[]);
    let src = "(require \"gantz/option\")
               (+ (unwrap-or (Some 5) 0)
                  (unwrap-or (None) 2)
                  (if (option? (Some 1)) 10 0)
                  (unwrap-or (map-option (lambda (x) (* x 10)) (Some 4)) 0))";
    assert_eq!(run_int_vm(&mut vm, src), 57);
}

/// Registration performs no compilation: a syntactically invalid module is
/// accepted silently, and the error surfaces only at the first `require`.
#[test]
fn module_registration_is_lazy() {
    let broken = gantz_core::vm::SteelModule {
        name: "gantz/test-broken",
        src: "(define (broken",
    };
    let mut vm = gantz_core::vm::new_engine(&[broken]);
    assert_eq!(run_int_vm(&mut vm, "(+ 1 2)"), 3);
    assert!(
        vm.run("(require \"gantz/test-broken\")".to_string())
            .is_err()
    );
}

/// A second program containing the same `require` hits the engine's module
/// cache: the module's top-level defines run once per engine, not per run.
#[test]
fn required_module_is_cached_across_runs() {
    let counting = gantz_core::vm::SteelModule {
        name: "gantz/test-counting",
        src: "(provide get-count)
              (define count (box 0))
              (set-box! count (+ (unbox count) 1))
              (define (get-count) (unbox count))",
    };
    let mut vm = gantz_core::vm::new_engine(&[counting]);
    let src = "(require \"gantz/test-counting\") (get-count)";
    assert_eq!(run_int_vm(&mut vm, src), 1);
    assert_eq!(run_int_vm(&mut vm, src), 1);
}

/// Requiring a name that was never registered errors rather than silently
/// binding nothing.
#[test]
fn require_of_unregistered_module_errors() {
    let mut vm = gantz_core::vm::new_engine(&[]);
    assert!(vm.run("(require \"gantz/nope\")".to_string()).is_err());
}
