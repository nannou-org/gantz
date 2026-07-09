pub use tick_bang::{Interval, TickBang, ToTickBang};
pub use update_bang::{ToUpdateBang, UpdateBang};

pub mod tick_bang;
pub mod update_bang;

/// Builtin specs for the bevy node set.
pub fn builtins<N>() -> Vec<gantz_core::Builtin<N>>
where
    N: gantz_core::FromNode<TickBang> + gantz_core::FromNode<UpdateBang>,
{
    use gantz_core::Builtin;
    vec![
        Builtin::new("tick!", || N::from_node(TickBang::default())),
        Builtin::new("update!", || N::from_node(UpdateBang)),
    ]
}
