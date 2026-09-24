//! Custom nodes for egui applications of gantz.
//!
//! Also re-exports some `gantz_core::node` items for convenience.

pub use bind::Bind;
pub use comment::Comment;
pub use dyn_node::{DynNode, NodeCodec, NodeUiInstance, NormalizeNodeError, UiBuiltins};
pub use fn_named_ref::FnNamedRef;
#[doc(inline)]
pub use gantz_core::node::{Id, state};
pub use gui::{GUI_REF_EXT_KEY, Gui, GuiDisplay, GuiRefExt, GuiRole};
pub use inspect::Inspect;
pub use instance_cache::{InstanceEntry, NodeInstances};
pub use named_ref::{NamedRef, missing_color, outdated_color};
pub use plot::{F32, Plot, PlotLook, PlotMode, PlotStyle};
pub use ref_ext::RefExtUi;

pub mod bind;
pub mod comment;
pub mod dyn_node;
pub mod fn_named_ref;
pub mod gui;
pub mod inspect;
pub mod instance_cache;
pub mod named_ref;
pub mod plot;
pub mod ref_ext;
mod size_sync;

/// Builtin specs for the egui node set.
pub fn builtins() -> Vec<gantz_core::Builtin> {
    use gantz_core::Builtin;
    // The `fn` builtin defaults to a reference to the `id` builtin. It is
    // pinned at the erased content address, the same scheme all builtins
    // index by.
    let identity_ca = gantz_core::data::erase_node_typed(&gantz_core::node::Identity)
        .expect("`id` must erase")
        .content_addr();
    let name = gantz_core::node::IDENTITY_NAME.parse().expect("infallible");
    let named_ref = NamedRef::new(name, gantz_core::node::Ref::new(identity_ca));
    vec![
        Builtin::new("bind", &Bind::default()),
        Builtin::new("comment", &Comment::default()),
        Builtin::new("fn", &gantz_core::node::Fn::new(named_ref)),
        Builtin::new("gui", &Gui::default()),
        Builtin::new("inspect", &Inspect::default()),
        Builtin::new("plot", &Plot::default()),
    ]
}
