//! The `%args` map holds per-evaluation inputs that the entrypoint caller
//! provides to the VM. Any node's [`expr`](crate::Node::expr) can read it via
//! the [`ARGS`](crate::ARGS) global.
//!
//! It mirrors `%root-state`. See [`ROOT_STATE`](crate::ROOT_STATE). The caller
//! sets the global before it invokes an entry fn. No value is threaded through
//! the function signatures. The entry fn stays nullary, so callers that do not
//! set `%args` see the [`default`].
//!
//! The one key so far is [`TIME`]. It is the monotonic firing time of the
//! evaluation in seconds. Timing-sensitive nodes read it to stamp their output.
//! A node reads it from Steel as `(hash-ref %args 'time)`.

use steel::{SteelVal, gc::Gc};

/// The `%args` key holding the entrypoint firing time, in monotonic seconds.
pub const TIME: &str = "time";

/// An `%args` map carrying the firing `time` in monotonic seconds.
pub fn time(secs: f64) -> SteelVal {
    let map = steel::HashMap::new().update(SteelVal::SymbolV(TIME.into()), SteelVal::NumV(secs));
    SteelVal::HashMapV(Gc::new(map).into())
}

/// The default `%args` registered at VM init. `time` is `0.0`, so a node that
/// reads `(hash-ref %args 'time)` is valid even when no caller has set `%args`.
pub fn default() -> SteelVal {
    time(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use steel::steel_vm::engine::Engine;

    #[test]
    fn args_time_roundtrips_through_the_vm() {
        // Mirror the runtime. Register the default, then update it as a caller
        // would before an entry fn reads `(hash-ref %args 'time)`.
        let mut vm = Engine::new_base();
        vm.register_value(crate::ARGS, default());
        let v = vm.run("(hash-ref %args 'time)").unwrap();
        assert_eq!(v.last(), Some(&SteelVal::NumV(0.0)));

        vm.update_value(crate::ARGS, time(1.5));
        let v = vm.run("(hash-ref %args 'time)").unwrap();
        assert_eq!(v.last(), Some(&SteelVal::NumV(1.5)));
    }
}
