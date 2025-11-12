//! The content-addressing implementation for `gantz` graphs.

#[doc(inline)]
pub use ca::{ContentAddr, ContentAddrShort, content_addr};
#[doc(inline)]
pub use hash::{CaHash, Hasher};

mod ca;
mod hash;
