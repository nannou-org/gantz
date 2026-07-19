//! A library of standard plugins for gantz.

pub use bang::Bang;
pub use log::Log;
pub use number::Number;
pub use sugar::StdSugar;

pub mod bang;
pub mod log;
pub mod number;
pub mod sugar;

/// Builtin specs for the std node set.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    use gantz_core::Builtin;
    vec![
        Builtin::new("bang", &Bang::default()),
        Builtin::new("log", &Log::default()),
        Builtin::new("number", &Number::default()),
    ]
}
