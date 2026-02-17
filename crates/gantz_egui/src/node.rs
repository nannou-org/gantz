//! Provides custom nodes that are commonly useful to egui applications of
//! gantz.
//!
//! Provides new node items, while re-exporting some of the `gantz_core::node`
//! items for convenience.

pub use comment::Comment;
pub use fn_named_ref::{FnNamedRef, FnNodeNames};
#[doc(inline)]
pub use gantz_core::node::{Id, state};
pub use inspect::Inspect;
pub use named_ref::{NameRegistry, NamedRef, missing_color, outdated_color};

pub mod comment;
pub mod fn_named_ref;
pub mod inspect;
pub mod named_ref;
