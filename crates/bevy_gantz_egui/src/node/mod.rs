pub use tick_bang::{Interval, TickBang};
pub use update_bang::UpdateBang;

pub mod tick_bang;
pub mod update_bang;

/// Builtin specs for the bevy node set.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    use gantz_core::Builtin;
    vec![
        Builtin::new("tick!", &TickBang::default()),
        Builtin::new("update!", &UpdateBang),
    ]
}
