//! Provides custom nodes that are commonly useful to egui applications of
//! gantz.
//!
//! Provides new node items, while re-exporting some of the `gantz_core::node`
//! items for convenience.

pub use comment::Comment;
pub use dyn_node::{DynNode, NodeCodec, NormalizeNodeError, UiBuiltins, UiNodeInstance};
pub use fn_named_ref::FnNamedRef;
#[doc(inline)]
pub use gantz_core::node::{Id, state};
pub use inspect::Inspect;
pub use named_ref::{NamedRef, missing_color, outdated_color};
pub use plot::{Plot, PlotMode, PlotStyle};
pub use ref_ext::RefExtUi;

pub mod comment;
pub mod dyn_node;
pub mod fn_named_ref;
pub mod inspect;
pub mod named_ref;
pub mod plot;
pub mod ref_ext;
mod size_sync;

/// Builtin specs for the egui node set.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    use gantz_core::Builtin;
    // The `fn` builtin defaults to referring to the `id` builtin, pinned at
    // its ERASED content address (the same scheme all builtins index by).
    let identity_ca = gantz_core::data::erase_node_typed(&gantz_core::node::Identity)
        .expect("`id` must erase")
        .content_addr();
    let name = gantz_core::node::IDENTITY_NAME.parse().expect("infallible");
    let named_ref = NamedRef::new(name, gantz_core::node::Ref::new(identity_ca));
    vec![
        Builtin::new("comment", &Comment::default()),
        Builtin::new("fn", &gantz_core::node::Fn::new(named_ref)),
        Builtin::new("inspect", &Inspect::default()),
        Builtin::new("plot", &Plot::default()),
    ]
}
