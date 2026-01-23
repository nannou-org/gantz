//! Provides custom nodes that are commonly useful to egui applications of
//! gantz.
//!
//! Provides new node items, while re-exporting some of the `gantz_core::node`
//! items for convenience.

pub use comment::Comment;
pub use fn_named_ref::{FnNamedRef, FnNodeNames};
#[doc(inline)]
pub use gantz_core::node::{Id, state};
pub use named_ref::{NameRegistry, NamedRef, outdated_color};

pub mod comment;
pub mod fn_named_ref;
pub mod named_ref;
