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
pub fn builtins<N>() -> Vec<gantz_core::Builtin<N>>
where
    N: gantz_core::FromNode<Bang> + gantz_core::FromNode<Log> + gantz_core::FromNode<Number>,
{
    use gantz_core::Builtin;
    vec![
        Builtin::new("bang", || N::from_node(Bang::default())),
        Builtin::new("log", || N::from_node(Log::default())),
        Builtin::new("number", || N::from_node(Number::default())),
    ]
}
