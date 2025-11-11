//! The content-addressing implementation for `gantz` graphs.

#[doc(inline)]
pub use ca::{content_addr, ContentAddr, ContentAddrShort};
#[doc(inline)]
pub use hash::{CaHash, Hasher};

mod ca;
mod hash;
