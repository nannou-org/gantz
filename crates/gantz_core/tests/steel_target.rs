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
