pub use await_::Await;
pub use gui_refresh::GuiMarkersDirty;
pub use sleep::Sleep;
pub use tick_bang::{Interval, TickBang};
pub use update_bang::UpdateBang;

pub mod await_;
pub mod gui_refresh;
pub mod sleep;
pub mod tick_bang;
pub mod update_bang;

/// Builtin specs for the bevy node set.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    use gantz_core::Builtin;
    vec![
        Builtin::new("await", &Await),
        Builtin::new("sleep", &Sleep::default()),
        Builtin::new("tick!", &TickBang::default()),
        Builtin::new("update!", &UpdateBang),
    ]
}
