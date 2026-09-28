//! Input and output nodes for gantz.
//!
//! These nodes pass values between a graph and the world outside it. The
//! [`Log`] node writes values to the `log` facade.

pub use log::Log;
pub use sugar::IoSugar;

pub mod log;
pub mod sugar;

/// Builtin specs for the io node set.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    use gantz_core::Builtin;
    vec![Builtin::new("log", &Log::default())]
}
