//! Helpers for writing gantz [`Node`] expressions as natural Rust functions.
//!
//! Instead of manually writing Steel (Scheme) strings, users can write
//! standard Rust functions with named parameters and register them with the
//! Steel VM. Type inference resolves the arity automatically.
//!
//! # Stateless example
//!
//! ```ignore
//! fn my_add(a: isize, b: isize) -> isize {
//!     a + b
//! }
//!
//! // In Node::register:
//! node::rust::register(ctx.vm(), "my-add", my_add);
//!
//! // In Node::expr:
//! node::rust::expr("my-add", &[a, b])
//! ```
//!
//! # Stateful example
//!
//! State and return types can be any type implementing `FromSteelVal` /
//! `IntoSteelVal` (including `SteelVal` itself for untyped usage):
//!
//! ```ignore
//! // Typed state - no manual SteelVal matching needed.
//! fn my_counter(_trigger: isize, state: &mut isize) -> isize {
//!     *state += 1;
//!     *state
//! }
//!
//! // In Node::register:
//! node::rust::register_stateful(ctx.vm(), "my-counter", my_counter);
//!
//! // In Node::expr:
//! node::rust::expr_stateful("my-counter", &[trigger])
//! ```

use super::ExprResult;
use steel::{
    SteelErr, SteelVal,
    rvals::{FromSteelVal, IntoSteelVal},
    steel_vm::{engine::Engine, register_fn::RegisterFn},
};

// ---------------------------------------------------------------------------
// Stateless registration
// ---------------------------------------------------------------------------

/// Register a stateless Rust function with the Steel VM.
///
/// Forwards directly to [`Engine::register_fn`]. Type inference resolves
/// the arity from the function signature.
pub fn register<FN, ARGS, RET>(vm: &mut Engine, name: &'static str, f: FN)
where
    Engine: steel::steel_vm::register_fn::RegisterFn<FN, ARGS, RET>,
{
    vm.register_fn(name, f);
}

// ---------------------------------------------------------------------------
// Stateful registration
// ---------------------------------------------------------------------------

/// Trait for registering a Rust function that takes `&mut S` state as its last
/// parameter and returns `R`.
///
/// Implementations are generated for arities 0-15 (positional inputs) via the
/// internal [`impl_register_stateful`] macro. The user writes:
///
/// ```ignore
/// fn my_fn(input0: A, input1: B, ..., state: &mut S) -> R
/// ```
///
/// where `S: FromSteelVal + IntoSteelVal` and `R: IntoSteelVal`.
///
/// The trait impl wraps it into a function that:
/// 1. Accepts `state` as an owned `S` argument (Steel handles `FromSteelVal`)
/// 2. Calls the user's function with `&mut state`
/// 3. Converts output and state back via `IntoSteelVal`
/// 4. Returns `vec![output, state]` (converted to a Steel list)
///
/// `SteelVal` trivially implements both `FromSteelVal` and `IntoSteelVal`, so
/// existing `&mut SteelVal` signatures continue to work unchanged.
pub trait RegisterStatefulNodeFn<ARGS, S, R> {
    fn register_node_fn(self, vm: &mut Engine, name: &'static str);
}

/// Register a stateful Rust function with the Steel VM.
///
/// The function's last parameter must be `&mut S` (the node's state) where
/// `S: FromSteelVal + IntoSteelVal`. The return type `R` must implement
/// `IntoSteelVal`.
///
/// # Typed state example
///
/// ```ignore
/// fn counter(_trigger: isize, state: &mut isize) -> isize {
///     *state += 1;
///     *state
/// }
/// node::rust::register_stateful(vm, "counter", counter);
/// ```
pub fn register_stateful<F, ARGS, S, R>(vm: &mut Engine, name: &'static str, f: F)
where
    F: RegisterStatefulNodeFn<ARGS, S, R>,
{
    f.register_node_fn(vm, name);
}

/// Generate `RegisterStatefulNodeFn` impls for arities 0..N (positional args).
macro_rules! impl_register_stateful {
    // Base case: 0 positional inputs, state only.
    (0 =>) => {
        impl<FN, S, R> RegisterStatefulNodeFn<(), S, R> for FN
        where
            FN: Fn(&mut S) -> R + Send + Sync + 'static,
            S: FromSteelVal + IntoSteelVal,
            R: IntoSteelVal,
        {
            fn register_node_fn(self, vm: &mut Engine, name: &'static str) {
                vm.register_fn(name, move |s_raw: S| -> Result<Vec<SteelVal>, SteelErr> {
                    let mut state = s_raw;
                    let output = (self)(&mut state);
                    Ok(vec![output.into_steelval()?, state.into_steelval()?])
                });
            }
        }
    };
    // N positional inputs + state. Takes pairs of (TypeParam, binding_name).
    ($n:tt => $($T:ident $b:ident),+) => {
        impl<FN, $($T,)+ S, R> RegisterStatefulNodeFn<($($T,)+), S, R> for FN
        where
            FN: Fn($($T,)+ &mut S) -> R + Send + Sync + 'static,
            $($T: FromSteelVal,)+
            S: FromSteelVal + IntoSteelVal,
            R: IntoSteelVal,
        {
            #[allow(non_snake_case)]
            fn register_node_fn(self, vm: &mut Engine, name: &'static str) {
                vm.register_fn(name, move |$($b: $T,)+ s_raw: S| -> Result<Vec<SteelVal>, SteelErr> {
                    let mut state = s_raw;
                    let output = (self)($($b,)+ &mut state);
                    Ok(vec![output.into_steelval()?, state.into_steelval()?])
                });
            }
        }
    };
}

impl_register_stateful!(0 =>);
impl_register_stateful!(1 => A a);
impl_register_stateful!(2 => A a, B b);
impl_register_stateful!(3 => A a, B b, C c);
impl_register_stateful!(4 => A a, B b, C c, D d);
impl_register_stateful!(5 => A a, B b, C c, D d, E e);
impl_register_stateful!(6 => A a, B b, C c, D d, E e, F2 f2);
impl_register_stateful!(7 => A a, B b, C c, D d, E e, F2 f2, G g);
impl_register_stateful!(8 => A a, B b, C c, D d, E e, F2 f2, G g, H h);
impl_register_stateful!(9 => A a, B b, C c, D d, E e, F2 f2, G g, H h, I i);
impl_register_stateful!(10 => A a, B b, C c, D d, E e, F2 f2, G g, H h, I i, J j);
impl_register_stateful!(11 => A a, B b, C c, D d, E e, F2 f2, G g, H h, I i, J j, K k);
impl_register_stateful!(12 => A a, B b, C c, D d, E e, F2 f2, G g, H h, I i, J j, K k, L l);
impl_register_stateful!(13 => A a, B b, C c, D d, E e, F2 f2, G g, H h, I i, J j, K k, L l, M m);
impl_register_stateful!(14 => A a, B b, C c, D d, E e, F2 f2, G g, H h, I i, J j, K k, L l, M m, N n);
impl_register_stateful!(15 => A a, B b, C c, D d, E e, F2 f2, G g, H h, I i, J j, K k, L l, M m, N n, O o);

// ---------------------------------------------------------------------------
// Expression generation
// ---------------------------------------------------------------------------

/// Generate a Steel call expression for a stateless Rust function.
///
/// Produces `(fn_name arg0 arg1 ...)`.
pub fn expr(fn_name: &str, args: &[&str]) -> ExprResult {
    let args_str = args.join(" ");
    let src = if args.is_empty() {
        format!("({fn_name})")
    } else {
        format!("({fn_name} {args_str})")
    };
    super::parse_expr(&src)
}

/// Generate a Steel call expression for a stateful Rust function.
///
/// The registered function accepts positional inputs followed by `state` as an
/// owned `SteelVal` and returns a two-element list `(output new-state)`.
///
/// This generates an expression that:
/// 1. Calls the function with the given args and the current `state` binding
/// 2. Destructures the result to extract the output and new state
/// 3. Updates the `state` binding via `set!`
/// 4. Evaluates to the output value
///
/// The compiler's stateful wrapping (see `node_fn.rs`) will then capture the
/// updated `state` binding and return `(list output state)`.
pub fn expr_stateful(fn_name: &str, args: &[&str]) -> ExprResult {
    let call_args = if args.is_empty() {
        "state".to_string()
    } else {
        format!("{} state", args.join(" "))
    };
    let src = format!(
        "(let ((result ({fn_name} {call_args})))\
           (set! state (car (cdr result)))\
           (car result))"
    );
    super::parse_expr(&src)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use steel::steel_vm::engine::Engine;

    #[test]
    fn test_register_stateless() {
        let mut vm = Engine::new_base();
        fn add(a: SteelVal, b: SteelVal) -> SteelVal {
            match (&a, &b) {
                (SteelVal::NumV(x), SteelVal::NumV(y)) => SteelVal::NumV(x + y),
                _ => SteelVal::Void,
            }
        }
        register(&mut vm, "test-add", add);
        let result = vm.run("(test-add 1.0 2.0)").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], SteelVal::NumV(3.0));
    }

    #[test]
    fn test_register_stateful_with_input() {
        let mut vm = Engine::new_base();
        fn counter(_trigger: SteelVal, state: &mut SteelVal) -> SteelVal {
            match state {
                SteelVal::IntV(n) => {
                    *n += 1;
                    state.clone()
                }
                _ => SteelVal::Void,
            }
        }
        register_stateful(&mut vm, "test-counter", counter);
        // The registered fn takes (trigger, state) and returns a list [output, new_state].
        let result = vm.run("(test-counter 'bang 0)").unwrap();
        assert_eq!(result.len(), 1);
        // Should be a list: (1 1)
        let list = Vec::<SteelVal>::from_steelval(&result[0]).unwrap();
        assert_eq!(list[0], SteelVal::IntV(1));
        assert_eq!(list[1], SteelVal::IntV(1));
    }

    #[test]
    fn test_register_stateful_no_inputs() {
        let mut vm = Engine::new_base();
        fn tick(state: &mut SteelVal) -> SteelVal {
            match state {
                SteelVal::IntV(n) => {
                    *n += 1;
                    state.clone()
                }
                _ => SteelVal::Void,
            }
        }
        register_stateful(&mut vm, "test-tick", tick);
        let result = vm.run("(test-tick 0)").unwrap();
        assert_eq!(result.len(), 1);
        let list = Vec::<SteelVal>::from_steelval(&result[0]).unwrap();
        assert_eq!(list[0], SteelVal::IntV(1));
        assert_eq!(list[1], SteelVal::IntV(1));
    }

    #[test]
    fn test_register_stateful_typed_with_input() {
        let mut vm = Engine::new_base();
        fn counter(_trigger: isize, state: &mut isize) -> isize {
            *state += 1;
            *state
        }
        register_stateful(&mut vm, "test-typed-counter", counter);
        let result = vm.run("(test-typed-counter 0 0)").unwrap();
        assert_eq!(result.len(), 1);
        let list = Vec::<SteelVal>::from_steelval(&result[0]).unwrap();
        assert_eq!(list[0], SteelVal::IntV(1));
        assert_eq!(list[1], SteelVal::IntV(1));
    }

    #[test]
    fn test_register_stateful_typed_no_inputs() {
        let mut vm = Engine::new_base();
        fn toggle(state: &mut bool) -> bool {
            *state = !*state;
            *state
        }
        register_stateful(&mut vm, "test-toggle", toggle);
        let result = vm.run("(test-toggle #f)").unwrap();
        assert_eq!(result.len(), 1);
        let list = Vec::<SteelVal>::from_steelval(&result[0]).unwrap();
        assert_eq!(list[0], SteelVal::BoolV(true));
        assert_eq!(list[1], SteelVal::BoolV(true));
    }

    #[test]
    fn test_register_closure() {
        let mut vm = Engine::new_base();
        let offset = 10.0;
        register(&mut vm, "test-offset", move |a: SteelVal| -> SteelVal {
            match a {
                SteelVal::NumV(x) => SteelVal::NumV(x + offset),
                _ => SteelVal::Void,
            }
        });
        let result = vm.run("(test-offset 5.0)").unwrap();
        assert_eq!(result[0], SteelVal::NumV(15.0));
    }

    #[test]
    fn test_expr_generation() {
        let e = expr("my-add", &["a", "b"]).unwrap();
        let s = e.to_pretty(80);
        assert_eq!(s, "(my-add a b)");
    }

    #[test]
    fn test_expr_zero_args() {
        let e = expr("my-fn", &[]).unwrap();
        let s = e.to_pretty(80);
        assert_eq!(s, "(my-fn)");
    }

    #[test]
    fn test_expr_stateful_generation() {
        let e = expr_stateful("my-counter", &["push"]).unwrap();
        let s = e.to_pretty(80);
        // Should contain the function call, set!, and car.
        assert!(s.contains("my-counter"));
        assert!(s.contains("push"));
        assert!(s.contains("state"));
        assert!(s.contains("set!"));
    }

    #[test]
    fn test_expr_stateful_no_inputs() {
        let e = expr_stateful("my-tick", &[]).unwrap();
        let s = e.to_pretty(80);
        assert!(s.contains("(my-tick state)"));
        assert!(s.contains("set!"));
    }
}
