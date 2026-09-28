//! A library of standard plugins for gantz.

pub use log::Log;
pub use sugar::StdSugar;

pub mod log;
pub mod sugar;
#[cfg(test)]
mod tests;

/// Builtin specs for the std node set.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    use gantz_core::Builtin;
    vec![Builtin::new("log", &Log::default())]
}
